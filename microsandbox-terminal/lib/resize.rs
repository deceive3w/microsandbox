//! Terminal window size management

use crate::error::{TerminalError, TerminalResult};
use nix::libc;
use std::os::unix::io::AsRawFd;

/// Set the window size of a terminal
///
/// # Arguments
///
/// * `fd` - File descriptor of the PTY master or slave
/// * `cols` - Number of columns
/// * `rows` - Number of rows
///
/// # Example
///
/// ```ignore
/// use microsandbox_terminal::set_window_size;
/// use std::os::unix::io::AsRawFd;
///
/// // Resize PTY to 120x40
/// set_window_size(&pty_master, 120, 40)?;
/// ```
pub fn set_window_size<F: AsRawFd>(fd: &F, cols: u16, rows: u16) -> TerminalResult<()> {
    // Validate size
    if cols == 0 || rows == 0 {
        return Err(TerminalError::InvalidSize { cols, rows });
    }

    // Maximum reasonable terminal size
    if cols > 1000 || rows > 500 {
        return Err(TerminalError::InvalidSize { cols, rows });
    }

    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // SAFETY: We're passing a valid winsize struct to ioctl
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ, &winsize) };

    if result < 0 {
        Err(TerminalError::PtyAllocation(nix::Error::last()))
    } else {
        Ok(())
    }
}

/// Get the window size of a terminal
///
/// # Arguments
///
/// * `fd` - File descriptor of the PTY master or slave
///
/// # Returns
///
/// Tuple of (cols, rows)
pub fn get_window_size<F: AsRawFd>(fd: &F) -> TerminalResult<(u16, u16)> {
    let mut winsize = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // SAFETY: We're passing a valid winsize struct to ioctl
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCGWINSZ, &mut winsize) };

    if result < 0 {
        Err(TerminalError::PtyAllocation(nix::Error::last()))
    } else {
        Ok((winsize.ws_col, winsize.ws_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_size_zero() {
        // We can't easily test with a real fd, but we can test validation
        // This would fail before the ioctl call
        let result = set_window_size(&std::io::stdout(), 0, 24);
        assert!(matches!(result, Err(TerminalError::InvalidSize { cols: 0, rows: 24 })));
    }

    #[test]
    fn test_invalid_size_too_large() {
        let result = set_window_size(&std::io::stdout(), 2000, 24);
        assert!(matches!(result, Err(TerminalError::InvalidSize { cols: 2000, .. })));
    }
}
