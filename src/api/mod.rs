pub mod data;
pub mod gamma;
pub mod clob;

use reqwest::Client;
use std::time::Duration;

pub fn build_http_client() -> reqwest::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(5)
        .build()
}
