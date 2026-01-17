//! PTY session management for interactive terminal access

use crate::error::{TerminalError, TerminalResult};
use crate::rate_limit::RateLimiter;
use crate::resize::set_window_size;

use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    pty::openpty,
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use std::{
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tokio::{
    io::{unix::AsyncFd, Interest},
    process::{Child, Command},
    sync::Mutex,
};

/// Default shell to spawn if not specified
const DEFAULT_SHELL: &str = "/bin/bash";

/// Read buffer size
const READ_BUFFER_SIZE: usize = 4096;

/// PTY session for interactive terminal access
///
/// Manages the lifecycle of a pseudo-terminal session, including:
/// - PTY allocation and shell spawning
/// - Async read/write operations
/// - Terminal resize
/// - Rate limiting for input
/// - Graceful shutdown
pub struct TtySession {
    /// Unique session identifier
    session_id: String,

    /// PTY master read handle (async)
    master_read: AsyncFd<std::fs::File>,

    /// PTY master write handle (kept for future async write support)
    #[allow(dead_code)]
    master_write: tokio::fs::File,

    /// Master fd for resize operations
    master_fd: i32,

    /// Child process
    child: Mutex<Option<Child>>,

    /// Child process ID
    child_pid: u32,

    /// Current terminal size (cols, rows)
    size: Mutex<(u16, u16)>,

    /// Rate limiter for input
    rate_limiter: Mutex<RateLimiter>,

    /// Last activity timestamp
    last_activity: Mutex<Instant>,

    /// Whether the session is closed
    closed: AtomicBool,
}

impl TtySession {
    /// Spawn a new PTY session with a shell
    ///
    /// # Arguments
    ///
    /// * `shell` - Path to shell executable (e.g., "/bin/bash")
    /// * `cols` - Initial terminal width
    /// * `rows` - Initial terminal height
    ///
    /// # Example
    ///
    /// ```ignore
    /// let session = TtySession::spawn("/bin/bash", 80, 24).await?;
    /// ```
    pub async fn spawn(shell: &str, cols: u16, rows: u16) -> TerminalResult<Self> {
        let session_id = uuid::Uuid::new_v4().to_string();

        tracing::info!(
            session_id = %session_id,
            shell = %shell,
            cols = cols,
            rows = rows,
            "Spawning PTY session"
        );

        // Allocate PTY
        let pty = openpty(None, None)?;

        // Set master to non-blocking mode
        {
            let flags = OFlag::from_bits_truncate(fcntl(&pty.master, FcntlArg::F_GETFL)?);
            let new_flags = flags | OFlag::O_NONBLOCK;
            fcntl(&pty.master, FcntlArg::F_SETFL(new_flags))?;
        }

        // Set initial window size on slave
        set_window_size(&pty.slave, cols, rows)?;

        // Clone slave for stdin, stdout, stderr
        let slave_in = pty.slave.try_clone()?;
        let slave_out = pty.slave.try_clone()?;
        let slave_err = pty.slave;

        // Build command
        let mut command = Command::new(shell);
        command
            .stdin(Stdio::from(slave_in))
            .stdout(Stdio::from(slave_out))
            .stderr(Stdio::from(slave_err))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor");

        // Set up session and controlling terminal
        unsafe {
            command.pre_exec(|| {
                // Create new session
                libc::setsid();

                // Set controlling terminal
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 1 as libc::c_long) < 0 {
                    return Err(std::io::Error::last_os_error());
                }

                Ok(())
            });
        }

        // Spawn child process
        let child = command.spawn()?;
        let child_pid = child.id().ok_or(TerminalError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to get child PID",
        )))?;

        tracing::info!(
            session_id = %session_id,
            child_pid = child_pid,
            "Shell process spawned"
        );

        // Set up master file handles for async I/O
        let master_fd = pty.master.as_raw_fd();
        let master_fd_owned: OwnedFd = pty.master;
        let master_write_fd = nix::unistd::dup(&master_fd_owned)?;

        let master_read_file = unsafe { std::fs::File::from_raw_fd(master_fd_owned.into_raw_fd()) };
        let master_write_file = unsafe { std::fs::File::from_raw_fd(master_write_fd.into_raw_fd()) };

        let master_read = AsyncFd::new(master_read_file)?;
        let master_write = tokio::fs::File::from_std(master_write_file);

        Ok(Self {
            session_id,
            master_read,
            master_write,
            master_fd,
            child: Mutex::new(Some(child)),
            child_pid,
            size: Mutex::new((cols, rows)),
            rate_limiter: Mutex::new(RateLimiter::default_terminal()),
            last_activity: Mutex::new(Instant::now()),
            closed: AtomicBool::new(false),
        })
    }

    /// Spawn with default shell (/bin/bash)
    pub async fn spawn_default(cols: u16, rows: u16) -> TerminalResult<Self> {
        Self::spawn(DEFAULT_SHELL, cols, rows).await
    }

    /// Get the session ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the child process ID
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Check if the session is closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// Get the current terminal size
    pub async fn size(&self) -> (u16, u16) {
        *self.size.lock().await
    }

    /// Get time since last activity
    pub async fn idle_duration(&self) -> Duration {
        self.last_activity.lock().await.elapsed()
    }

    /// Update last activity timestamp
    async fn touch(&self) {
        *self.last_activity.lock().await = Instant::now();
    }

    /// Write data to the PTY (input from user)
    ///
    /// This method is rate-limited to prevent abuse.
    pub async fn write(&self, data: &[u8]) -> TerminalResult<()> {
        if self.is_closed() {
            return Err(TerminalError::AlreadyClosed);
        }

        // Check rate limit
        {
            let mut limiter = self.rate_limiter.lock().await;
            if !limiter.check() {
                return Err(TerminalError::RateLimited(limiter.rate()));
            }
        }

        self.touch().await;

        // Write to PTY master
        // Note: We need interior mutability for write, using a separate approach
        let fd = self.master_fd;
        let data = data.to_vec();

        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
            let result = file.write_all(&data);
            // Don't close the fd - forget the file handle
            std::mem::forget(file);
            result
        })
        .await
        .map_err(|e| TerminalError::WriteFailed(e.to_string()))?
        .map_err(|e| TerminalError::WriteFailed(e.to_string()))?;

        Ok(())
    }

    /// Read data from the PTY (output from shell)
    ///
    /// Returns available data or empty vec if no data available.
    /// This is non-blocking - use in a loop with appropriate delays.
    pub async fn read(&self) -> TerminalResult<Vec<u8>> {
        if self.is_closed() {
            return Err(TerminalError::AlreadyClosed);
        }

        self.touch().await;

        let mut buffer = vec![0u8; READ_BUFFER_SIZE];

        // Try to read with async interest
        match self.master_read.ready(Interest::READABLE).await {
            Ok(mut guard) => {
                match guard.try_io(|inner| {
                    use std::io::Read;
                    let mut file = inner.get_ref();
                    file.read(&mut buffer)
                }) {
                    Ok(Ok(0)) => {
                        // EOF - shell exited
                        Ok(vec![])
                    }
                    Ok(Ok(n)) => {
                        buffer.truncate(n);
                        Ok(buffer)
                    }
                    Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No data available
                        Ok(vec![])
                    }
                    Ok(Err(e)) => Err(TerminalError::ReadFailed(e.to_string())),
                    Err(_) => {
                        // Would block, try again later
                        Ok(vec![])
                    }
                }
            }
            Err(e) => Err(TerminalError::ReadFailed(e.to_string())),
        }
    }

    /// Resize the terminal
    pub async fn resize(&self, cols: u16, rows: u16) -> TerminalResult<()> {
        if self.is_closed() {
            return Err(TerminalError::AlreadyClosed);
        }

        self.touch().await;

        // Update stored size
        {
            let mut size = self.size.lock().await;
            *size = (cols, rows);
        }

        // Apply resize via ioctl
        // We need to use the master fd
        let fd = self.master_fd;
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let result = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &winsize) };

        if result < 0 {
            return Err(TerminalError::PtyAllocation(nix::Error::last()));
        }

        // Send SIGWINCH to child process to notify of resize
        if let Err(e) = kill(Pid::from_raw(self.child_pid as i32), Signal::SIGWINCH) {
            tracing::warn!(
                session_id = %self.session_id,
                error = %e,
                "Failed to send SIGWINCH to child process"
            );
        }

        tracing::debug!(
            session_id = %self.session_id,
            cols = cols,
            rows = rows,
            "Terminal resized"
        );

        Ok(())
    }

    /// Close the session and terminate the child process
    ///
    /// Returns the exit code of the child process if available.
    pub async fn close(&self) -> TerminalResult<Option<i32>> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Err(TerminalError::AlreadyClosed);
        }

        tracing::info!(
            session_id = %self.session_id,
            "Closing PTY session"
        );

        let mut child_guard = self.child.lock().await;

        if let Some(mut child) = child_guard.take() {
            // Try graceful shutdown first (SIGTERM)
            if let Err(e) = kill(Pid::from_raw(self.child_pid as i32), Signal::SIGTERM) {
                tracing::warn!(
                    session_id = %self.session_id,
                    error = %e,
                    "Failed to send SIGTERM to child process"
                );
            }

            // Wait for child with timeout
            let wait_result = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

            match wait_result {
                Ok(Ok(status)) => {
                    let exit_code = status.code();
                    tracing::info!(
                        session_id = %self.session_id,
                        exit_code = ?exit_code,
                        "Child process exited"
                    );
                    return Ok(exit_code);
                }
                Ok(Err(e)) => {
                    tracing::error!(
                        session_id = %self.session_id,
                        error = %e,
                        "Error waiting for child process"
                    );
                }
                Err(_) => {
                    // Timeout - send SIGKILL
                    tracing::warn!(
                        session_id = %self.session_id,
                        "Child process did not exit, sending SIGKILL"
                    );

                    if let Err(e) = kill(Pid::from_raw(self.child_pid as i32), Signal::SIGKILL) {
                        tracing::error!(
                            session_id = %self.session_id,
                            error = %e,
                            "Failed to send SIGKILL to child process"
                        );
                    }

                    // Wait again briefly
                    if let Ok(Ok(status)) =
                        tokio::time::timeout(Duration::from_secs(1), child.wait()).await
                    {
                        return Ok(status.code());
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check if the child process is still running
    pub async fn is_running(&self) -> bool {
        if self.is_closed() {
            return false;
        }

        let mut child_guard = self.child.lock().await;
        if let Some(child) = child_guard.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => false, // Process exited
                Ok(None) => true,     // Still running
                Err(_) => false,      // Error checking
            }
        } else {
            false
        }
    }
}

impl Drop for TtySession {
    fn drop(&mut self) {
        // Best-effort cleanup - send SIGTERM to child
        if !self.closed.load(Ordering::Relaxed) {
            let _ = kill(Pid::from_raw(self.child_pid as i32), Signal::SIGTERM);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_creation() {
        // Skip if /bin/bash doesn't exist (e.g., in minimal containers)
        if !std::path::Path::new("/bin/bash").exists() {
            return;
        }

        let session = TtySession::spawn("/bin/bash", 80, 24).await.unwrap();
        assert!(!session.session_id().is_empty());
        assert!(session.child_pid() > 0);
        assert!(!session.is_closed());

        let (cols, rows) = session.size().await;
        assert_eq!(cols, 80);
        assert_eq!(rows, 24);

        // Clean up
        let _ = session.close().await;
    }
}
