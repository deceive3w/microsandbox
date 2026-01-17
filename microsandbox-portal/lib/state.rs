//! Shared state management for the microsandbox portal server.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::portal::{command::CommandHandle, repl::EngineHandle, terminal::TerminalSessionManager};

//--------------------------------------------------------------------------------------------------
// Types
//--------------------------------------------------------------------------------------------------

/// SharedState for the server
#[derive(Clone)]
pub struct SharedState {
    /// Indicates if the server is ready to process requests
    pub ready: Arc<Mutex<bool>>,

    /// Engine handle for REPL environment
    pub engine_handle: Arc<Mutex<Option<EngineHandle>>>,

    /// Command handle for command execution
    pub command_handle: Arc<Mutex<Option<CommandHandle>>>,

    /// Terminal session manager for interactive PTY sessions
    pub terminal_manager: TerminalSessionManager,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            ready: Arc::new(Mutex::new(false)),
            engine_handle: Arc::new(Mutex::new(None)),
            command_handle: Arc::new(Mutex::new(None)),
            terminal_manager: TerminalSessionManager::new(),
        }
    }
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("ready", &self.ready)
            .field("engine_handle", &self.engine_handle)
            .field("command_handle", &self.command_handle)
            .field("terminal_manager", &"<TerminalSessionManager>")
            .finish()
    }
}
