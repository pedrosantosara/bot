use anyhow::{Context, Result};
use colored::Colorize;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::info;

use alloy_primitives::U256;
use polymarket_client_sdk::auth::{LocalSigner, Signer};
type PrivateKeySigner = LocalSigner<k256::ecdsa::SigningKey>;
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::clob::{Client, Config};
use polymarket_client_sdk::clob::types::{Amount, OrderType, Side, SignatureType};

type AuthClient = Client<Authenticated<Normal>>;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub struct Executor {
    client: AuthClient,
    signer: PrivateKeySigner,
    pub address: String,
}

impl Executor {
    pub async fn new() -> Result<Self> {
        let private_key = std::env::var("POLYMARKET_PRIVATE_KEY")
            .context("POLYMARKET_PRIVATE_KEY not set in .env")?;

        info!("Private key loaded ({} chars)", private_key.len());

        let signer = LocalSigner::from_str(&private_key)
            .context("Invalid private key format — deve ser hex com ou sem 0x prefix")?
            .with_chain_id(Some(137)); // Polygon mainnet

        info!("Signer created, address: {:?}", signer.address());

        let client = Client::new(
            "https://clob.polymarket.com",
            Config::default(),
        )
        .context("Failed to create CLOB client")?;

        info!("CLOB client created, authenticating...");

        let client = client
            .authentication_builder(&signer)
            .signature_type(SignatureType::Proxy)
            .authenticate()
            .await
            .context("CLOB authentication failed — verifique se sua wallet fez pelo menos 1 trade na Polymarket")?;

        info!("Polymarket CLOB authenticated successfully");

        let address = format!("{:?}", signer.address());
        Ok(Self { client, signer, address })
    }

    fn parse_token_id(token_id: &str) -> Result<U256> {
        U256::from_str(token_id).context("Invalid token ID")
    }

    /// BUY market order (FAK)
    #[allow(dead_code)]
    pub async fn buy(&self, token_id: &str, usdc_amount: f64) -> Result<OrderResult> {
        let amount = Decimal::from_str(&format!("{:.2}", usdc_amount))
            .context("Invalid USDC amount")?;
        let tid = Self::parse_token_id(token_id)?;

        let signable = match self.client
            .market_order()
            .token_id(tid)
            .amount(Amount::usdc(amount)?)
            .side(Side::Buy)
            .order_type(OrderType::FOK)
            .build()
            .await
        {
            Ok(s) => s,
            Err(e) => {
                println!("  {} {}", "❌", format!("Build order error (token {}...): {:?}", &token_id[..token_id.len().min(20)], e).red());
                anyhow::bail!("Failed to build order: {}", e);
            }
        };

        let signed = self.client
            .sign(&self.signer, signable)
            .await
            .context("Failed to sign order")?;

        let resp = match self.client.post_order(signed).await {
            Ok(r) => r,
            Err(e) => {
                println!("  {} {}", "❌", format!("Order POST error: {:?}", e).red());
                return Ok(OrderResult { success: false, status: format!("{:?}", e), fill_price: 0.0, filled_ts: 0 });
            }
        };

        let filled_ts = now_ms();
        let success = resp.success;
        let filled = resp.making_amount;

        if success {
            println!("  {} {}", "💰", format!("BUY filled: ${:.2} (order: {})", filled, &resp.order_id[..10]).green());
        } else {
            let err = resp.error_msg.as_deref().unwrap_or("unknown");
            println!("  {} {}", "❌", format!("BUY failed: {}", err).red());
        }

        Ok(OrderResult { success, status: format!("{:?}", resp.status), fill_price: 0.0, filled_ts })
    }

    /// BUY limit order at specific price (for hedge legs)
    pub async fn buy_limit(&self, token_id: &str, shares: f64, max_price: f64) -> Result<OrderResult> {
        let size = Decimal::from_str(&format!("{:.2}", shares))
            .context("Invalid share size")?;
        let price_dec = Decimal::from_str(&format!("{:.2}", max_price))
            .context("Invalid price")?;
        let tid = Self::parse_token_id(token_id)?;

        let signable = match self.client
            .limit_order()
            .token_id(tid)
            .size(size)
            .price(price_dec)
            .side(Side::Buy)
            .order_type(OrderType::GTC)
            .build()
            .await
        {
            Ok(s) => s,
            Err(e) => {
                println!("  {} {}", "❌", format!("Build limit order error: {:?}", e).red());
                anyhow::bail!("Failed to build limit order: {}", e);
            }
        };

        let signed = self.client.sign(&self.signer, signable).await
            .context("Failed to sign limit order")?;

        let resp = match self.client.post_order(signed).await {
            Ok(r) => r,
            Err(e) => {
                println!("  {} {}", "❌", format!("Limit order POST error: {:?}", e).red());
                return Ok(OrderResult { success: false, status: format!("{:?}", e), fill_price: 0.0, filled_ts: 0 });
            }
        };

        let filled_ts = now_ms();
        let success = resp.success;
        if success {
            println!("  {} {}", "💰", format!("Leg filled: {} shares @{:.3} (${:.2})", shares, max_price, shares * max_price).green());
        } else {
            let err = resp.error_msg.as_deref().unwrap_or("unknown");
            println!("  {} {}", "❌", format!("Leg failed: {}", err).red());
        }

        Ok(OrderResult { success, status: format!("{:?}", resp.status), fill_price: max_price, filled_ts })
    }

    /// Cancel all open orders on the account
    pub async fn cancel_all_orders(&self) -> Result<()> {
        let resp = self.client.cancel_all_orders().await
            .context("Failed to cancel all orders")?;
        let n = resp.canceled.len();
        if n > 0 {
            info!("Canceled {} orders", n);
        }
        if !resp.not_canceled.is_empty() {
            info!("Not canceled: {:?}", resp.not_canceled);
        }
        Ok(())
    }

    /// SELL limit order (GTD)
    #[allow(dead_code)]
    pub async fn sell(&self, token_id: &str, shares: f64, price: f64) -> Result<OrderResult> {
        let size = Decimal::from_str(&format!("{:.6}", shares))
            .context("Invalid share size")?;
        let price_dec = Decimal::from_str(&format!("{:.4}", price))
            .context("Invalid price")?;
        let tid = Self::parse_token_id(token_id)?;

        let signable = self.client
            .limit_order()
            .token_id(tid)
            .size(size)
            .price(price_dec)
            .side(Side::Sell)
            .build()
            .await
            .context("Failed to build sell order")?;

        let signed = self.client
            .sign(&self.signer, signable)
            .await
            .context("Failed to sign sell order")?;

        let resp = self.client
            .post_order(signed)
            .await
            .context("Failed to post sell order")?;

        let filled_ts = now_ms();
        let status = format!("{:?}", resp);
        let success = status.contains("MATCHED") || status.contains("ACTIVE");

        Ok(OrderResult { success, status: format!("{:?}", resp.status), fill_price: price, filled_ts })
    }
}

pub struct OrderResult {
    pub success: bool,
    #[allow(dead_code)]
    pub status: String,
    pub fill_price: f64,       // price the order actually filled at
    pub filled_ts: i64,        // epoch ms when fill confirmed
}
