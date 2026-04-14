use anyhow::{Context, Result};
use reqwest::Client;
use tracing::debug;

use crate::models::OrderbookResponse;

pub struct ClobApi {
    client: Client,
    base_url: String,
}

impl ClobApi {
    pub fn new(client: Client, base_url: &str) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch orderbook for a specific token
    pub async fn get_orderbook(&self, token_id: &str) -> Result<OrderbookResponse> {
        let url = format!("{}/book", self.base_url);
        debug!(token_id, "Fetching orderbook");

        let resp = self.client
            .get(&url)
            .query(&[("token_id", token_id)])
            .send()
            .await
            .context("Failed to fetch orderbook")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("CLOB book API returned {}: {}", status, body);
        }

        let book: OrderbookResponse = resp.json().await
            .context("Failed to parse orderbook response")?;

        debug!(
            token_id,
            best_ask = ?book.best_ask(),
            "Orderbook fetched"
        );
        Ok(book)
    }

    /// Fetch midpoint price for a token
    pub async fn get_midpoint(&self, token_id: &str) -> Result<Option<f64>> {
        let url = format!("{}/midpoint", self.base_url);

        let resp = self.client
            .get(&url)
            .query(&[("token_id", token_id)])
            .send()
            .await
            .context("Failed to fetch midpoint")?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let body: serde_json::Value = resp.json().await
            .context("Failed to parse midpoint response")?;

        let mid = body.get("mid")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok());

        Ok(mid)
    }

    /// Fetch the fee rate for a token
    pub async fn get_fee_rate(&self, token_id: &str) -> Result<f64> {
        let url = format!("{}/fee-rate", self.base_url);

        let resp = self.client
            .get(&url)
            .query(&[("token_id", token_id)])
            .send()
            .await
            .context("Failed to fetch fee rate")?;

        if !resp.status().is_success() {
            return Ok(0.0);
        }

        let body: serde_json::Value = resp.json().await
            .unwrap_or(serde_json::json!({}));

        let fee = body.get("fee_rate_bps")
            .or_else(|| body.get("feeRate"))
            .and_then(|v| {
                v.as_str().and_then(|s| s.parse::<f64>().ok())
                    .or_else(|| v.as_f64())
            })
            .unwrap_or(0.0);

        Ok(fee)
    }
}
