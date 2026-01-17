//! # microsandbox-terminal
//!
//! Interactive PTY/TTY terminal support for microsandbox.
//!
//! This crate provides:
//! - `TtySession`: PTY lifecycle management (spawn, read, write, resize, close)
//! - `TerminalMessage`: WebSocket message types for terminal communication
//! - `RateLimiter`: Leaky bucket rate limiting for terminal input
//!
//! ## Example
//!
//! ```ignore
//! use microsandbox_terminal::{TtySession, TerminalMessage};
//!
//! // Spawn a shell with PTY
//! let session = TtySession::spawn("/bin/bash", 80, 24).await?;
//!
//! // Write input to PTY
//! session.write(b"ls -la\n").await?;
//!
//! // Read output from PTY
//! let output = session.read().await?;
//!
//! // Resize terminal
//! session.resize(120, 40)?;
//!
//! // Close session
//! let exit_code = session.close().await?;
//! ```

mod error;
mod message;
mod rate_limit;
mod resize;
mod session;

pub use error::{TerminalError, TerminalResult};
pub use message::TerminalMessage;
pub use rate_limit::RateLimiter;
pub use resize::{get_window_size, set_window_size};
pub use session::TtySession;
