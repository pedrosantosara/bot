use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use std::collections::HashMap;
use crate::api::data::DataApi;
use crate::models::{BotStatus, ConfigEntry, SimulatedCopy, TradeStats, TrackedWallet, WsEvent};
use crate::web::state::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

fn err(status: StatusCode, msg: impl ToString) -> (StatusCode, String) {
    (status, msg.to_string())
}

// ── Config ──

pub async fn get_config(State(state): State<AppState>) -> ApiResult<Vec<ConfigEntry>> {
    state.db.get_all_config().await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
pub struct SetConfigBody {
    pub key: String,
    pub value: serde_json::Value,
}

pub async fn set_config(
    State(state): State<AppState>,
    Json(body): Json<SetConfigBody>,
) -> ApiResult<serde_json::Value> {
    state.db.set_config(&body.key, body.value.clone()).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── Wallets ──

pub async fn get_wallets(State(state): State<AppState>) -> ApiResult<Vec<TrackedWallet>> {
    state.db.get_wallets().await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
pub struct AddWalletBody {
    pub address: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub pnl: f64,
    #[serde(default)]
    pub volume: f64,
}

pub async fn add_wallet(
    State(state): State<AppState>,
    Json(body): Json<AddWalletBody>,
) -> ApiResult<TrackedWallet> {
    state.db.add_wallet(&body.address, &body.label, body.pnl, body.volume).await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
pub struct ToggleWalletBody {
    pub enabled: bool,
}

pub async fn toggle_wallet(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(body): Json<ToggleWalletBody>,
) -> ApiResult<serde_json::Value> {
    state.db.toggle_wallet(id, body.enabled).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

pub async fn delete_wallet(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> ApiResult<serde_json::Value> {
    state.db.delete_wallet(id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── Trades ──

#[derive(Deserialize)]
pub struct TradesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    pub mode: Option<String>,
}
fn default_limit() -> i64 { 50 }

pub async fn get_trades(
    State(state): State<AppState>,
    Query(q): Query<TradesQuery>,
) -> ApiResult<Vec<SimulatedCopy>> {
    state.db.get_trades(q.limit, q.offset, q.mode.as_deref()).await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub async fn get_trade_stats(
    State(state): State<AppState>,
    Query(q): Query<TradesQuery>,
) -> ApiResult<TradeStats> {
    state.db.get_trade_stats(q.mode.as_deref()).await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

// ── Bot Status ──

pub async fn get_status(State(state): State<AppState>) -> ApiResult<BotStatus> {
    state.db.get_bot_status().await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
pub struct SetModeBody {
    pub mode: String,
}

pub async fn set_mode(
    State(state): State<AppState>,
    Json(body): Json<SetModeBody>,
) -> ApiResult<serde_json::Value> {
    let current = state.db.get_bot_status().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.db.set_bot_status(current.running, &body.mode).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let updated = state.db.get_bot_status().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.broadcast(WsEvent::BotStatusChanged(updated));
    Ok(Json(serde_json::json!({"ok": true})))
}

pub async fn start_bot(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let current = state.db.get_bot_status().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.db.set_bot_status(true, &current.mode).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    *state.bot_running.write().await = true;
    let updated = state.db.get_bot_status().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.broadcast(WsEvent::BotStatusChanged(updated));
    Ok(Json(serde_json::json!({"ok": true, "status": "started"})))
}

pub async fn stop_bot(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let current = state.db.get_bot_status().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.db.set_bot_status(false, &current.mode).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    *state.bot_running.write().await = false;
    let updated = state.db.get_bot_status().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.broadcast(WsEvent::BotStatusChanged(updated));

    // Return final results
    let balance = state.db.get_balance_info().await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let stats = state.db.get_trade_stats(Some(&current.mode)).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "status": "stopped",
        "results": {
            "initial_capital": balance.initial_capital,
            "current_balance": balance.current_balance,
            "total_pnl": balance.total_pnl,
            "pnl_pct": balance.pnl_pct,
            "total_trades": stats.total,
            "wins": stats.wins,
            "losses": stats.losses,
            "win_rate": stats.win_rate,
            "roi": stats.roi,
        }
    })))
}

pub async fn get_balance(State(state): State<AppState>) -> ApiResult<crate::models::BalanceInfo> {
    state.db.get_balance_info().await
        .map(Json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

// ── Leaderboard (proxy) ──

#[derive(Deserialize)]
pub struct LeaderboardQuery {
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_period")]
    pub period: String,
    #[serde(default = "default_limit_u32")]
    pub limit: u32,
}
fn default_category() -> String { "CRYPTO".into() }
fn default_period() -> String { "WEEK".into() }
fn default_limit_u32() -> u32 { 20 }

pub async fn get_leaderboard(
    State(state): State<AppState>,
    Query(q): Query<LeaderboardQuery>,
) -> ApiResult<Vec<crate::models::LeaderboardEntry>> {
    let data_api = DataApi::new(state.http_client.clone(), "https://data-api.polymarket.com");
    data_api.get_leaderboard(&q.category, &q.period, "PNL", q.limit).await
        .map(Json)
        .map_err(|e| {
            warn!("Leaderboard fetch failed: {}", e);
            err(StatusCode::BAD_GATEWAY, "Failed to fetch leaderboard")
        })
}

// ── Analyze Wallet ──

#[derive(Serialize)]
pub struct WalletAnalysis {
    pub wallet: String,
    pub total_trades: usize,
    pub buy_count: usize,
    pub sell_count: usize,
    pub unique_markets: usize,
    pub total_volume: f64,
    pub avg_trade_size: f64,
    pub avg_price: f64,
    pub trades_per_day: f64,
    pub last_activity: Option<i64>,
    pub last_activity_ago: String,
    pub top_markets: Vec<MarketBreakdown>,
    pub recent_trades: Vec<serde_json::Value>,
    pub copyability_score: f64,
    pub copyability_reasons: Vec<String>,
}

#[derive(Serialize)]
pub struct MarketBreakdown {
    pub title: String,
    pub slug: String,
    pub trade_count: usize,
    pub volume: f64,
    pub avg_price: f64,
}

pub async fn analyze_wallet(
    State(state): State<AppState>,
    Path(wallet): Path<String>,
) -> ApiResult<WalletAnalysis> {
    let data_api = DataApi::new(state.http_client.clone(), "https://data-api.polymarket.com");

    let trades = data_api.get_trades(&wallet, 200).await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Failed to fetch trades: {}", e)))?;

    if trades.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "No trades found for this wallet"));
    }

    let total = trades.len();
    let buys: Vec<_> = trades.iter().filter(|t| t.side == "BUY").collect();
    let sells: Vec<_> = trades.iter().filter(|t| t.side == "SELL").collect();
    let buy_count = buys.len();
    let sell_count = sells.len();

    let total_volume: f64 = trades.iter().map(|t| t.size * t.price).sum();
    let avg_trade_size = if total > 0 { total_volume / total as f64 } else { 0.0 };
    let avg_price: f64 = if total > 0 {
        trades.iter().map(|t| t.price).sum::<f64>() / total as f64
    } else { 0.0 };

    // Unique markets
    let mut market_map: HashMap<String, (String, usize, f64, f64)> = HashMap::new();
    for t in &trades {
        let key = if !t.slug.is_empty() { t.slug.clone() } else { t.condition_id.clone() };
        let entry = market_map.entry(key).or_insert((t.title.clone(), 0, 0.0, 0.0));
        entry.1 += 1;
        entry.2 += t.size * t.price;
        entry.3 += t.price;
    }
    let unique_markets = market_map.len();

    let mut top_markets: Vec<MarketBreakdown> = market_map.into_iter()
        .map(|(slug, (title, count, vol, price_sum))| MarketBreakdown {
            title,
            slug,
            trade_count: count,
            volume: vol,
            avg_price: if count > 0 { price_sum / count as f64 } else { 0.0 },
        })
        .collect();
    top_markets.sort_by(|a, b| b.volume.partial_cmp(&a.volume).unwrap_or(std::cmp::Ordering::Equal));
    top_markets.truncate(10);

    // Trades per day
    let ts_first = trades.last().map(|t| t.timestamp).unwrap_or(0);
    let ts_last = trades.first().map(|t| t.timestamp).unwrap_or(0);
    let span_days = ((ts_last - ts_first) as f64 / 86400.0).max(1.0);
    let trades_per_day = total as f64 / span_days;

    // Last activity
    let last_activity = if ts_last > 0 { Some(ts_last) } else { None };
    let now_ts = chrono::Utc::now().timestamp();
    let last_activity_ago = if ts_last > 0 {
        let secs = now_ts - ts_last;
        if secs < 60 { format!("{}s ago", secs) }
        else if secs < 3600 { format!("{}m ago", secs / 60) }
        else if secs < 86400 { format!("{}h ago", secs / 3600) }
        else { format!("{}d ago", secs / 86400) }
    } else {
        "unknown".to_string()
    };

    // Copyability score (0-100)
    let mut score: f64 = 50.0;
    let mut reasons: Vec<String> = vec![];

    // Recent activity bonus
    if ts_last > 0 {
        let hours_ago = (now_ts - ts_last) as f64 / 3600.0;
        if hours_ago < 1.0 {
            score += 10.0;
            reasons.push(format!("Very active: last trade {}", last_activity_ago));
        } else if hours_ago < 24.0 {
            score += 5.0;
            reasons.push(format!("Active today: last trade {}", last_activity_ago));
        } else if hours_ago > 72.0 {
            score -= 10.0;
            reasons.push(format!("Inactive: last trade {}", last_activity_ago));
        }
    }

    // Prefer directional traders (more buys than sells)
    let buy_ratio = buy_count as f64 / total.max(1) as f64;
    if buy_ratio > 0.6 {
        score += 15.0;
        reasons.push(format!("Directional: {:.0}% buys", buy_ratio * 100.0));
    } else if buy_ratio < 0.4 {
        score -= 10.0;
        reasons.push(format!("Mostly selling ({:.0}% sells) - harder to copy", (1.0 - buy_ratio) * 100.0));
    }

    // Trade frequency: 5-50/day is ideal
    if trades_per_day >= 5.0 && trades_per_day <= 50.0 {
        score += 15.0;
        reasons.push(format!("Good frequency: {:.1} trades/day", trades_per_day));
    } else if trades_per_day > 200.0 {
        score -= 20.0;
        reasons.push(format!("Too fast: {:.0} trades/day (likely bot/MM)", trades_per_day));
    } else if trades_per_day < 1.0 {
        score -= 10.0;
        reasons.push(format!("Low activity: {:.1} trades/day", trades_per_day));
    }

    // Trade size: $50-$50K is copyable
    if avg_trade_size >= 50.0 && avg_trade_size <= 50000.0 {
        score += 10.0;
        reasons.push(format!("Copyable size: avg ${:.0}/trade", avg_trade_size));
    } else if avg_trade_size < 10.0 {
        score -= 15.0;
        reasons.push(format!("Dust trades: avg ${:.2}/trade", avg_trade_size));
    }

    // Market concentration: fewer markets = specialist
    if unique_markets <= 10 {
        score += 10.0;
        reasons.push(format!("Focused: {} markets (specialist)", unique_markets));
    } else if unique_markets > 50 {
        score -= 10.0;
        reasons.push(format!("Spread across {} markets (likely MM)", unique_markets));
    }

    score = score.clamp(0.0, 100.0);

    // Recent trades as raw JSON for display
    let recent_trades: Vec<serde_json::Value> = trades.iter().take(20)
        .map(|t| serde_json::json!({
            "side": t.side,
            "title": t.title,
            "outcome": t.outcome,
            "price": t.price,
            "size": t.size,
            "value": t.size * t.price,
            "timestamp": t.timestamp,
            "slug": t.slug,
        }))
        .collect();

    Ok(Json(WalletAnalysis {
        wallet,
        total_trades: total,
        buy_count,
        sell_count,
        unique_markets,
        total_volume,
        avg_trade_size,
        avg_price,
        trades_per_day,
        last_activity,
        last_activity_ago,
        top_markets,
        recent_trades,
        copyability_score: score,
        copyability_reasons: reasons,
    }))
}

// ── Crypto Short-Term Markets (BTC/ETH/SOL 5min, 15min) ──

#[derive(Deserialize)]
pub struct CryptoMarketsQuery {
    #[serde(default = "default_crypto_query")]
    pub q: String,
}
fn default_crypto_query() -> String { "bitcoin up or down".into() }

pub async fn get_btc_markets(
    State(state): State<AppState>,
    Query(q): Query<CryptoMarketsQuery>,
) -> ApiResult<Vec<serde_json::Value>> {
    // Use public-search endpoint which finds the short-term crypto markets
    let url = format!(
        "https://gamma-api.polymarket.com/public-search?q={}&limit=20",
        urlencoding::encode(&q.q)
    );

    let resp = state.http_client.get(&url).send().await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Search failed: {}", e)))?;

    let body: serde_json::Value = resp.json().await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Parse failed: {}", e)))?;

    let events = body.get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut results: Vec<serde_json::Value> = Vec::new();

    for event in &events {
        let event_closed = event.get("closed").and_then(|v| v.as_bool()).unwrap_or(true);

        let markets = event.get("markets")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for m in &markets {
            let slug = m.get("slug").and_then(|v| v.as_str()).unwrap_or("");
            let market_closed = m.get("closed").and_then(|v| v.as_bool()).unwrap_or(true);

            let outcomes: Vec<String> = m.get("outcomes")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let prices: Vec<String> = m.get("outcomePrices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            results.push(serde_json::json!({
                "id": m.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "question": m.get("question").and_then(|v| v.as_str()).unwrap_or(""),
                "slug": slug,
                "outcomes": outcomes,
                "prices": prices,
                "volume": event.get("volume").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "liquidity": event.get("liquidity").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "end_date": m.get("endDate").and_then(|v| v.as_str()).unwrap_or(""),
                "active": !event_closed && !market_closed,
                "closed": market_closed,
            }));
        }
    }

    Ok(Json(results))
}
