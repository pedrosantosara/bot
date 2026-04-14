use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::db::Database;
use crate::models::WsEvent;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub http_client: reqwest::Client,
    pub ws_tx: broadcast::Sender<WsEvent>,
    pub bot_running: Arc<RwLock<bool>>,
}

impl AppState {
    pub fn new(db: Database, http_client: reqwest::Client) -> Self {
        let (ws_tx, _) = broadcast::channel(256);
        Self {
            db,
            http_client,
            ws_tx,
            bot_running: Arc::new(RwLock::new(false)),
        }
    }

    pub fn broadcast(&self, event: WsEvent) {
        let _ = self.ws_tx.send(event);
    }
}
