//! WebSocket handler for interactive terminal sessions.
//!
//! Provides PTY-based terminal access via WebSocket connections.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use microsandbox_terminal::{TerminalMessage, TtySession};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

//--------------------------------------------------------------------------------------------------
// Types
//--------------------------------------------------------------------------------------------------

/// Manages active terminal sessions
#[derive(Clone, Default)]
pub struct TerminalSessionManager {
    /// Active sessions by session ID
    sessions: Arc<RwLock<HashMap<String, Arc<TtySession>>>>,
}

impl TerminalSessionManager {
    /// Create a new terminal session manager
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new terminal session
    pub async fn create_session(
        &self,
        shell: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<Arc<TtySession>, String> {
        let shell = shell.unwrap_or_else(|| "/bin/bash".to_string());

        let session = TtySession::spawn(&shell, cols, rows)
            .await
            .map_err(|e| e.to_string())?;

        let session_id = session.session_id().to_string();
        let session = Arc::new(session);

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), Arc::clone(&session));
        }

        info!(session_id = %session_id, "Terminal session created");
        Ok(session)
    }

    /// Get an existing session by ID
    pub async fn get_session(&self, session_id: &str) -> Option<Arc<TtySession>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    /// Remove a session
    pub async fn remove_session(&self, session_id: &str) -> Option<Arc<TtySession>> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id)
    }

    /// Get count of active sessions
    pub async fn session_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.len()
    }

    /// Clean up idle sessions
    pub async fn cleanup_idle_sessions(&self, max_idle: Duration) {
        let mut sessions = self.sessions.write().await;
        let mut to_remove = Vec::new();

        for (id, session) in sessions.iter() {
            if session.idle_duration().await > max_idle {
                to_remove.push(id.clone());
            }
        }

        for id in to_remove {
            if let Some(session) = sessions.remove(&id) {
                info!(session_id = %id, "Removing idle terminal session");
                let _ = session.close().await;
            }
        }
    }
}

//--------------------------------------------------------------------------------------------------
// WebSocket Handler
//--------------------------------------------------------------------------------------------------

/// State for terminal WebSocket handler
#[derive(Clone)]
pub struct TerminalState {
    /// Terminal session manager
    pub manager: TerminalSessionManager,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self {
            manager: TerminalSessionManager::new(),
        }
    }
}

/// WebSocket upgrade handler for terminal connections
pub async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<TerminalState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_terminal_connection(socket, state))
}

