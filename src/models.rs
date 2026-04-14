use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Polymarket Data API types ──

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    pub rank: String,
    pub proxy_wallet: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub x_username: String,
    #[serde(default)]
    pub vol: f64,
    #[serde(default)]
    pub pnl: f64,
    #[serde(default)]
    pub profile_image: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletTrade {
    pub proxy_wallet: String,
    pub side: String,
    pub asset: String,
    pub condition_id: String,
    pub size: f64,
    pub price: f64,
    pub timestamp: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub event_slug: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub outcome_index: i32,
    #[serde(default)]
    pub transaction_hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletPosition {
    pub proxy_wallet: String,
    pub asset: String,
    pub condition_id: String,
    pub size: f64,
    #[serde(default)]
    pub avg_price: f64,
    #[serde(default)]
    pub initial_value: f64,
    #[serde(default)]
    pub current_value: f64,
    #[serde(default)]
    pub cash_pnl: f64,
    #[serde(default)]
    pub percent_pnl: f64,
    #[serde(default)]
    pub cur_price: f64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub outcome_index: i32,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub redeemable: bool,
}

// ── Gamma API types ──

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaMarket {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub condition_id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub outcomes: Option<serde_json::Value>,
    #[serde(default)]
    pub outcome_prices: Option<serde_json::Value>,
    #[serde(default, rename = "clobTokenIds")]
    pub clob_token_ids: Option<serde_json::Value>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub closed: Option<bool>,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(default)]
    pub liquidity: Option<String>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub end_date_iso: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub enable_order_book: Option<bool>,
}

impl GammaMarket {
    pub fn is_active(&self) -> bool {
        self.active.unwrap_or(false) && !self.closed.unwrap_or(true)
    }

    pub fn is_resolved(&self) -> bool {
        self.resolved.unwrap_or(false)
    }

    pub fn winning_outcome(&self) -> Option<String> {
        if !self.is_resolved() {
            return None;
        }
        let prices = self.outcome_prices.as_ref()?;
        let outcomes = self.outcomes.as_ref()?;
        let prices_arr: Vec<String> = serde_json::from_value(prices.clone()).ok()?;
        let outcomes_arr: Vec<String> = serde_json::from_value(outcomes.clone()).ok()?;
        for (i, price_str) in prices_arr.iter().enumerate() {
            if let Ok(p) = price_str.parse::<f64>() {
                if p > 0.99 {
                    return outcomes_arr.get(i).cloned();
                }
            }
        }
        None
    }
}

// ── CLOB API types ──

#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookResponse {
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub bids: Option<Vec<OrderbookLevel>>,
    pub asks: Option<Vec<OrderbookLevel>>,
    pub hash: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookLevel {
    pub price: String,
    pub size: String,
}

impl OrderbookResponse {
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.as_ref()?
            .iter()
            .filter_map(|l| l.price.parse::<f64>().ok())
            .reduce(f64::min)
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids.as_ref()?
            .iter()
            .filter_map(|l| l.price.parse::<f64>().ok())
            .reduce(f64::max)
    }

    /// Total ask depth in USD up to max_price
    pub fn ask_depth_usd(&self, max_price: f64) -> f64 {
        self.asks.as_ref().map(|asks| {
            asks.iter()
                .filter_map(|l| {
                    let p = l.price.parse::<f64>().ok()?;
                    let s = l.size.parse::<f64>().ok()?;
                    if p <= max_price { Some(p * s) } else { None }
                })
                .sum()
        }).unwrap_or(0.0)
    }
}

// ── Database row types ──

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConfigEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TrackedWallet {
    pub id: i32,
    pub address: String,
    pub label: String,
    pub pnl: f64,
    pub volume: f64,
    pub enabled: bool,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SimulatedCopy {
    pub id: i32,
    pub whale_wallet: String,
    pub whale_tx_hash: String,
    pub market_slug: String,
    pub market_title: String,
    pub condition_id: String,
    pub asset_id: String,
    pub outcome: String,
    pub side: String,
    pub whale_price: f64,
    pub whale_size: f64,
    pub sim_entry_price: f64,
    pub sim_size_shares: f64,
    pub sim_cost_usdc: f64,
    pub detection_time: DateTime<Utc>,
    pub market_resolved: bool,
    pub winning_outcome: Option<String>,
    pub sim_pnl: Option<f64>,
    pub status: String,
    pub mode: String,
    pub created_at: DateTime<Utc>,
    // Analytics: latency + slippage
    #[serde(default)]
    pub signal_ts: i64,         // epoch ms: price signal detected
    #[serde(default)]
    pub orderbook_ts: i64,      // epoch ms: orderbook fetched
    #[serde(default)]
    pub order_sent_ts: i64,     // epoch ms: order sent to API
    #[serde(default)]
    pub order_filled_ts: i64,   // epoch ms: fill confirmed
    #[serde(default)]
    pub intended_price: f64,    // price we wanted
    #[serde(default)]
    pub fill_price: f64,        // price we actually got
    #[serde(default)]
    pub latency_total_ms: i64,  // signal → fill (ms)
    #[serde(default)]
    pub latency_exec_ms: i64,   // order sent → fill (ms)
    #[serde(default)]
    pub slippage_bps: f64,      // (fill - intended) / intended * 10000
    #[serde(default)]
    pub strategy: String,       // "oracle-lag", "copy", "hedge", "mm"
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BotStatus {
    pub id: i32,
    pub running: bool,
    pub mode: String,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeStats {
    pub total: i32,
    pub resolved: i32,
    pub open: i32,
    pub skipped: i32,
    pub wins: i32,
    pub losses: i32,
    pub total_pnl: f64,
    pub total_invested: f64,
    pub open_invested: f64,
    pub win_rate: f64,
    pub avg_slippage: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub roi: f64,
}

// ── Balance tracking ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceInfo {
    pub initial_capital: f64,
    pub current_balance: f64,
    pub total_pnl: f64,
    pub pnl_pct: f64,
    pub open_positions_value: f64,
    pub available_capital: f64,
}

// ── WebSocket event types ──

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum WsEvent {
    TradeDetected(SimulatedCopy),
    TradeResolved(SimulatedCopy),
    StatsUpdate(TradeStats),
    BotStatusChanged(BotStatus),
    BalanceUpdate(BalanceInfo),
}
