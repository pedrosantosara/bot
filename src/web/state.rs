use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock, Mutex};

use crate::db::Database;
use crate::models::{LogEntry, WsEvent};
use crate::telegram::TelegramBot;

const LOG_BUFFER_SIZE: usize = 500;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub http_client: reqwest::Client,
    pub ws_tx: broadcast::Sender<WsEvent>,
    pub bot_running: Arc<RwLock<bool>>,
    pub log_buffer: Arc<Mutex<VecDeque<LogEntry>>>,
    pub telegram: Option<TelegramBot>,
}

impl AppState {
    pub fn new(db: Database, http_client: reqwest::Client) -> Self {
        let (ws_tx, _) = broadcast::channel(256);
        let telegram = TelegramBot::from_env(http_client.clone());
        Self {
            db,
            http_client,
            ws_tx,
            bot_running: Arc::new(RwLock::new(false)),
            log_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(LOG_BUFFER_SIZE))),
            telegram,
        }
    }

    pub fn broadcast(&self, event: WsEvent) {
        let _ = self.ws_tx.send(event);
    }

    pub async fn push_log(&self, entry: LogEntry) {
        {
            let mut buf = self.log_buffer.lock().await;
            if buf.len() >= LOG_BUFFER_SIZE {
                buf.pop_front();
            }
            buf.push_back(entry.clone());
        }
        self.broadcast(WsEvent::LogEntry(entry));
    }

    pub async fn recent_logs(&self) -> Vec<LogEntry> {
        let buf = self.log_buffer.lock().await;
        buf.iter().cloned().collect()
    }
}
