use anyhow::{Context, Result};
use reqwest::Client;
use tracing::{debug, warn};

use crate::models::GammaMarket;

pub struct GammaApi {
    client: Client,
    base_url: String,
}

impl GammaApi {
    pub fn new(client: Client, base_url: &str) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch a single market by slug
    pub async fn get_market_by_slug(&self, slug: &str) -> Result<Option<GammaMarket>> {
        let url = format!("{}/markets", self.base_url);
        debug!(slug, "Fetching market by slug");

        let resp = self.client
            .get(&url)
            .query(&[("slug", slug)])
            .send()
            .await
            .context("Failed to fetch market")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("Gamma API returned {}: {}", status, body);
            return Ok(None);
        }

        // Gamma returns an array even for slug queries
        let markets: Vec<GammaMarket> = resp.json().await
            .context("Failed to parse market response")?;

        Ok(markets.into_iter().next())
    }

    /// Fetch a market by condition ID
    pub async fn get_market_by_condition(&self, condition_id: &str) -> Result<Option<GammaMarket>> {
        let url = format!("{}/markets", self.base_url);
        debug!(condition_id, "Fetching market by condition_id");

        let resp = self.client
            .get(&url)
            .query(&[("condition_id", condition_id)])
            .send()
            .await
            .context("Failed to fetch market by condition")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("Gamma API returned {}: {}", status, body);
            return Ok(None);
        }

        let markets: Vec<GammaMarket> = resp.json().await
            .unwrap_or_default();

        Ok(markets.into_iter().next())
    }

    /// Fetch multiple active markets with filters
    pub async fn get_active_markets(
        &self,
        limit: u32,
        category: Option<&str>,
    ) -> Result<Vec<GammaMarket>> {
        let url = format!("{}/markets", self.base_url);
        debug!(limit, category, "Fetching active markets");

        let mut query: Vec<(&str, String)> = vec![
            ("active", "true".to_string()),
            ("closed", "false".to_string()),
            ("limit", limit.to_string()),
            ("order", "volume_24hr".to_string()),
            ("ascending", "false".to_string()),
        ];

        if let Some(cat) = category {
            query.push(("tag_id", cat.to_string()));
        }

        let resp = self.client
            .get(&url)
            .query(&query.iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>())
            .send()
            .await
            .context("Failed to fetch active markets")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("Gamma markets API returned {}: {}", status, body);
            return Ok(vec![]);
        }

        let markets: Vec<GammaMarket> = resp.json().await
            .context("Failed to parse markets response")?;

        debug!(count = markets.len(), "Active markets fetched");
        Ok(markets)
    }
}
