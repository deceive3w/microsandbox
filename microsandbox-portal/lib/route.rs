//! Router configuration for the microsandbox portal.
//!
//! This module handles:
//! - API route definitions
//! - Router configuration and setup
//! - Request routing and handling

use axum::{Router, routing::{get, post}};
use tower_http::trace::TraceLayer;

use crate::{handler, portal::terminal, state::SharedState};

//--------------------------------------------------------------------------------------------------
// Functions
//--------------------------------------------------------------------------------------------------

/// Create a new router with the given state
pub fn create_router(state: SharedState) -> Router {
    // Create JSON-RPC routes - a single endpoint that handles all RPC methods
    // Using an adapter function to properly handle the state parameter
    let rpc_api = Router::new().route("/", post(handler::json_rpc_handler));

    // Create terminal state from shared state
    let terminal_state = terminal::TerminalState {
        manager: state.terminal_manager.clone(),
    };

    // Create WebSocket routes for terminal access
    let ws_api = Router::new()
        .route("/terminal", get(terminal::terminal_ws_handler))
        .with_state(terminal_state);

    // Combine all routes with tracing middleware
    Router::new()
        .nest("/api/v1/rpc", rpc_api)
        .nest("/ws", ws_api)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
