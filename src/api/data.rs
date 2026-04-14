use anyhow::{Context, Result};
use reqwest::Client;
use tracing::{debug, warn};

use crate::models::{LeaderboardEntry, WalletPosition, WalletTrade};

pub struct DataApi {
    client: Client,
    base_url: String,
}

impl DataApi {
    pub fn new(client: Client, base_url: &str) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch top traders from the leaderboard
    pub async fn get_leaderboard(
        &self,
        category: &str,
        period: &str,
        order_by: &str,
        limit: u32,
    ) -> Result<Vec<LeaderboardEntry>> {
        let url = format!("{}/v1/leaderboard", self.base_url);
        debug!(url = %url, category, period, "Fetching leaderboard");

        let resp = self.client
            .get(&url)
            .query(&[
                ("category", category),
                ("timePeriod", period),
                ("orderBy", order_by),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch leaderboard")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Leaderboard API returned {}: {}", status, body);
        }

        let entries: Vec<LeaderboardEntry> = resp.json().await
            .context("Failed to parse leaderboard response")?;

        debug!(count = entries.len(), "Leaderboard entries fetched");
        Ok(entries)
    }

    /// Fetch recent trades for a specific wallet
    pub async fn get_trades(
        &self,
        wallet: &str,
        limit: u32,
    ) -> Result<Vec<WalletTrade>> {
        let url = format!("{}/trades", self.base_url);
        debug!(url = %url, wallet, limit, "Fetching wallet trades");

        let resp = self.client
            .get(&url)
            .query(&[
                ("user", wallet),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch trades")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("Trades API returned {}: {}", status, body);
            return Ok(vec![]);
        }

        let trades: Vec<WalletTrade> = resp.json().await
            .context("Failed to parse trades response")?;

        debug!(count = trades.len(), wallet, "Trades fetched");
        Ok(trades)
    }

    /// Fetch current positions for a wallet
    pub async fn get_positions(
        &self,
        wallet: &str,
    ) -> Result<Vec<WalletPosition>> {
        let url = format!("{}/positions", self.base_url);
        debug!(url = %url, wallet, "Fetching wallet positions");

        let resp = self.client
            .get(&url)
            .query(&[
                ("user", wallet),
                ("limit", "100"),
                ("sizeThreshold", "0.1"),
            ])
            .send()
            .await
            .context("Failed to fetch positions")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("Positions API returned {}: {}", status, body);
            return Ok(vec![]);
        }

        let positions: Vec<WalletPosition> = resp.json().await
            .context("Failed to parse positions response")?;

        debug!(count = positions.len(), wallet, "Positions fetched");
        Ok(positions)
    }
}
