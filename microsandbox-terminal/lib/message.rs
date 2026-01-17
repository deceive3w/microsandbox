//! WebSocket message types for terminal communication

use serde::{Deserialize, Serialize};

/// Messages exchanged over WebSocket for terminal communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalMessage {
    /// Input data from client to PTY
    Input {
        /// Raw bytes as base64-encoded string
        data: String,
    },

    /// Output data from PTY to client
    Output {
        /// Raw bytes as base64-encoded string
        data: String,
    },

    /// Terminal resize request
    Resize {
        /// Number of columns
        cols: u16,
        /// Number of rows
        rows: u16,
    },

    /// Ping message for keepalive
    Ping,

    /// Pong response to ping
    Pong,

    /// Error message
    Error {
        /// Error description
        message: String,
    },

    /// Session initialization
    Init {
        /// Shell to spawn (e.g., "/bin/bash")
        shell: Option<String>,
        /// Initial columns
        cols: u16,
        /// Initial rows
        rows: u16,
    },

    /// Session initialized successfully
    Ready {
        /// Session identifier
        session_id: String,
    },

    /// Close the session
    Close,

    /// Session closed
    Closed {
        /// Process exit code
        exit_code: Option<i32>,
    },
}

impl TerminalMessage {
    /// Create an input message from raw bytes
    pub fn input(data: &[u8]) -> Self {
        use base64::Engine;
        Self::Input {
            data: base64::engine::general_purpose::STANDARD.encode(data),
        }
    }

    /// Create an output message from raw bytes
    pub fn output(data: &[u8]) -> Self {
        use base64::Engine;
        Self::Output {
            data: base64::engine::general_purpose::STANDARD.encode(data),
        }
    }

    /// Create an error message
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }

    /// Create a resize message
    pub fn resize(cols: u16, rows: u16) -> Self {
        Self::Resize { cols, rows }
    }

    /// Create an init message
    pub fn init(shell: Option<String>, cols: u16, rows: u16) -> Self {
        Self::Init { shell, cols, rows }
    }

    /// Create a ready message
    pub fn ready(session_id: impl Into<String>) -> Self {
        Self::Ready {
            session_id: session_id.into(),
        }
    }

    /// Create a closed message
    pub fn closed(exit_code: Option<i32>) -> Self {
        Self::Closed { exit_code }
    }

    /// Decode input data from base64
    pub fn decode_input(&self) -> Option<Vec<u8>> {
        use base64::Engine;
        match self {
            Self::Input { data } => {
                base64::engine::general_purpose::STANDARD.decode(data).ok()
            }
            _ => None,
        }
    }

    /// Decode output data from base64
    pub fn decode_output(&self) -> Option<Vec<u8>> {
        use base64::Engine;
        match self {
            Self::Output { data } => {
                base64::engine::general_purpose::STANDARD.decode(data).ok()
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_roundtrip() {
        let original = b"hello world";
        let msg = TerminalMessage::input(original);
        let decoded = msg.decode_input().unwrap();
        assert_eq!(original.as_slice(), decoded.as_slice());
    }

    #[test]
    fn test_output_roundtrip() {
        let original = b"\x1b[32mgreen text\x1b[0m";
        let msg = TerminalMessage::output(original);
        let decoded = msg.decode_output().unwrap();
        assert_eq!(original.as_slice(), decoded.as_slice());
    }

    #[test]
    fn test_serialize_resize() {
        let msg = TerminalMessage::resize(80, 24);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"resize\""));
        assert!(json.contains("\"cols\":80"));
        assert!(json.contains("\"rows\":24"));
    }
}