/// Handle a terminal WebSocket connection
async fn handle_terminal_connection(socket: WebSocket, state: TerminalState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Wait for init message
    let init_msg = match ws_receiver.next().await {
        Some(Ok(Message::Text(text))) => {
            match serde_json::from_str::<TerminalMessage>(&text) {
                Ok(TerminalMessage::Init { shell, cols, rows }) => (shell, cols, rows),
                Ok(_) => {
                    let _ = ws_sender
                        .send(Message::Text(
                            serde_json::to_string(&TerminalMessage::error(
                                "Expected Init message",
                            ))
                            .unwrap().into(),
                        ))
                        .await;
                    return;
                }
                Err(e) => {
                    let _ = ws_sender
                        .send(Message::Text(
                            serde_json::to_string(&TerminalMessage::error(format!(
                                "Invalid message: {}",
                                e
                            )))
                            .unwrap().into(),
                        ))
                        .await;
                    return;
                }
            }
        }
        _ => {
            error!("Failed to receive init message");
            return;
        }
    };

    let (shell, cols, rows) = init_msg;

    // Create terminal session
    let session = match state.manager.create_session(shell, cols, rows).await {
        Ok(s) => s,
        Err(e) => {
            let _ = ws_sender
                .send(Message::Text(
                    serde_json::to_string(&TerminalMessage::error(format!(
                        "Failed to create session: {}",
                        e
                    )))
                    .unwrap().into(),
                ))
                .await;
            return;
        }
    };

    let session_id = session.session_id().to_string();

    // Send ready message
    if let Err(e) = ws_sender
        .send(Message::Text(
            serde_json::to_string(&TerminalMessage::ready(&session_id))
                .unwrap().into(),
        ))
        .await
    {
        error!(error = %e, "Failed to send ready message");
        let _ = session.close().await;
        state.manager.remove_session(&session_id).await;
        return;
    }

    info!(session_id = %session_id, "Terminal WebSocket connected");

    // Wrap sender in mutex for shared access
    let ws_sender = Arc::new(Mutex::new(ws_sender));

    // Spawn task to read from PTY and send to WebSocket
    let read_session = Arc::clone(&session);
    let read_sender = Arc::clone(&ws_sender);
    let read_session_id = session_id.clone();
    let read_task = tokio::spawn(async move {
        loop {
            if read_session.is_closed() {
                break;
            }

            match read_session.read().await {
                Ok(data) if !data.is_empty() => {
                    let msg = TerminalMessage::output(&data);
                    let json = match serde_json::to_string(&msg) {
                        Ok(j) => j,
                        Err(e) => {
                            error!(error = %e, "Failed to serialize output");
                            continue;
                        }
                    };

                    let mut sender = read_sender.lock().await;
                    if let Err(e) = sender.send(Message::Text(json.into())).await {
                        warn!(error = %e, session_id = %read_session_id, "Failed to send output");
                        break;
                    }
                }
                Ok(_) => {
                    // No data available, small delay
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => {
                    if !read_session.is_closed() {
                        error!(error = %e, session_id = %read_session_id, "Error reading from PTY");
                    }
                    break;
                }
            }
        }
    });

    // Handle incoming WebSocket messages
    let write_session = Arc::clone(&session);
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                match serde_json::from_str::<TerminalMessage>(&text) {
                    Ok(TerminalMessage::Input { data }) => {
                        // Decode base64 input
                        use base64::Engine;
                        if let Ok(bytes) =
                            base64::engine::general_purpose::STANDARD.decode(&data)
                        {
                            if let Err(e) = write_session.write(&bytes).await {
                                warn!(error = %e, session_id = %session_id, "Failed to write to PTY");
                                if matches!(e, microsandbox_terminal::TerminalError::RateLimited(_)) {
                                    let mut sender = ws_sender.lock().await;
                                    let _ = sender
                                        .send(Message::Text(
                                            serde_json::to_string(&TerminalMessage::error(
                                                "Rate limited",
                                            ))
                                            .unwrap().into(),
                                        ))
                                        .await;
                                }
                            }
                        }
                    }
                    Ok(TerminalMessage::Resize { cols, rows }) => {
                        if let Err(e) = write_session.resize(cols, rows).await {
                            warn!(error = %e, session_id = %session_id, "Failed to resize terminal");
                        }
                    }
                    Ok(TerminalMessage::Ping) => {
                        let mut sender = ws_sender.lock().await;
                        let _ = sender
                            .send(Message::Text(
                                serde_json::to_string(&TerminalMessage::Pong).unwrap().into(),
                            ))
                            .await;
                    }
                    Ok(TerminalMessage::Close) => {
                        info!(session_id = %session_id, "Client requested close");
                        break;
                    }
                    Ok(_) => {
                        // Ignore other message types
                    }
                    Err(e) => {
                        warn!(error = %e, session_id = %session_id, "Invalid message format");
                    }
                }
            }
            Ok(Message::Binary(data)) => {
                // Treat binary messages as raw input
                if let Err(e) = write_session.write(&data).await {
                    warn!(error = %e, session_id = %session_id, "Failed to write binary to PTY");
                }
            }
            Ok(Message::Ping(data)) => {
                let mut sender = ws_sender.lock().await;
                let _ = sender.send(Message::Pong(data)).await;
            }
            Ok(Message::Close(_)) => {
                info!(session_id = %session_id, "WebSocket closed by client");
                break;
            }
            Err(e) => {
                error!(error = %e, session_id = %session_id, "WebSocket error");
                break;
            }
            _ => {}
        }
    }

    // Cleanup
    read_task.abort();

    let exit_code = session.close().await.ok().flatten();
    state.manager.remove_session(&session_id).await;

    // Send closed message
    {
        let mut sender = ws_sender.lock().await;
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&TerminalMessage::closed(exit_code))
                    .unwrap().into(),
            ))
            .await;
        let _ = sender.close().await;
    }

    info!(session_id = %session_id, exit_code = ?exit_code, "Terminal session closed");
}

//--------------------------------------------------------------------------------------------------
// Cleanup Task
//--------------------------------------------------------------------------------------------------

/// Spawn a background task to cleanup idle sessions
pub fn spawn_cleanup_task(manager: TerminalSessionManager, idle_timeout: Duration) {
    tokio::spawn(async move {
        let check_interval = idle_timeout / 2;
        loop {
            tokio::time::sleep(check_interval).await;
            manager.cleanup_idle_sessions(idle_timeout).await;
        }
    });
}
