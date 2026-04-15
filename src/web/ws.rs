use axum::{
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use tracing::debug;

use crate::web::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.ws_tx.subscribe();

    debug!("WebSocket client connected");

    // Send initial data (ignore errors — client may disconnect immediately)
    if let Ok(stats) = state.db.get_trade_stats(None).await {
        let msg = serde_json::to_string(&crate::models::WsEvent::StatsUpdate(stats)).unwrap_or_default();
        if sender.send(Message::Text(msg.into())).await.is_err() { return; }
    }
    if let Ok(status) = state.db.get_bot_status().await {
        let msg = serde_json::to_string(&crate::models::WsEvent::BotStatusChanged(status)).unwrap_or_default();
        if sender.send(Message::Text(msg.into())).await.is_err() { return; }
    }
    if let Ok(balance) = state.db.get_balance_info().await {
        let msg = serde_json::to_string(&crate::models::WsEvent::BalanceUpdate(balance)).unwrap_or_default();
        if sender.send(Message::Text(msg.into())).await.is_err() { return; }
    }

    // Send recent logs
    let recent_logs = state.recent_logs().await;
    for entry in recent_logs {
        let msg = serde_json::to_string(&crate::models::WsEvent::LogEntry(entry)).unwrap_or_default();
        if sender.send(Message::Text(msg.into())).await.is_err() { return; }
    }

    // Forward broadcast events to this client
    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let msg = serde_json::to_string(&event).unwrap_or_default();
                    if sender.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    // Read from client (keep connection alive)
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    debug!("WebSocket client disconnected");
}
