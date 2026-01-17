//! Error types for microsandbox-terminal

use thiserror::Error;

/// Result type for terminal operations
pub type TerminalResult<T> = Result<T, TerminalError>;

/// Errors that can occur during terminal operations
#[derive(Debug, Error)]
pub enum TerminalError {
    /// PTY allocation failed
    #[error("Failed to allocate PTY: {0}")]
    PtyAllocation(#[from] nix::Error),

    /// Failed to spawn shell process
    #[error("Failed to spawn shell: {0}")]
    SpawnFailed(#[from] std::io::Error),

    /// Failed to read from PTY
    #[error("Failed to read from PTY: {0}")]
    ReadFailed(String),

    /// Failed to write to PTY
    #[error("Failed to write to PTY: {0}")]
    WriteFailed(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {0} requests/sec allowed")]
    RateLimited(u32),

    /// Session not initialized
    #[error("Terminal session not initialized")]
    NotInitialized,

    /// Session already closed
    #[error("Terminal session already closed")]
    AlreadyClosed,

    /// Invalid terminal size
    #[error("Invalid terminal size: cols={cols}, rows={rows}")]
    InvalidSize { cols: u16, rows: u16 },

    /// Process signal error
    #[error("Failed to signal process: {0}")]
    SignalFailed(String),

    /// Timeout waiting for process
    #[error("Timeout waiting for process")]
    Timeout,
}
