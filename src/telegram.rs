use reqwest::Client;
use tracing::{info, warn};

#[derive(Clone)]
pub struct TelegramBot {
    client: Client,
    token: String,
    chat_id: String,
}

impl TelegramBot {
    pub fn new(client: Client, token: &str, chat_id: &str) -> Self {
        Self {
            client,
            token: token.to_string(),
            chat_id: chat_id.to_string(),
        }
    }

    /// Try to create from env vars. Returns None if token or chat_id missing.
    pub fn from_env(client: Client) -> Option<Self> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID").ok()?;
        if token.is_empty() || chat_id.is_empty() {
            return None;
        }
        info!("Telegram notifications enabled (chat_id: {})", chat_id);
        Some(Self::new(client, &token, &chat_id))
    }

    /// Send a message — returns the Telegram message_id (0 on failure)
    pub async fn send(&self, text: &str) -> i64 {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let res = self.client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": self.chat_id,
                "text": text,
                "parse_mode": "HTML",
                "disable_web_page_preview": true,
            }))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {
                // Extract message_id from response
                if let Ok(body) = r.json::<serde_json::Value>().await {
                    body.get("result")
                        .and_then(|r| r.get("message_id"))
                        .and_then(|id| id.as_i64())
                        .unwrap_or(0)
                } else {
                    0
                }
            }
            Ok(r) => { warn!("Telegram send failed: {}", r.status()); 0 }
            Err(e) => { warn!("Telegram send error: {}", e); 0 }
        }
    }

    /// Edit an existing message by message_id
    pub async fn edit(&self, message_id: i64, text: &str) {
        if message_id == 0 { return; }
        let url = format!("https://api.telegram.org/bot{}/editMessageText", self.token);
        let res = self.client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": self.chat_id,
                "message_id": message_id,
                "text": text,
                "parse_mode": "HTML",
                "disable_web_page_preview": true,
            }))
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => warn!("Telegram edit failed: {}", r.status()),
            Err(e) => warn!("Telegram edit error: {}", e),
        }
    }

    /// Send a batch of messages as a single combined message
    pub async fn send_batch(&self, messages: &[String]) -> i64 {
        if messages.is_empty() {
            return 0;
        }
        if messages.len() == 1 {
            return self.send(&messages[0]).await;
        }
        // Combine into one message (Telegram limit is 4096 chars)
        let combined = messages.join("\n\n---\n\n");
        if combined.len() <= 4000 {
            self.send(&combined).await
        } else {
            let mut last_id = 0;
            for msg in messages {
                last_id = self.send(msg).await;
            }
            last_id
        }
    }
}

/// Format a trade entry notification
pub fn format_entry(
    wallet_label: &str,
    outcome: &str,
    market: &str,
    cost: f64,
    price: f64,
    whale_price: f64,
    mode: &str,
) -> String {
    let mode_tag = if mode == "live" { "LIVE" } else { "TEST" };
    format!(
        "▶ <b>ENTRY</b> [{mode_tag}]\n\
         Wallet: <code>{wallet_label}</code>\n\
         Market: {market}\n\
         Outcome: <b>{outcome}</b>\n\
         Price: {price:.3} (whale: {whale_price:.3})\n\
         Cost: <b>${cost:.2}</b>\n\
         ⏳ Aguardando resultado..."
    )
}

/// Format the edited message after resolution (replaces the original entry message)
pub fn format_entry_resolved(
    wallet_label: &str,
    outcome: &str,
    market: &str,
    cost: f64,
    price: f64,
    whale_price: f64,
    mode: &str,
    winner: &str,
    pnl: f64,
    won: bool,
) -> String {
    let mode_tag = if mode == "live" { "LIVE" } else { "TEST" };
    let emoji = if won { "✅" } else { "❌" };
    let result_tag = if won { "WIN" } else { "LOSS" };
    let pnl_str = if pnl >= 0.0 {
        format!("+${:.2}", pnl)
    } else {
        format!("-${:.2}", pnl.abs())
    };
    format!(
        "{emoji} <b>{result_tag}</b> [{mode_tag}]\n\
         Wallet: <code>{wallet_label}</code>\n\
         Market: {market}\n\
         Outcome: <b>{outcome}</b> | Winner: <b>{winner}</b>\n\
         Price: {price:.3} (whale: {whale_price:.3})\n\
         Cost: <b>${cost:.2}</b>\n\
         P&L: <b>{pnl_str}</b>"
    )
}

/// Format a trade resolution notification (standalone, used as fallback)
pub fn format_resolution(
    market: &str,
    outcome: &str,
    winner: &str,
    pnl: f64,
    won: bool,
) -> String {
    let emoji = if won { "✅" } else { "❌" };
    let pnl_str = if pnl >= 0.0 {
        format!("+${:.2}", pnl)
    } else {
        format!("-${:.2}", pnl.abs())
    };
    format!(
        "{emoji} <b>{result}</b>\n\
         Market: {market}\n\
         Bet: {outcome} | Winner: <b>{winner}</b>\n\
         P&L: <b>{pnl_str}</b>",
        result = if won { "WIN" } else { "LOSS" },
    )
}

/// Format whale exit notification
pub fn format_exit(
    market: &str,
    outcome: &str,
    exit_price: f64,
    pnl: f64,
    reason: &str,
) -> String {
    let pnl_str = if pnl >= 0.0 {
        format!("+${:.2}", pnl)
    } else {
        format!("-${:.2}", pnl.abs())
    };
    format!(
        "◀ <b>EXIT</b>\n\
         Market: {market}\n\
         Outcome: {outcome}\n\
         Exit price: {exit_price:.3}\n\
         P&L: <b>{pnl_str}</b>\n\
         Reason: {reason}"
    )
}
