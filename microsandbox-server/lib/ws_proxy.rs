//! WebSocket proxy for terminal connections to sandboxes.
//!
//! Proxies WebSocket connections from clients to the appropriate sandbox portal.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

use crate::{ServerError, ValidationError, state::AppState};

//--------------------------------------------------------------------------------------------------
// Constants
//--------------------------------------------------------------------------------------------------

/// Timeout for connecting to portal WebSocket
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Sandbox name validation regex pattern
const SANDBOX_NAME_PATTERN: &str = r"^[a-zA-Z0-9_-]{1,64}$";

//--------------------------------------------------------------------------------------------------
// WebSocket Proxy Handler
//--------------------------------------------------------------------------------------------------

/// WebSocket upgrade handler for terminal proxy
///
/// Route: `/ws/terminal/:namespace/:sandbox`
pub async fn terminal_ws_proxy(
    ws: WebSocketUpgrade,
    Path((namespace, sandbox)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    // Validate sandbox name
    let re = regex::Regex::new(SANDBOX_NAME_PATTERN).unwrap();
    if !re.is_match(&sandbox) {
        return Err(ServerError::ValidationError(ValidationError::InvalidInput(
            format!("Invalid sandbox name: {}", sandbox),
        )));
    }
    if !re.is_match(&namespace) {
        return Err(ServerError::ValidationError(ValidationError::InvalidInput(
            format!("Invalid namespace: {}", namespace),
        )));
    }

    info!(
        namespace = %namespace,
        sandbox = %sandbox,
        "Terminal WebSocket proxy request"
    );

    // Get portal URL for this sandbox
    let portal_url = state
        .get_portal_url_for_sandbox(&namespace, &sandbox)
        .await?;

    // Build WebSocket URL for portal terminal endpoint
    let portal_ws_url = format!(
        "{}",
        portal_url.replace("http://", "ws://").replace("https://", "wss://")
    ) + "/ws/terminal";

    info!(
        portal_url = %portal_ws_url,
        "Connecting to portal terminal"
    );

    Ok(ws.on_upgrade(move |client_socket| {
        handle_proxy_connection(client_socket, portal_ws_url, namespace, sandbox)
    }))
}

/// Handle the bidirectional proxy connection
async fn handle_proxy_connection(
    client_socket: WebSocket,
    portal_ws_url: String,
    namespace: String,
    sandbox: String,
) {
    // Connect to portal WebSocket
    let portal_socket = match connect_to_portal(&portal_ws_url).await {
        Ok(socket) => socket,
        Err(e) => {
            error!(
                error = %e,
                portal_url = %portal_ws_url,
                "Failed to connect to portal WebSocket"
            );
            return;
        }
    };

    info!(
        namespace = %namespace,
        sandbox = %sandbox,
        "Terminal proxy established"
    );

    // Split both sockets
    let (mut client_sender, mut client_receiver) = client_socket.split();
    let (mut portal_sender, mut portal_receiver) = portal_socket.split();

    // Task to forward client -> portal
    let client_to_portal = async {
        while let Some(msg) = client_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = portal_sender
                        .send(tokio_tungstenite::tungstenite::Message::Text(text.to_string().into()))
                        .await
                    {
                        warn!(error = %e, "Error forwarding to portal");
                        break;
                    }
                }
                Ok(Message::Binary(data)) => {
                    if let Err(e) = portal_sender
                        .send(tokio_tungstenite::tungstenite::Message::Binary(data.to_vec().into()))
                        .await
                    {
                        warn!(error = %e, "Error forwarding binary to portal");
                        break;
                    }
                }
                Ok(Message::Ping(data)) => {
                    if let Err(e) = portal_sender
                        .send(tokio_tungstenite::tungstenite::Message::Ping(data.to_vec().into()))
                        .await
                    {
                        warn!(error = %e, "Error forwarding ping to portal");
                        break;
                    }
                }
                Ok(Message::Pong(data)) => {
                    if let Err(e) = portal_sender
                        .send(tokio_tungstenite::tungstenite::Message::Pong(data.to_vec().into()))
                        .await
                    {
                        warn!(error = %e, "Error forwarding pong to portal");
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    let _ = portal_sender
                        .send(tokio_tungstenite::tungstenite::Message::Close(None))
                        .await;
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "Client WebSocket error");
                    break;
                }
            }
        }
    };

    // Task to forward portal -> client
    let portal_to_client = async {
        while let Some(msg) = portal_receiver.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    if let Err(e) = client_sender.send(Message::Text(text.to_string().into())).await {
                        warn!(error = %e, "Error forwarding to client");
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                    if let Err(e) = client_sender.send(Message::Binary(data.to_vec().into())).await {
                        warn!(error = %e, "Error forwarding binary to client");
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Ping(data)) => {
                    if let Err(e) = client_sender.send(Message::Ping(data.to_vec().into())).await {
                        warn!(error = %e, "Error forwarding ping to client");
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Pong(data)) => {
                    if let Err(e) = client_sender.send(Message::Pong(data.to_vec().into())).await {
                        warn!(error = %e, "Error forwarding pong to client");
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    let _ = client_sender.send(Message::Close(None)).await;
                    break;
                }
                Ok(tokio_tungstenite::tungstenite::Message::Frame(_)) => {
                    // Raw frames are not typically forwarded
                }
                Err(e) => {
                    warn!(error = %e, "Portal WebSocket error");
                    break;
                }
            }
        }
    };

    // Run both tasks concurrently, stop when either finishes
    tokio::select! {
        _ = client_to_portal => {
            info!(namespace = %namespace, sandbox = %sandbox, "Client connection closed");
        }
        _ = portal_to_client => {
            info!(namespace = %namespace, sandbox = %sandbox, "Portal connection closed");
        }
    }

    info!(
        namespace = %namespace,
        sandbox = %sandbox,
        "Terminal proxy closed"
    );
}

/// Connect to the portal WebSocket
async fn connect_to_portal(
    url: &str,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, String> {
    let connect_future = tokio_tungstenite::connect_async(url);

    match tokio::time::timeout(CONNECT_TIMEOUT, connect_future).await {
        Ok(Ok((socket, _response))) => Ok(socket),
        Ok(Err(e)) => Err(format!("WebSocket connection failed: {}", e)),
        Err(_) => Err("Connection timeout".to_string()),
    }
}
