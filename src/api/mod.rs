pub mod data;
pub mod gamma;
pub mod clob;

use reqwest::Client;
use std::time::Duration;

pub fn build_http_client() -> reqwest::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(300))
        .tcp_nodelay(true)
        .build()
}

/// Pre-warm TLS connections to Polymarket APIs so first trade has no handshake delay
pub async fn warmup_connections(client: &Client) {
    let urls = [
        "https://clob.polymarket.com/time",
        "https://data-api.polymarket.com/trades?limit=1",
        "https://gamma-api.polymarket.com/events?limit=1",
    ];
    for url in &urls {
        let _ = client.get(*url).send().await;
    }
}
