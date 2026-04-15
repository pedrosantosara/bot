use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Instant;

use anyhow::Result;
use chrono::Utc;
use colored::Colorize;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};
use tracing::{debug, error, info, warn};

use crate::models::{SimulatedCopy, WalletTrade, WsEvent};
use crate::web::state::AppState;

// ── Helpers for RTDS fields that may be number OR string ──

fn json_as_f64(val: &serde_json::Value) -> Option<f64> {
    val.as_f64().or_else(|| val.as_str().and_then(|s| s.parse::<f64>().ok()))
}

fn json_as_i64(val: &serde_json::Value) -> Option<i64> {
    val.as_i64().or_else(|| val.as_str().and_then(|s| s.parse::<i64>().ok()))
}

enum TrackedPosition {
    StillHolding,  // Whale still has the position
    Exited,        // Whale sold / no position found and market not ended
    Resolved,      // Position is redeemable (market resolved)
    Unknown,       // API error or position not found after market ended
}

pub struct Simulator {
    state: AppState,
    clob_api: crate::api::clob::ClobApi,
    gamma_api: crate::api::gamma::GammaApi,
    /// Real order executor (only in "live" mode)
    executor: Option<Arc<crate::execution::Executor>>,
    seen_tx_hashes: Arc<Mutex<HashSet<String>>>,
    /// slug → outcome ("Up"/"Down") of open positions
    open_market_slugs: Arc<Mutex<HashMap<String, String>>>,
    condition_slug_cache: Arc<Mutex<HashMap<String, String>>>,
    subscribed_assets: Arc<Mutex<HashSet<String>>>,
    /// Circuit breaker
    consecutive_losses: Arc<Mutex<u32>>,
    daily_pnl: Arc<Mutex<f64>>,
    /// Rate limiter: timestamps of recent API calls
    api_timestamps: Arc<Mutex<VecDeque<Instant>>>,
    started_at: i64,
}

impl Simulator {
    pub fn new(state: AppState) -> Self {
        let clob_api = crate::api::clob::ClobApi::new(state.http_client.clone(), "https://clob.polymarket.com");
        let gamma_api = crate::api::gamma::GammaApi::new(state.http_client.clone(), "https://gamma-api.polymarket.com");

        Self {
            state,
            clob_api,
            gamma_api,
            executor: None,
            seen_tx_hashes: Arc::new(Mutex::new(HashSet::new())),
            open_market_slugs: Arc::new(Mutex::new(HashMap::new())),
            condition_slug_cache: Arc::new(Mutex::new(HashMap::new())),
            subscribed_assets: Arc::new(Mutex::new(HashSet::new())),
            consecutive_losses: Arc::new(Mutex::new(0)),
            daily_pnl: Arc::new(Mutex::new(0.0)),
            api_timestamps: Arc::new(Mutex::new(VecDeque::new())),
            started_at: Utc::now().timestamp(),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        info!(started_at = self.started_at, "Simulator starting — only trades AFTER this timestamp will be copied");

        // Initialize real executor if in live mode
        let mode = self.state.db.get_bot_status().await
            .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());
        if mode == "live" {
            match crate::execution::Executor::new().await {
                Ok(exec) => {
                    self.executor = Some(Arc::new(exec));
                    println!("  {} {}", "🔑", "Polymarket CLOB autenticado — modo LIVE".green().bold());
                }
                Err(e) => {
                    println!("  {} {}", "❌", format!("Falha na autenticação CLOB: {:?}", e).red());
                    println!("  {} {}", "🛑", "Bot parado — corrija a autenticação e tente novamente".red().bold());
                    *self.state.bot_running.write().await = false;
                    self.state.db.set_bot_status(false, "live").await.ok();
                    return Ok(());
                }
            }
        }

        // Load open market keys + asset IDs from DB
        if let Ok(open_copies) = self.state.db.get_open_copies().await {
            // First: sync with tracked wallets — close positions where the whale already exited
            if !open_copies.is_empty() {
                info!(count = open_copies.len(), "Checking open positions against tracked wallets on startup...");
                self.sync_open_positions_on_startup(&open_copies).await;
            }

            // Reload after sync (some may have been closed)
            let remaining = self.state.db.get_open_copies().await.unwrap_or_default();
            let mut open_mkts = self.open_market_slugs.lock().await;
            let mut assets = self.subscribed_assets.lock().await;
            for copy in &remaining {
                let key = self.market_key_from_copy(copy);
                if !key.is_empty() {
                    open_mkts.insert(key, copy.outcome.clone());
                }
                if !copy.asset_id.is_empty() {
                    assets.insert(copy.asset_id.clone());
                }
            }
            info!(open_markets = open_mkts.len(), assets = assets.len(), "Loaded state from DB");
        }

        // Read strategy
        let strategy = self.state.db.get_config("strategy").await
            .ok().flatten()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "copy".to_string());

        let resolve_handle = self.run_resolution_loop();
        let clob_handle = self.run_clob_resolution_listener();
        let balance_handle = self.run_balance_tracker();

        info!(strategy = %strategy, "Simulator running");

        match strategy.as_str() {
            "oracle" => {
                let oracle_handle = self.run_oracle_lag_loop();
                tokio::select! {
                    r = oracle_handle => { if let Err(e) = r { error!(error = %e, "Oracle lag error"); } }
                    r = clob_handle => { if let Err(e) = r { error!(error = %e, "CLOB error"); } }
                    r = resolve_handle => { if let Err(e) = r { error!(error = %e, "Resolution error"); } }
                    r = balance_handle => { if let Err(e) = r { error!(error = %e, "Balance tracker error"); } }
                }
            }
            "hedge" => {
                let hedge_handle = self.run_hedge_loop();
                tokio::select! {
                    r = hedge_handle => { if let Err(e) = r { error!(error = %e, "Hedge error"); } }
                    r = clob_handle => { if let Err(e) = r { error!(error = %e, "CLOB error"); } }
                    r = resolve_handle => { if let Err(e) = r { error!(error = %e, "Resolution error"); } }
                    r = balance_handle => { if let Err(e) = r { error!(error = %e, "Balance tracker error"); } }
                }
            }
            "mm" => {
                let mm_handle = self.run_market_making_loop();
                tokio::select! {
                    r = mm_handle => { if let Err(e) = r { error!(error = %e, "Market Making error"); } }
                    r = clob_handle => { if let Err(e) = r { error!(error = %e, "CLOB error"); } }
                    r = resolve_handle => { if let Err(e) = r { error!(error = %e, "Resolution error"); } }
                    r = balance_handle => { if let Err(e) = r { error!(error = %e, "Balance tracker error"); } }
                }
            }
            _ => {
                // "copy" (default) — requires wallets
                let wallets = self.state.db.get_enabled_wallets().await?;
                if wallets.is_empty() {
                    warn!("No enabled wallets to monitor. Add wallets via the UI.");
                    return Ok(());
                }
                let wallet_set: HashSet<String> = wallets.iter()
                    .map(|w| w.address.to_lowercase())
                    .collect();
                info!(count = wallet_set.len(), "Monitoring wallets via RTDS WebSocket");

                // Load seen tx hashes
                if let Ok(existing) = self.state.db.get_trades(2000, 0, None).await {
                    let mut seen = self.seen_tx_hashes.lock().await;
                    for copy in &existing {
                        seen.insert(copy.whale_tx_hash.clone());
                    }
                    info!(seen = seen.len(), "Loaded seen tx hashes from DB");
                }

                let ws_handle = self.run_ws_listener(&wallet_set);
                tokio::select! {
                    r = ws_handle => { if let Err(e) = r { error!(error = %e, "RTDS error"); } }
                    r = clob_handle => { if let Err(e) = r { error!(error = %e, "CLOB error"); } }
                    r = resolve_handle => { if let Err(e) = r { error!(error = %e, "Resolution error"); } }
                    r = balance_handle => { if let Err(e) = r { error!(error = %e, "Balance tracker error"); } }
                }
            }
        }

        Ok(())
    }

    /// Derive a market key from a SimulatedCopy (slug preferred, condition_id fallback)
    fn market_key_from_copy(&self, copy: &SimulatedCopy) -> String {
        if !copy.market_slug.is_empty() {
            copy.market_slug.clone()
        } else if !copy.condition_id.is_empty() {
            copy.condition_id.clone()
        } else {
            String::new()
        }
    }

    /// Resolve slug for a trade: use RTDS slug, or look up via Gamma API, with cache
    async fn resolve_market_slug(&self, trade: &WalletTrade) -> String {
        // If RTDS provided slug, use it
        if !trade.slug.is_empty() {
            return trade.slug.clone();
        }

        // Check cache
        if !trade.condition_id.is_empty() {
            let cache = self.condition_slug_cache.lock().await;
            if let Some(slug) = cache.get(&trade.condition_id) {
                return slug.clone();
            }
        }

        // Look up via Gamma API
        if !trade.condition_id.is_empty() {
            if let Ok(Some(market)) = self.gamma_api.get_market_by_condition(&trade.condition_id).await {
                if !market.slug.is_empty() {
                    self.condition_slug_cache.lock().await
                        .insert(trade.condition_id.clone(), market.slug.clone());
                    return market.slug;
                }
            }
        }

        // Last resort: condition_id as key (per-outcome, not ideal but better than nothing)
        trade.condition_id.clone()
    }

    // ── Oracle Lag Loop: exploit price feed delay ──

    async fn run_oracle_lag_loop(&self) -> Result<()> {
        // Markets ordered by profitability: ETH > BTC > SOL
        // XRP removido: 50% win rate, -$3.70
        let markets = vec![
            ("eth-updown-5m", "ethusdt"),
            ("btc-updown-5m", "btcusdt"),
            ("sol-updown-5m", "solusdt"),
        ];

        // State: opening prices + price history for trend detection
        let opening_prices: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        let latest_prices: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        // Price history: symbol → VecDeque of (timestamp, price) for trend calculation
        let price_history: Arc<Mutex<HashMap<String, VecDeque<(i64, f64)>>>> = Arc::new(Mutex::new(HashMap::new()));

        // Connect to RTDS for crypto prices
        let prices_clone = latest_prices.clone();
        let open_clone = opening_prices.clone();
        let history_clone = price_history.clone();
        let bot_running = self.state.bot_running.clone();

        let price_ws_handle = tokio::spawn(async move {
            loop {
                if !*bot_running.read().await { return; }

                let ws_url = "wss://ws-live-data.polymarket.com";
                let conn = connect_async(ws_url).await;
                let (mut ws, _) = match conn {
                    Ok(c) => c,
                    Err(_) => {
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        continue;
                    }
                };

                // Subscribe to crypto prices (no filter — filter client-side)
                let sub = serde_json::json!({
                    "action": "subscribe",
                    "subscriptions": [{
                        "topic": "crypto_prices",
                        "type": "update",
                        "filters": ""
                    }]
                });
                let _ = ws.send(WsMsg::Text(sub.to_string())).await;

                println!("  {} {}", "📡", "Oracle price feed connected".dimmed());

                let mut ping_timer = tokio::time::interval(tokio::time::Duration::from_secs(5));
                loop {
                    tokio::select! {
                        msg = ws.next() => {
                            match msg {
                                Some(Ok(WsMsg::Text(text))) => {
                                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) {
                                        if let Some(payload) = msg.get("payload") {
                                            let symbol = payload.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                                            let value = payload.get("value").and_then(json_as_f64).unwrap_or(0.0);

                                            if value > 0.0 && !symbol.is_empty() {
                                                prices_clone.lock().await.insert(symbol.to_string(), value);

                                                // Record price history for trend detection
                                                let ts_ms = payload.get("timestamp").and_then(json_as_i64).unwrap_or(0);
                                                let ts_s = if ts_ms > 1_000_000_000_000 { ts_ms / 1000 } else { ts_ms };
                                                {
                                                    let mut hist = history_clone.lock().await;
                                                    let h = hist.entry(symbol.to_string()).or_insert_with(VecDeque::new);
                                                    h.push_back((ts_s, value));
                                                    // Keep last 120 seconds of data
                                                    while h.len() > 300 { h.pop_front(); }
                                                }

                                                let now = Utc::now().timestamp();
                                                let window_start = now - (now % 300);
                                                let key = format!("{}-{}", symbol, window_start);
                                                let mut opens = open_clone.lock().await;
                                                opens.entry(key).or_insert(value);
                                            }
                                        }
                                    }
                                }
                                Some(Ok(WsMsg::Close(_))) | None => break,
                                Some(Err(_)) => break,
                                _ => {}
                            }
                        }
                        _ = ping_timer.tick() => {
                            if ws.send(WsMsg::Ping(vec![])).await.is_err() { break; }
                        }
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });

        // Wait for first prices to arrive
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        info!("Oracle lag strategy started — entries only in last 60s of each 5min window");

        let mut token_cache: HashMap<String, (String, String, String, String)> = HashMap::new();
        let mut traded_slugs: HashSet<String> = HashSet::new();
        let mut last_report = tokio::time::Instant::now();

        loop {
            if !*self.state.bot_running.read().await {
                price_ws_handle.abort();
                return Ok(());
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let now = Utc::now().timestamp();
            let window_start = now - (now % 300);
            let time_remaining = window_start + 300 - now;

            // Config
            let min_move_pct = self.get_config_f64("oracle_min_move_pct").await.unwrap_or(0.07);
            let max_token_price = self.get_config_f64("oracle_max_token_price").await.unwrap_or(0.70);
            let trade_amount = self.get_config_f64("max_per_trade").await.unwrap_or(1.5);
            let entry_window = self.get_config_f64("oracle_entry_window_secs").await.unwrap_or(270.0) as i64;

            // Trade from 30s after window opens until 10s before close (default 270s window)
            // Data shows entries at 0-60s had 72.5% win rate — early entries work when trend is correct
            if time_remaining > entry_window || time_remaining < 10 {
                if time_remaining > 295 {
                    traded_slugs.clear();
                    // Clean old opening prices
                    let mut opens = opening_prices.lock().await;
                    opens.retain(|k, _| {
                        k.split('-').last()
                            .and_then(|ts| ts.parse::<i64>().ok())
                            .map(|ts| now - ts < 600)
                            .unwrap_or(false)
                    });
                }
                continue;
            }

            // Circuit breaker
            let max_losses = self.get_config_f64("max_consecutive_losses").await.unwrap_or(5.0) as u32;
            if *self.consecutive_losses.lock().await >= max_losses { continue; }
            let daily_limit = self.get_config_f64("daily_loss_limit").await.unwrap_or(10.0);
            if *self.daily_pnl.lock().await < -daily_limit { continue; }

            // Max open positions
            let max_open = self.get_config_f64("max_open_positions").await.unwrap_or(10.0) as usize;
            if self.open_market_slugs.lock().await.len() >= max_open { continue; }

            // Count trades this window — data shows 3+ simultaneous = negative EV
            let max_per_window = 2;
            let trades_this_window = traded_slugs.iter()
                .filter(|s| s.ends_with(&window_start.to_string()))
                .count();
            if trades_this_window >= max_per_window { continue; }

            // ── PRE-SCAN: check all coins' direction BEFORE entering any trade ──
            // Data: same direction = +$5.90, mixed = -$6.09
            // Only trade if coins agree on direction
            let mut directions: Vec<(&str, f64)> = Vec::new();
            for (prefix, symbol) in &markets {
                let cp = latest_prices.lock().await.get(*symbol).copied().unwrap_or(0.0);
                let op_key = format!("{}-{}", symbol, window_start);
                let op = opening_prices.lock().await.get(&op_key).copied().unwrap_or(0.0);
                if cp > 0.0 && op > 0.0 {
                    let mv = ((cp - op) / op) * 100.0;
                    if mv.abs() >= min_move_pct {
                        directions.push((prefix, mv));
                    }
                }
            }
            // If we have signals in BOTH directions, skip this window entirely
            let has_up = directions.iter().any(|(_, m)| *m > 0.0);
            let has_down = directions.iter().any(|(_, m)| *m < 0.0);
            if has_up && has_down && directions.len() >= 2 {
                // Mixed signals — skip
                continue;
            }

            // Time-in-window: seconds elapsed since window start
            let elapsed_secs = 300 - time_remaining;

            for (prefix, symbol) in &markets {
                let slug = format!("{}-{}", prefix, window_start);
                if traded_slugs.contains(&slug) { continue; }

                // Recheck window limit inside loop
                let current_window_trades = traded_slugs.iter()
                    .filter(|s| s.ends_with(&window_start.to_string()))
                    .count();
                if current_window_trades >= max_per_window { break; }

                let coin = prefix.split('-').next().unwrap_or("?").to_uppercase();

                // ── PER-COIN TIMING FILTER ──
                // Data: SOL at 30-50s = 65% WR (-$5.58), BTC at 50-100s = 50% WR (-$5.14)
                // ETH is good at all times. Delay SOL and BTC entries.
                match coin.as_str() {
                    "SOL" if elapsed_secs < 60 => continue,  // SOL: wait 60s
                    "BTC" if elapsed_secs < 50 => continue,  // BTC: wait 50s
                    _ => {} // ETH: enter anytime
                }

                // Get prices
                let current_price = {
                    let prices = latest_prices.lock().await;
                    prices.get(*symbol).copied().unwrap_or(0.0)
                };
                let open_price = {
                    let key = format!("{}-{}", symbol, window_start);
                    let opens = opening_prices.lock().await;
                    opens.get(&key).copied().unwrap_or(0.0)
                };

                let coin = prefix.split('-').next().unwrap_or("?").to_uppercase();

                // Periodic report (show even if prices are 0 for debugging)
                if last_report.elapsed().as_secs() >= 30 {
                    if current_price == 0.0 || open_price == 0.0 {
                        println!("  {} {} price={} open={} (waiting for data) [{}s]",
                            "⏳".dimmed(), coin.dimmed(),
                            if current_price == 0.0 { "none" } else { "ok" },
                            if open_price == 0.0 { "none" } else { "ok" },
                            time_remaining,
                        );
                    } else {
                        let move_pct = ((current_price - open_price) / open_price) * 100.0;
                        println!("  {} {} open=${:.2} now=${:.2} move={}{:.3}% [{}s]",
                            "📈".dimmed(), coin.dimmed(),
                            open_price, current_price,
                            if move_pct >= 0.0 { "+" } else { "" },
                            move_pct, time_remaining,
                        );
                    }
                    if *symbol == "xrpusdt" { last_report = tokio::time::Instant::now(); }
                }

                if current_price == 0.0 || open_price == 0.0 { continue; }

                let move_pct = ((current_price - open_price) / open_price) * 100.0;
                let abs_move = move_pct.abs();

                // Check: price moved enough?
                if abs_move < min_move_pct { continue; }

                // ⏱️ SIGNAL DETECTED — start timing
                let signal_ts = Utc::now().timestamp_millis();

                // Trend filter: check BOTH 30s and 120s momentum
                let trend_confirmed = {
                    let hist = price_history.lock().await;
                    if let Some(h) = hist.get(*symbol) {
                        let now_ts = Utc::now().timestamp();
                        let price_30s = h.iter().rev()
                            .find(|(ts, _)| now_ts - ts >= 30)
                            .map(|(_, p)| *p);
                        let price_120s = h.iter().rev()
                            .find(|(ts, _)| now_ts - ts >= 120)
                            .map(|(_, p)| *p);

                        let short_ok = match price_30s {
                            Some(p30) => {
                                if move_pct > 0.0 { current_price > p30 } else { current_price < p30 }
                            }
                            _ => true
                        };
                        let long_ok = match price_120s {
                            Some(p120) => {
                                if move_pct > 0.0 { current_price > p120 } else { current_price < p120 }
                            }
                            _ => true
                        };
                        short_ok && long_ok
                    } else {
                        true
                    }
                };

                if !trend_confirmed {
                    println!("  {} {} move={}{:.3}% mas trend NÃO confirmado [{}s]",
                        "⏭️".dimmed(), coin.dimmed(),
                        if move_pct >= 0.0 { "+" } else { "" }, move_pct, time_remaining);
                    continue;
                }

                // Determine direction
                let target_outcome = if move_pct > 0.0 { "Up" } else { "Down" };

                // Get market tokens
                let tokens = if let Some(t) = token_cache.get(&slug) {
                    t.clone()
                } else {
                    match self.fetch_market_tokens(&slug).await {
                        Some(t) => { token_cache.insert(slug.clone(), t.clone()); t }
                        None => continue,
                    }
                };

                let (token_0, token_1, outcome_0, outcome_1) = tokens;

                let (target_token, opposite_token, target_outcome_name) =
                    if outcome_0.eq_ignore_ascii_case(target_outcome) {
                        (token_0.clone(), token_1.clone(), outcome_0.clone())
                    } else {
                        (token_1.clone(), token_0.clone(), outcome_1.clone())
                    };

                // ⏱️ ORDERBOOK FETCH — measure API latency
                let pre_book_ts = Utc::now().timestamp_millis();
                let book_target = match self.clob_api.get_orderbook(&target_token).await {
                    Ok(b) => b, Err(_) => continue,
                };
                let book_opposite = match self.clob_api.get_orderbook(&opposite_token).await {
                    Ok(b) => b, Err(_) => continue,
                };
                let orderbook_ts = Utc::now().timestamp_millis();

                let target_ask = book_target.best_ask().unwrap_or(1.0);
                let _opposite_ask = book_opposite.best_ask().unwrap_or(1.0);
                let intended_price = target_ask; // the price we WANT to enter at

                // ── PRICE SWEET SPOT FILTER ──
                // Data: 0.67-0.70 = 77% WR (EV +$0.11), <0.66 = 63% WR (EV -$0.23)
                // Only enter when market confirms our signal (token priced 0.66-0.70)
                if target_ask > max_token_price {
                    println!("  {} {} {}@{:.2} > max {:.2} [{}s]",
                        "⏭️".dimmed(), coin.dimmed(),
                        target_outcome_name, target_ask, max_token_price, time_remaining);
                    continue;
                }
                if target_ask < 0.66 {
                    println!("  {} {} @{:.2} < 0.66 weak signal [{}s]",
                        "⏭️".dimmed(), coin.dimmed(), target_ask, time_remaining);
                    continue;
                }

                let depth = book_target.ask_depth_usd(target_ask + 0.05);
                if depth < 50.0 {
                    println!("  {} {} depth ${:.0} < $50 [{}s]",
                        "⏭️".dimmed(), coin.dimmed(), depth, time_remaining);
                    continue;
                }

                let balance = match self.state.db.get_balance_info().await {
                    Ok(b) => b, Err(_) => continue,
                };
                let mut shares = trade_amount / target_ask;
                // Polymarket minimum: 5 shares per order
                if shares < 5.0 { shares = 5.0; }
                let actual_cost = shares * target_ask;
                if balance.available_capital < actual_cost { continue; }
                let potential_profit = shares - actual_cost;

                println!("{} {} {} move={}{:.3}% trend=✓ → {} @{:.2} ${:.2} ({:.0}sh) +${:.2} [{}s] book={}ms",
                    "⚡ ORACLE".cyan().bold(),
                    coin.yellow().bold(),
                    format!("${:.2}→${:.2}", open_price, current_price).dimmed(),
                    if move_pct >= 0.0 { "+" } else { "" }, move_pct,
                    target_outcome_name.white().bold(),
                    target_ask,
                    actual_cost, shares,
                    potential_profit,
                    time_remaining,
                    orderbook_ts - pre_book_ts,
                );

                // Mark as traded BEFORE execution to prevent repeated attempts
                traded_slugs.insert(slug.clone());

                // ⏱️ ORDER SENT — measure execution latency
                let order_sent_ts = Utc::now().timestamp_millis();
                let mut fill_price = target_ask; // default for test mode
                let mut order_filled_ts = order_sent_ts;

                if let Some(ref exec) = self.executor {
                    match exec.buy_limit(&target_token, shares, target_ask).await {
                        Ok(r) if r.success => {
                            order_filled_ts = r.filled_ts;
                            if r.fill_price > 0.0 { fill_price = r.fill_price; }
                        }
                        Ok(_) => { warn!("Oracle limit order not filled"); continue; }
                        Err(e) => { warn!(error = %e, "Oracle order error"); continue; }
                    }
                }

                // Calculate analytics
                let latency_total_ms = order_filled_ts - signal_ts;
                let latency_exec_ms = order_filled_ts - order_sent_ts;
                let slippage_bps = if intended_price > 0.0 {
                    ((fill_price - intended_price) / intended_price) * 10000.0
                } else { 0.0 };

                // Record in DB with full analytics
                let mode = self.state.db.get_bot_status().await
                    .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());

                let copy = SimulatedCopy {
                    id: 0,
                    whale_wallet: "oracle-lag".to_string(),
                    whale_tx_hash: format!("oracle-{}-{}", slug, target_outcome_name),
                    market_slug: slug.clone(),
                    market_title: format!("{} 5m oracle", coin),
                    condition_id: String::new(),
                    asset_id: target_token,
                    outcome: target_outcome_name.clone(),
                    side: "BUY".to_string(),
                    whale_price: target_ask,
                    whale_size: shares,
                    sim_entry_price: fill_price,
                    sim_size_shares: shares,
                    sim_cost_usdc: actual_cost,
                    detection_time: Utc::now(),
                    market_resolved: false,
                    winning_outcome: None,
                    sim_pnl: None,
                    status: "OPEN".to_string(),
                    mode,
                    created_at: Utc::now(),
                    // Analytics
                    signal_ts,
                    orderbook_ts,
                    order_sent_ts,
                    order_filled_ts,
                    intended_price,
                    fill_price,
                    latency_total_ms,
                    latency_exec_ms,
                    slippage_bps,
                    strategy: "oracle-lag".to_string(),
                };

                let id = self.state.db.insert_copy(&copy).await.unwrap_or(0);
                self.open_market_slugs.lock().await.insert(slug.clone(), target_outcome_name);
                self.subscribed_assets.lock().await.insert(copy.asset_id.clone());

                println!("  {} #{} {} {}",
                    "✅".green(), id,
                    format!("[bal ${:.0}]", balance.available_capital - trade_amount).dimmed(),
                    format!("⏱️ total={}ms exec={}ms slip={:.1}bps", latency_total_ms, latency_exec_ms, slippage_bps).dimmed(),
                );

                self.state.broadcast(WsEvent::TradeDetected(copy));
                if let Ok(stats) = self.state.db.get_trade_stats(None).await {
                    self.state.broadcast(WsEvent::StatsUpdate(stats));
                }
                if let Ok(bal) = self.state.db.get_balance_info().await {
                    self.state.broadcast(WsEvent::BalanceUpdate(bal));
                }
            }
        }
    }

    // ── Hedge Loop: Buy Up+Down when sum < threshold ──

    async fn run_hedge_loop(&self) -> Result<()> {
        let market_prefixes = vec![
            "btc-updown-5m", "eth-updown-5m", "sol-updown-5m",
            "xrp-updown-5m", "doge-updown-5m",
        ];

        let mut token_cache: HashMap<String, (String, String, String, String)> = HashMap::new();
        let mut hedged_slugs: HashSet<String> = HashSet::new();
        let mut last_report = tokio::time::Instant::now();

        info!("Hedge loop started — monitoring {} markets for arbitrage", market_prefixes.len());

        loop {
            if !*self.state.bot_running.read().await {
                return Ok(());
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            let now = Utc::now().timestamp();
            let market_start = now - (now % 300);

            // Clean hedged slugs at new window
            let time_remaining = market_start + 300 - now;
            if time_remaining > 290 {
                hedged_slugs.clear();
            }

            // Don't hedge in last 10 seconds (risk of resolution during execution)
            if time_remaining < 10 {
                continue;
            }

            let threshold = self.get_config_f64("hedge_threshold").await.unwrap_or(0.95);
            let amount_per_leg = self.get_config_f64("hedge_amount_per_leg").await.unwrap_or(2.0);
            let max_hedges = self.get_config_f64("max_hedges_open").await.unwrap_or(3.0) as usize;

            // Check max open hedges
            let open_count = self.open_market_slugs.lock().await.len();
            if open_count >= max_hedges * 2 {
                continue; // Each hedge = 2 positions
            }

            for prefix in &market_prefixes {
                let slug = format!("{}-{}", prefix, market_start);

                if hedged_slugs.contains(&slug) {
                    continue;
                }

                // Get token IDs
                let tokens = if let Some(t) = token_cache.get(&slug) {
                    t.clone()
                } else {
                    match self.fetch_market_tokens(&slug).await {
                        Some(t) => { token_cache.insert(slug.clone(), t.clone()); t }
                        None => continue,
                    }
                };

                let (token_0, token_1, outcome_0, outcome_1) = tokens;

                // Get orderbooks
                let book_0 = match self.clob_api.get_orderbook(&token_0).await {
                    Ok(b) => b, Err(_) => continue,
                };
                let book_1 = match self.clob_api.get_orderbook(&token_1).await {
                    Ok(b) => b, Err(_) => continue,
                };

                let ask_0 = book_0.best_ask().unwrap_or(1.0);
                let ask_1 = book_1.best_ask().unwrap_or(1.0);
                let sum = ask_0 + ask_1;

                // Check depth ($50 minimum per side)
                let depth_0 = book_0.ask_depth_usd(ask_0 + 0.05);
                let depth_1 = book_1.ask_depth_usd(ask_1 + 0.05);

                let coin = prefix.split('-').next().unwrap_or("?").to_uppercase();

                // Periodic report every 30s
                if last_report.elapsed().as_secs() >= 30 {
                    println!("  {} {} {}={:.3}+{}={:.3}={:.3} {} depth ${:.0}/${:.0} [{}s]",
                        "📊".dimmed(),
                        coin.dimmed(),
                        outcome_0.dimmed(), ask_0,
                        outcome_1.dimmed(), ask_1,
                        sum,
                        if sum < threshold { "✅".to_string() } else { format!("need<{:.2}", threshold) }.dimmed(),
                        depth_0, depth_1,
                        time_remaining,
                    );
                    if coin == "DOGE" { last_report = tokio::time::Instant::now(); }
                }

                if sum < threshold && depth_0 >= 50.0 && depth_1 >= 50.0 {
                    // ARBITRAGE OPPORTUNITY
                    let shares = amount_per_leg / sum;
                    let cost_0 = shares * ask_0;
                    let cost_1 = shares * ask_1;
                    let total_cost = cost_0 + cost_1;
                    let payout = shares * 0.98; // $1/share - 2% fee
                    let profit = payout - total_cost;

                    if profit <= 0.0 {
                        continue; // Not profitable after fees
                    }

                    println!("{} {} {}={:.3} + {}={:.3} = {:.3} → profit ${:.2} ({:.1}%)",
                        "🔄 HEDGE".magenta().bold(),
                        coin.cyan(),
                        outcome_0, ask_0,
                        outcome_1, ask_1,
                        sum,
                        profit,
                        (profit / total_cost) * 100.0,
                    );

                    // Circuit breaker
                    let daily = *self.daily_pnl.lock().await;
                    let daily_limit = self.get_config_f64("daily_loss_limit").await.unwrap_or(10.0);
                    if daily < -daily_limit {
                        println!("  {} {}", "🛑", "Daily loss limit hit".red());
                        continue;
                    }

                    // Capital check
                    let balance = match self.state.db.get_balance_info().await {
                        Ok(b) => b, Err(_) => continue,
                    };
                    if balance.available_capital < total_cost {
                        continue;
                    }

                    // Execute leg 1
                    let leg1_ok = if let Some(ref exec) = self.executor {
                        match exec.buy_limit(&token_0, shares, ask_0).await {
                            Ok(r) => r.success,
                            Err(_) => false,
                        }
                    } else {
                        true // test mode = always "succeeds"
                    };

                    if !leg1_ok {
                        println!("  {} {}", "❌", "Leg 1 failed — aborting hedge".red());
                        continue;
                    }

                    // Execute leg 2
                    let leg2_ok = if let Some(ref exec) = self.executor {
                        match exec.buy_limit(&token_1, shares, ask_1).await {
                            Ok(r) => r.success,
                            Err(_) => false,
                        }
                    } else {
                        true
                    };

                    if !leg2_ok {
                        println!("  {} {}", "⚠️", "Leg 2 failed — directional exposure! Selling leg 1".yellow());
                        // Try to sell leg 1 to cut exposure
                        if let Some(ref exec) = self.executor {
                            let _ = exec.sell(&token_0, shares, ask_0 * 0.95).await;
                        }
                        continue;
                    }

                    // Both legs filled — record in DB
                    let mode = self.state.db.get_bot_status().await
                        .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());

                    // Record leg 1 (outcome_0)
                    let copy_0 = SimulatedCopy {
                        id: 0,
                        whale_wallet: "hedge-bot".to_string(),
                        whale_tx_hash: format!("hedge-{}-{}", slug, outcome_0),
                        market_slug: slug.clone(),
                        market_title: format!("{} 5m hedge", coin),
                        condition_id: String::new(),
                        asset_id: token_0.clone(),
                        outcome: outcome_0.clone(),
                        side: "BUY".to_string(),
                        whale_price: ask_0,
                        whale_size: shares,
                        sim_entry_price: ask_0,
                        sim_size_shares: shares,
                        sim_cost_usdc: cost_0,
                        detection_time: Utc::now(),
                        market_resolved: false,
                        winning_outcome: None,
                        sim_pnl: None,
                        status: "OPEN".to_string(),
                        mode: mode.clone(),
                        created_at: Utc::now(),
                    signal_ts: 0, orderbook_ts: 0, order_sent_ts: 0, order_filled_ts: 0,
                    intended_price: 0.0, fill_price: 0.0, latency_total_ms: 0, latency_exec_ms: 0, slippage_bps: 0.0,
                    strategy: "hedge".to_string(),
                    };
                    let id_0 = self.state.db.insert_copy(&copy_0).await.unwrap_or(0);

                    // Record leg 2 (outcome_1)
                    let copy_1 = SimulatedCopy {
                        id: 0,
                        whale_wallet: "hedge-bot".to_string(),
                        whale_tx_hash: format!("hedge-{}-{}", slug, outcome_1),
                        market_slug: slug.clone(),
                        market_title: format!("{} 5m hedge", coin),
                        condition_id: String::new(),
                        asset_id: token_1.clone(),
                        outcome: outcome_1.clone(),
                        side: "BUY".to_string(),
                        whale_price: ask_1,
                        whale_size: shares,
                        sim_entry_price: ask_1,
                        sim_size_shares: shares,
                        sim_cost_usdc: cost_1,
                        detection_time: Utc::now(),
                        market_resolved: false,
                        winning_outcome: None,
                        sim_pnl: None,
                        status: "OPEN".to_string(),
                        mode,
                        created_at: Utc::now(),
                    signal_ts: 0, orderbook_ts: 0, order_sent_ts: 0, order_filled_ts: 0,
                    intended_price: 0.0, fill_price: 0.0, latency_total_ms: 0, latency_exec_ms: 0, slippage_bps: 0.0,
                    strategy: "hedge".to_string(),
                    };
                    let id_1 = self.state.db.insert_copy(&copy_1).await.unwrap_or(0);

                    self.subscribed_assets.lock().await.insert(token_0);
                    self.subscribed_assets.lock().await.insert(token_1);
                    hedged_slugs.insert(slug);

                    println!("  {} {} #{} + #{} | cost ${:.2} → payout ${:.2} → {} ",
                        "✅".green(),
                        coin.cyan(),
                        id_0, id_1,
                        total_cost,
                        payout,
                        format!("+${:.2}", profit).green().bold(),
                    );

                    self.state.broadcast(WsEvent::TradeDetected(copy_0));
                    self.state.broadcast(WsEvent::TradeDetected(copy_1));
                    if let Ok(stats) = self.state.db.get_trade_stats(None).await {
                        self.state.broadcast(WsEvent::StatsUpdate(stats));
                    }
                    if let Ok(bal) = self.state.db.get_balance_info().await {
                        self.state.broadcast(WsEvent::BalanceUpdate(bal));
                    }
                }
            }
        }
    }

    // ── Market Making (Stoikov) Loop ──

    async fn run_market_making_loop(&self) -> Result<()> {
        let markets = vec![
            ("btc-updown-5m", "btcusdt"),
            ("eth-updown-5m", "ethusdt"),
            ("sol-updown-5m", "solusdt"),
        ];

        // Shared price state
        let opening_prices: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        let latest_prices: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));

        // Spawn RTDS price feed
        let prices_clone = latest_prices.clone();
        let open_clone = opening_prices.clone();
        let bot_running = self.state.bot_running.clone();

        let price_ws_handle = tokio::spawn(async move {
            loop {
                if !*bot_running.read().await { return; }
                let ws_url = "wss://ws-live-data.polymarket.com";
                let conn = connect_async(ws_url).await;
                let (mut ws, _) = match conn {
                    Ok(c) => c,
                    Err(_) => { tokio::time::sleep(tokio::time::Duration::from_secs(3)).await; continue; }
                };
                let sub = serde_json::json!({
                    "action": "subscribe",
                    "subscriptions": [{"topic": "crypto_prices", "type": "update", "filters": ""}]
                });
                let _ = ws.send(WsMsg::Text(sub.to_string())).await;
                println!("  {} {}", "📡", "MM price feed connected".dimmed());
                let mut ping_timer = tokio::time::interval(tokio::time::Duration::from_secs(5));
                loop {
                    tokio::select! {
                        msg = ws.next() => {
                            match msg {
                                Some(Ok(WsMsg::Text(text))) => {
                                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) {
                                        if let Some(payload) = msg.get("payload") {
                                            let symbol = payload.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                                            let value = payload.get("value").and_then(json_as_f64).unwrap_or(0.0);
                                            if value > 0.0 && !symbol.is_empty() {
                                                prices_clone.lock().await.insert(symbol.to_string(), value);
                                                let now = Utc::now().timestamp();
                                                let window_start = now - (now % 300);
                                                let key = format!("{}-{}", symbol, window_start);
                                                open_clone.lock().await.entry(key).or_insert(value);
                                            }
                                        }
                                    }
                                }
                                Some(Ok(WsMsg::Close(_))) | None => break,
                                Some(Err(_)) => break,
                                _ => {}
                            }
                        }
                        _ = ping_timer.tick() => {
                            if ws.send(WsMsg::Ping(vec![])).await.is_err() { break; }
                        }
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });

        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        info!("Market Making (Stoikov) strategy started — quoting on {} markets", markets.len());

        let mut token_cache: HashMap<String, (String, String, String, String)> = HashMap::new();
        let mut last_report = tokio::time::Instant::now();
        let mut traded_windows: HashSet<String> = HashSet::new();
        // Inventory: slug → net shares (positive = more Up, negative = more Down)
        let mut inventory: HashMap<String, f64> = HashMap::new();

        loop {
            if !*self.state.bot_running.read().await {
                price_ws_handle.abort();
                return Ok(());
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

            let now = Utc::now().timestamp();
            let window_start = now - (now % 300);
            let time_remaining = window_start + 300 - now;

            // Config
            let gamma = self.get_config_f64("mm_gamma").await.unwrap_or(0.35);
            let sigma = self.get_config_f64("mm_sigma").await.unwrap_or(0.08);
            let k_param = self.get_config_f64("mm_k").await.unwrap_or(1.8);
            let min_edge = self.get_config_f64("mm_min_edge").await.unwrap_or(0.01);
            let sensitivity = self.get_config_f64("mm_sensitivity").await.unwrap_or(50.0);
            let trade_amount = self.get_config_f64("max_per_trade").await.unwrap_or(2.0);

            // Don't quote in first 15s (let market form) or last 10s (resolution risk)
            if time_remaining > 285 || time_remaining < 10 {
                if time_remaining > 295 {
                    traded_windows.clear();
                    inventory.clear();
                    if let Some(ref exec) = self.executor {
                        let _ = exec.cancel_all_orders().await;
                    }
                    let mut opens = opening_prices.lock().await;
                    opens.retain(|k, _| {
                        k.split('-').last()
                            .and_then(|ts| ts.parse::<i64>().ok())
                            .map(|ts| now - ts < 600)
                            .unwrap_or(false)
                    });
                }
                continue;
            }

            // Circuit breaker
            let max_losses = self.get_config_f64("max_consecutive_losses").await.unwrap_or(5.0) as u32;
            if *self.consecutive_losses.lock().await >= max_losses { continue; }
            let daily_limit = self.get_config_f64("daily_loss_limit").await.unwrap_or(10.0);
            if *self.daily_pnl.lock().await < -daily_limit { continue; }

            let balance = match self.state.db.get_balance_info().await {
                Ok(b) => b, Err(_) => continue,
            };
            if balance.available_capital < trade_amount { continue; }

            // Max open positions
            let max_open = self.get_config_f64("max_open_positions").await.unwrap_or(10.0) as usize;
            if self.open_market_slugs.lock().await.len() >= max_open { continue; }

            for (prefix, symbol) in &markets {
                let slug = format!("{}-{}", prefix, window_start);
                let coin = prefix.split('-').next().unwrap_or("?").to_uppercase();

                let current_price = latest_prices.lock().await.get(*symbol).copied().unwrap_or(0.0);
                let open_key = format!("{}-{}", symbol, window_start);
                let open_price = opening_prices.lock().await.get(&open_key).copied().unwrap_or(0.0);
                if current_price == 0.0 || open_price == 0.0 { continue; }

                let move_pct = ((current_price - open_price) / open_price) * 100.0;

                // Fair value via sigmoid: P(Up) = 1 / (1 + exp(-sensitivity * move))
                let fair_up = 1.0 / (1.0 + (-sensitivity * move_pct / 100.0).exp());
                let fair_down = 1.0 - fair_up;

                // Get tokens
                let tokens = if let Some(t) = token_cache.get(&slug) {
                    t.clone()
                } else {
                    match self.fetch_market_tokens(&slug).await {
                        Some(t) => { token_cache.insert(slug.clone(), t.clone()); t }
                        None => continue,
                    }
                };
                let (token_up, token_down, outcome_up, outcome_down) = tokens;

                // Get orderbooks
                let book_up = match self.clob_api.get_orderbook(&token_up).await {
                    Ok(b) => b, Err(_) => continue,
                };
                let book_down = match self.clob_api.get_orderbook(&token_down).await {
                    Ok(b) => b, Err(_) => continue,
                };

                let mkt_ask_up = book_up.best_ask().unwrap_or(0.50);
                let mkt_ask_down = book_down.best_ask().unwrap_or(0.50);

                // Stoikov model
                let net_inv = inventory.get(&slug).copied().unwrap_or(0.0);
                let t_secs = time_remaining as f64;

                // Reservation price: fair value adjusted for inventory risk
                let reservation = fair_up - gamma * net_inv * sigma.powi(2);

                // Optimal spread: wider when volatile or more time left
                let spread = (gamma * sigma.powi(2) * t_secs
                    + (2.0 / gamma) * (1.0 + gamma / k_param).ln())
                    .max(0.02); // minimum 2% spread

                let bid_up = (reservation - spread / 2.0).clamp(0.01, 0.98);
                let ask_up = (reservation + spread / 2.0).clamp(0.02, 0.99);

                // Edge check
                let edge_up = fair_up - mkt_ask_up;
                let edge_down = fair_down - mkt_ask_down;

                // Periodic report
                if last_report.elapsed().as_secs() >= 20 {
                    let edge_str = if edge_up.abs() > min_edge || edge_down.abs() > min_edge {
                        format!("edge={:.3}/{:.3} ✓", edge_up, edge_down).to_string()
                    } else {
                        format!("edge={:.3}/{:.3}", edge_up, edge_down)
                    };
                    println!("  {} {} fair={:.3} mkt={}@{:.3}/{}@{:.3} {} bid={:.3} ask={:.3} inv={:.1} [{}s]",
                        "📊".dimmed(), coin.dimmed(),
                        fair_up,
                        outcome_up.dimmed(), mkt_ask_up,
                        outcome_down.dimmed(), mkt_ask_down,
                        edge_str.dimmed(),
                        bid_up, ask_up,
                        net_inv,
                        time_remaining,
                    );
                    if coin == "SOL" { last_report = tokio::time::Instant::now(); }
                }

                // Kelly sizing: quarter Kelly
                let best_p = fair_up.max(fair_down);
                let kelly = ((2.0 * best_p - 1.0).max(0.0) * 0.25).min(0.05);
                let size_usdc = (kelly * balance.available_capital).min(trade_amount).max(0.0);
                if size_usdc < 0.50 { continue; }

                // BUY Up if underpriced
                let up_key = format!("{}-up", slug);
                if edge_up > min_edge && !traded_windows.contains(&up_key) {
                    let buy_price = bid_up.min(mkt_ask_up);
                    let shares = size_usdc / buy_price;
                    let depth = book_up.ask_depth_usd(buy_price + 0.05);
                    if depth < 20.0 { continue; }

                    println!("{} {} fair={:.3} mkt@{:.3} edge={:.3} → BUY {} @{:.3} ${:.2} [{}s]",
                        "📊 MM".blue().bold(), coin.cyan(),
                        fair_up, mkt_ask_up, edge_up,
                        outcome_up.white().bold(), buy_price, size_usdc, time_remaining,
                    );

                    if let Some(ref exec) = self.executor {
                        match exec.buy_limit(&token_up, shares, buy_price).await {
                            Ok(r) if r.success => {}
                            Ok(_) => { warn!("MM Up order not filled"); continue; }
                            Err(e) => { warn!(error = %e, "MM Up order error"); continue; }
                        }
                    }

                    let mode = self.state.db.get_bot_status().await
                        .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());
                    let copy = SimulatedCopy {
                        id: 0,
                        whale_wallet: "mm-stoikov".to_string(),
                        whale_tx_hash: format!("mm-{}-{}", slug, outcome_up),
                        market_slug: slug.clone(),
                        market_title: format!("{} 5m MM", coin),
                        condition_id: String::new(),
                        asset_id: token_up.clone(),
                        outcome: outcome_up.clone(),
                        side: "BUY".to_string(),
                        whale_price: buy_price,
                        whale_size: shares,
                        sim_entry_price: buy_price,
                        sim_size_shares: shares,
                        sim_cost_usdc: size_usdc,
                        detection_time: Utc::now(),
                        market_resolved: false,
                        winning_outcome: None,
                        sim_pnl: None,
                        status: "OPEN".to_string(),
                        mode,
                        created_at: Utc::now(),
                    signal_ts: 0, orderbook_ts: 0, order_sent_ts: 0, order_filled_ts: 0,
                    intended_price: 0.0, fill_price: 0.0, latency_total_ms: 0, latency_exec_ms: 0, slippage_bps: 0.0,
                    strategy: "mm".to_string(),
                    };
                    let id = self.state.db.insert_copy(&copy).await.unwrap_or(0);
                    self.open_market_slugs.lock().await.insert(slug.clone(), outcome_up.clone());
                    self.subscribed_assets.lock().await.insert(token_up.clone());
                    traded_windows.insert(up_key);
                    *inventory.entry(slug.clone()).or_insert(0.0) += shares;

                    println!("  {} #{} {}", "✅".green(), id,
                        format!("[bal ${:.0} inv={:.1}]", balance.available_capital - size_usdc, net_inv + shares).dimmed());
                    self.state.broadcast(WsEvent::TradeDetected(copy));
                }

                // BUY Down if underpriced
                let down_key = format!("{}-down", slug);
                if edge_down > min_edge && !traded_windows.contains(&down_key) {
                    let buy_price = (1.0 - ask_up).max(0.01).min(mkt_ask_down);
                    let shares = size_usdc / buy_price;
                    let depth = book_down.ask_depth_usd(buy_price + 0.05);
                    if depth < 20.0 { continue; }

                    println!("{} {} fair={:.3} mkt@{:.3} edge={:.3} → BUY {} @{:.3} ${:.2} [{}s]",
                        "📊 MM".blue().bold(), coin.cyan(),
                        fair_down, mkt_ask_down, edge_down,
                        outcome_down.white().bold(), buy_price, size_usdc, time_remaining,
                    );

                    if let Some(ref exec) = self.executor {
                        match exec.buy_limit(&token_down, shares, buy_price).await {
                            Ok(r) if r.success => {}
                            Ok(_) => { warn!("MM Down order not filled"); continue; }
                            Err(e) => { warn!(error = %e, "MM Down order error"); continue; }
                        }
                    }

                    let mode = self.state.db.get_bot_status().await
                        .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());
                    let copy = SimulatedCopy {
                        id: 0,
                        whale_wallet: "mm-stoikov".to_string(),
                        whale_tx_hash: format!("mm-{}-{}", slug, outcome_down),
                        market_slug: slug.clone(),
                        market_title: format!("{} 5m MM", coin),
                        condition_id: String::new(),
                        asset_id: token_down.clone(),
                        outcome: outcome_down.clone(),
                        side: "BUY".to_string(),
                        whale_price: buy_price,
                        whale_size: shares,
                        sim_entry_price: buy_price,
                        sim_size_shares: shares,
                        sim_cost_usdc: size_usdc,
                        detection_time: Utc::now(),
                        market_resolved: false,
                        winning_outcome: None,
                        sim_pnl: None,
                        status: "OPEN".to_string(),
                        mode,
                        created_at: Utc::now(),
                    signal_ts: 0, orderbook_ts: 0, order_sent_ts: 0, order_filled_ts: 0,
                    intended_price: 0.0, fill_price: 0.0, latency_total_ms: 0, latency_exec_ms: 0, slippage_bps: 0.0,
                    strategy: "mm".to_string(),
                    };
                    let id = self.state.db.insert_copy(&copy).await.unwrap_or(0);
                    if !self.open_market_slugs.lock().await.contains_key(&slug) {
                        self.open_market_slugs.lock().await.insert(slug.clone(), outcome_down.clone());
                    }
                    self.subscribed_assets.lock().await.insert(token_down.clone());
                    traded_windows.insert(down_key);
                    *inventory.entry(slug.clone()).or_insert(0.0) -= shares;

                    println!("  {} #{} {}", "✅".green(), id,
                        format!("[bal ${:.0}]", balance.available_capital - size_usdc).dimmed());
                    self.state.broadcast(WsEvent::TradeDetected(copy));
                }

                // Broadcast stats if any trade happened
                if traded_windows.contains(&format!("{}-up", slug)) || traded_windows.contains(&format!("{}-down", slug)) {
                    if let Ok(stats) = self.state.db.get_trade_stats(None).await {
                        self.state.broadcast(WsEvent::StatsUpdate(stats));
                    }
                    if let Ok(bal) = self.state.db.get_balance_info().await {
                        self.state.broadcast(WsEvent::BalanceUpdate(bal));
                    }
                }
            }
        }
    }

    // ── Balance Tracker: fetch real USDC balance every 30min ──

    async fn run_balance_tracker(&self) -> Result<()> {
        // Fetch immediately on start, then every 30 min
        let mut first = true;
        loop {
            if !*self.state.bot_running.read().await {
                return Ok(());
            }

            if !first {
                tokio::time::sleep(tokio::time::Duration::from_secs(1800)).await;
            }
            first = false;

            // Get wallet address from executor or env
            let address = if let Some(ref exec) = self.executor {
                exec.address.clone()
            } else {
                // test mode — try env
                match std::env::var("POLYMARKET_ADDRESS") {
                    Ok(a) => a,
                    Err(_) => continue,
                }
            };

            // Fetch USDC balance from Polymarket Data API
            let url = format!(
                "https://data-api.polymarket.com/wallets?address={}",
                address
            );
            let resp = match self.state.http_client.get(&url)
                .timeout(std::time::Duration::from_secs(10))
                .send().await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Balance fetch failed");
                    continue;
                }
            };

            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Try to extract USDC balance — API may return different formats
            let usdc_balance = body.get("usdcBalance")
                .or_else(|| body.get("balance"))
                .or_else(|| body.get("collateralBalance"))
                .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));

            // Also try the positions endpoint for total portfolio value
            let portfolio_url = format!(
                "https://data-api.polymarket.com/wallets/{}",
                address
            );
            let portfolio_balance = if let Ok(resp) = self.state.http_client.get(&portfolio_url)
                .timeout(std::time::Duration::from_secs(10))
                .send().await
            {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                body.get("usdcBalance")
                    .or_else(|| body.get("balance"))
                    .or_else(|| body.get("collateralBalance"))
                    .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .or(usdc_balance)
            } else {
                usdc_balance
            };

            if let Some(balance) = portfolio_balance {
                // Update simulated_capital in config with real balance
                let _ = self.state.db.set_config("real_usdc_balance", serde_json::json!(balance)).await;
                let _ = self.state.db.set_config("balance_updated_at",
                    serde_json::json!(Utc::now().to_rfc3339())).await;

                println!("  {} {}",
                    "💰".green(),
                    format!("Real balance: ${:.2} USDC (wallet {}...{})",
                        balance,
                        &address[..6.min(address.len())],
                        &address[address.len().saturating_sub(4)..],
                    ).green(),
                );

                // Broadcast balance update
                if let Ok(bal) = self.state.db.get_balance_info().await {
                    self.state.broadcast(WsEvent::BalanceUpdate(bal));
                }
            } else {
                warn!("Could not parse balance from API response");
                debug!(body = %body, "Balance API response");
            }
        }
    }

    /// Fetch token IDs and outcomes for a market from Gamma API
    async fn fetch_market_tokens(&self, slug: &str) -> Option<(String, String, String, String)> {
        let url = format!("https://gamma-api.polymarket.com/events/slug/{}", slug);
        let resp = self.rate_limited_get(&url).await.ok()?;
        let event: serde_json::Value = resp.json().await.ok()?;

        let market = event.get("markets")?.as_array()?.first()?;

        let token_ids = market.get("clobTokenIds")
            .or_else(|| market.get("clob_token_ids"))?;
        let tokens: Vec<String> = serde_json::from_value(token_ids.clone()).ok()
            .or_else(|| token_ids.as_str().and_then(|s| serde_json::from_str(s).ok()))?;

        let outcomes_val = market.get("outcomes")?;
        let outcomes: Vec<String> = serde_json::from_value(outcomes_val.clone()).ok()
            .or_else(|| outcomes_val.as_str().and_then(|s| serde_json::from_str(s).ok()))?;

        if tokens.len() >= 2 && outcomes.len() >= 2 {
            Some((tokens[0].clone(), tokens[1].clone(), outcomes[0].clone(), outcomes[1].clone()))
        } else {
            None
        }
    }

    // ── WebSocket Listener ──

    async fn run_ws_listener(&self, wallet_set: &HashSet<String>) -> Result<()> {
        let mut backoff_secs: u64 = 1;

        loop {
            if !*self.state.bot_running.read().await {
                info!("Bot stopped by user");
                return Ok(());
            }

            info!("Connecting to Polymarket RTDS WebSocket...");

            let ws_url = "wss://ws-live-data.polymarket.com";
            let connect_result = connect_async(ws_url).await;

            let (mut ws, _) = match connect_result {
                Ok(conn) => {
                    backoff_secs = 1; // Reset on success
                    conn
                }
                Err(e) => {
                    println!("  {} {}", "⚠️".yellow(), format!("RTDS conexão falhou, retry em {}s: {}", backoff_secs, e).yellow());
                    tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(60); // Exponential backoff, max 60s
                    continue;
                }
            };

            info!("RTDS WebSocket connected! Subscribing to trades...");

            let sub_msg = serde_json::json!({
                "action": "subscribe",
                "subscriptions": [{
                    "topic": "activity",
                    "type": "trades",
                    "filters": ""
                }]
            });
            if let Err(e) = ws.send(WsMsg::Text(sub_msg.to_string())).await {
                warn!(error = %e, "Failed to subscribe, reconnecting...");
                continue;
            }

            info!("Subscribed to RTDS activity/trades — listening for bot trades in real-time");

            let ping_interval = tokio::time::Duration::from_secs(5);
            let mut ping_timer = tokio::time::interval(ping_interval);
            let mut msg_count: u64 = 0;
            let mut last_heartbeat = tokio::time::Instant::now();
            let mut last_msg_time = tokio::time::Instant::now();
            let stale_timeout = std::time::Duration::from_secs(30);

            loop {
                if !*self.state.bot_running.read().await {
                    info!("Bot stopped by user");
                    return Ok(());
                }

                tokio::select! {
                    msg = ws.next() => {
                        match msg {
                            Some(Ok(WsMsg::Text(text))) => {
                                msg_count += 1;
                                last_msg_time = tokio::time::Instant::now();
                                if last_heartbeat.elapsed().as_secs() >= 60 {
                                    println!("  {} {}",
                                        "📡".dimmed(),
                                        format!("RTDS alive — {} msgs", msg_count).dimmed(),
                                    );
                                    last_heartbeat = tokio::time::Instant::now();
                                }
                                self.handle_rtds_message(&text, wallet_set).await;
                            }
                            Some(Ok(WsMsg::Ping(data))) => {
                                last_msg_time = tokio::time::Instant::now();
                                let _ = ws.send(WsMsg::Pong(data)).await;
                            }
                            Some(Ok(WsMsg::Close(_))) | None => {
                                println!("  {} {}", "⚠️".yellow(), "RTDS desconectou — reconectando...".yellow());
                                break;
                            }
                            Some(Err(e)) => {
                                println!("  {} {}", "⚠️".yellow(), format!("RTDS erro: {} — reconectando...", e).yellow());
                                break;
                            }
                            _ => {
                                last_msg_time = tokio::time::Instant::now();
                            }
                        }
                    }
                    _ = ping_timer.tick() => {
                        // Check for stale connection (no messages in 30s)
                        if last_msg_time.elapsed() > stale_timeout {
                            println!("  {} {}", "⚠️".yellow(), "RTDS sem dados há 30s — reconectando...".yellow());
                            break;
                        }
                        if ws.send(WsMsg::Ping(vec![])).await.is_err() {
                            println!("  {} {}", "⚠️".yellow(), "RTDS ping falhou — reconectando...".yellow());
                            break;
                        }
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(60);
        }
    }

    // ── Handle a single RTDS message ──

    async fn handle_rtds_message(&self, text: &str, wallet_set: &HashSet<String>) {
        let msg: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return,
        };

        // RTDS wraps trade data in { payload: { ... }, topic, type }
        // payload can be a single object OR an array of trades
        let payloads: Vec<serde_json::Value> = if let Some(payload) = msg.get("payload") {
            if let Some(arr) = payload.as_array() {
                arr.clone()
            } else {
                vec![payload.clone()]
            }
        } else if msg.is_array() {
            msg.as_array().cloned().unwrap_or_default()
        } else {
            vec![msg]
        };

        for event in &payloads {
            let proxy_wallet = event.get("proxyWallet")
                .or_else(|| event.get("proxy_wallet"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if proxy_wallet.is_empty() || !wallet_set.contains(&proxy_wallet.to_lowercase()) {
                continue;
            }

            // Log raw RTDS payload for tracked wallets (debug level)
            debug!(
                wallet = &proxy_wallet[..proxy_wallet.len().min(8)],
                raw = %event,
                "RTDS event from tracked wallet"
            );

            // Parse timestamp (handles both number and string)
            let mut trade_ts = event.get("timestamp")
                .and_then(json_as_i64)
                .unwrap_or(0);
            // Handle millisecond timestamps
            if trade_ts > 1_000_000_000_000 {
                trade_ts /= 1000;
            }

            // FAIL-SAFE: reject trades with unknown or pre-start timestamps
            // If trade_ts=0 (missing), 0 < started_at → rejected
            if trade_ts < self.started_at {
                debug!(
                    wallet = &proxy_wallet[..proxy_wallet.len().min(8)],
                    trade_ts,
                    started_at = self.started_at,
                    "Ignoring trade: timestamp before bot start"
                );
                continue;
            }

            let tx_hash = event.get("transactionHash")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            {
                let seen = self.seen_tx_hashes.lock().await;
                if tx_hash.is_empty() || seen.contains(tx_hash) {
                    continue;
                }
            }
            self.seen_tx_hashes.lock().await.insert(tx_hash.to_string());

            // Parse trade fields (handles both number and string values from RTDS)
            let trade = WalletTrade {
                proxy_wallet: proxy_wallet.to_string(),
                side: event.get("side").and_then(|v| v.as_str()).unwrap_or("BUY").to_string(),
                asset: event.get("asset").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                condition_id: event.get("conditionId")
                    .or_else(|| event.get("condition_id"))
                    .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                size: event.get("size").and_then(json_as_f64).unwrap_or(0.0),
                price: event.get("price").and_then(json_as_f64).unwrap_or(0.0),
                timestamp: trade_ts,
                title: event.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                slug: event.get("slug")
                    .or_else(|| event.get("marketSlug"))
                    .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                event_slug: event.get("eventSlug")
                    .or_else(|| event.get("event_slug"))
                    .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                outcome: event.get("outcome").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                outcome_index: event.get("outcomeIndex")
                    .or_else(|| event.get("outcome_index"))
                    .and_then(json_as_i64).unwrap_or(0) as i32,
                transaction_hash: tx_hash.to_string(),
            };

            debug!(
                wallet = &trade.proxy_wallet[..8],
                side = %trade.side,
                outcome = %trade.outcome,
                price = trade.price,
                market = %trade.title,
                "Whale trade detected"
            );

            // Read config fresh
            let max_per_trade = self.get_config_f64("max_per_trade").await.unwrap_or(5.0);
            let min_trade_size = self.get_config_f64("min_trade_size").await.unwrap_or(5.0);
            let slippage_pct = self.get_config_f64("slippage_estimate_pct").await.unwrap_or(2.0);
            let max_drift_pct = self.get_config_f64("max_price_drift_pct").await.unwrap_or(10.0);

            let mode = self.state.db.get_bot_status().await
                .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());

            if let Err(e) = self.process_trade(&trade, &mode, min_trade_size, max_per_trade, slippage_pct, max_drift_pct).await {
                warn!(error = %e, "Error processing trade");
            }
        }
    }

    // ── CLOB WebSocket — real-time market_resolved events ──

    async fn run_clob_resolution_listener(&self) -> Result<()> {
        // Wait a bit for initial copies to load
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        loop {
            if !*self.state.bot_running.read().await {
                return Ok(());
            }

            // Collect current asset IDs to subscribe
            let assets: Vec<String> = self.subscribed_assets.lock().await.iter().cloned().collect();
            if assets.is_empty() {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }

            let clob_ws_url = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
            let connect_result = connect_async(clob_ws_url).await;

            let (mut ws, _) = match connect_result {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "CLOB WS connection failed, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            // Subscribe with custom_feature_enabled for market_resolved events
            let sub_msg = serde_json::json!({
                "assets_ids": assets,
                "type": "market",
                "custom_feature_enabled": true
            });
            if let Err(e) = ws.send(WsMsg::Text(sub_msg.to_string())).await {
                warn!(error = %e, "CLOB WS subscribe failed");
                continue;
            }

            info!(assets = assets.len(), "CLOB WS subscribed for market_resolved events");

            let mut ping_timer = tokio::time::interval(tokio::time::Duration::from_secs(5));
            let mut resub_timer = tokio::time::interval(tokio::time::Duration::from_secs(30));

            loop {
                if !*self.state.bot_running.read().await {
                    return Ok(());
                }

                tokio::select! {
                    msg = ws.next() => {
                        match msg {
                            Some(Ok(WsMsg::Text(text))) => {
                                // Handle "PONG" control messages
                                if text == "PONG" { continue; }

                                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) {
                                    let event_type = event.get("event_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    if event_type == "market_resolved" {
                                        self.handle_market_resolved(&event).await;
                                    }
                                }
                            }
                            Some(Ok(WsMsg::Close(_))) | None => {
                                warn!("CLOB WS closed, reconnecting...");
                                break;
                            }
                            Some(Err(e)) => {
                                warn!(error = %e, "CLOB WS error, reconnecting...");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = ping_timer.tick() => {
                        let _ = ws.send(WsMsg::Text("PING".to_string())).await;
                    }
                    _ = resub_timer.tick() => {
                        // Re-subscribe with any new asset IDs
                        let current: Vec<String> = self.subscribed_assets.lock().await.iter().cloned().collect();
                        if current.len() != assets.len() {
                            let resub = serde_json::json!({
                                "assets_ids": current,
                                "type": "market",
                                "custom_feature_enabled": true
                            });
                            let _ = ws.send(WsMsg::Text(resub.to_string())).await;
                            debug!(assets = current.len(), "CLOB WS re-subscribed with updated assets");
                        }
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        }
    }

    /// Handle a market_resolved event from CLOB WebSocket (instant resolution)
    async fn handle_market_resolved(&self, event: &serde_json::Value) {
        let condition_id = event.get("market").and_then(|v| v.as_str()).unwrap_or("");
        let winning_outcome = event.get("winning_outcome").and_then(|v| v.as_str()).unwrap_or("");

        if condition_id.is_empty() || winning_outcome.is_empty() {
            return;
        }

        let open = match self.state.db.get_open_copies().await {
            Ok(o) => o,
            Err(_) => return,
        };

        let matching: Vec<&SimulatedCopy> = open.iter()
            .filter(|c| c.condition_id == condition_id || c.asset_id == condition_id)
            .collect();

        if matching.is_empty() {
            return;
        }

        let market_key = self.market_key_from_copy(matching[0]);
        self.open_market_slugs.lock().await.remove(&market_key);

        for c in &matching {
            let won = c.outcome.eq_ignore_ascii_case(winning_outcome);
            let pnl = if won { c.sim_size_shares - c.sim_cost_usdc } else { -c.sim_cost_usdc };

            let _ = self.state.db.resolve_copy(c.id, winning_outcome, pnl).await;
            self.update_circuit_breaker(won, pnl).await;

            {
                let short_market = Self::short_market(&c.market_title);
                let pnl_str = if pnl >= 0.0 {
                    format!("+${:.2}", pnl).green().bold()
                } else {
                    format!("-${:.2}", pnl.abs()).red().bold()
                };
                let label = if won {
                    "⚡WIN ".green().bold()
                } else {
                    "⚡LOSS".red().bold()
                };
                println!("{} {} {} winner={} {} {}",
                    label,
                    c.outcome.white(),
                    short_market.cyan(),
                    winning_outcome.white().bold(),
                    pnl_str,
                    format!("#{}", c.id).dimmed(),
                );
            }

            let mut resolved = (*c).clone();
            resolved.market_resolved = true;
            resolved.winning_outcome = Some(winning_outcome.to_string());
            resolved.sim_pnl = Some(pnl);
            resolved.status = "RESOLVED".to_string();
            self.state.broadcast(WsEvent::TradeResolved(resolved));
        }

        // Remove asset IDs from subscribed set
        for c in &matching {
            self.subscribed_assets.lock().await.remove(&c.asset_id);
        }

        // Broadcast updated stats/balance
        if let Ok(stats) = self.state.db.get_trade_stats(None).await {
            self.state.broadcast(WsEvent::StatsUpdate(stats));
        }
        if let Ok(balance) = self.state.db.get_balance_info().await {
            self.state.broadcast(WsEvent::BalanceUpdate(balance));
        }
    }

    // ── Gamma API Resolution Fallback (every 10s) ──

    async fn run_resolution_loop(&self) -> Result<()> {
        let interval = tokio::time::Duration::from_secs(5);
        loop {
            if !*self.state.bot_running.read().await {
                return Ok(());
            }
            tokio::time::sleep(interval).await;
            if let Err(e) = self.check_resolutions().await {
                warn!(error = %e, "Error checking resolutions");
            }
        }
    }

    async fn get_config_f64(&self, key: &str) -> Option<f64> {
        self.state.db.get_config(key).await.ok()?
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
    }

    /// Execute a real BUY order if in live mode. Returns true if executed.
    #[allow(dead_code)]
    async fn execute_real_buy(&self, token_id: &str, usdc_amount: f64) -> bool {
        if let Some(ref exec) = self.executor {
            if token_id.is_empty() {
                warn!("Cannot execute real order: no token_id");
                return false;
            }
            match exec.buy(token_id, usdc_amount).await {
                Ok(result) => result.success,
                Err(e) => {
                    println!("  {} {}", "❌", format!("Real order error: {}", e).red());
                    false
                }
            }
        } else {
            false // test mode, no real execution
        }
    }

    /// Rate-limited HTTP GET (max 20 requests per 10 seconds)
    async fn rate_limited_get(&self, url: &str) -> Result<reqwest::Response> {
        let mut ts = self.api_timestamps.lock().await;
        let now = Instant::now();
        while ts.front().map(|t| now - *t > std::time::Duration::from_secs(10)).unwrap_or(false) {
            ts.pop_front();
        }
        if ts.len() >= 20 {
            if let Some(oldest) = ts.front() {
                let wait = std::time::Duration::from_secs(10).saturating_sub(now - *oldest);
                drop(ts);
                tokio::time::sleep(wait).await;
                let mut ts = self.api_timestamps.lock().await;
                ts.push_back(Instant::now());
            }
        } else {
            ts.push_back(now);
        }
        Ok(self.state.http_client.get(url).timeout(std::time::Duration::from_secs(5)).send().await?)
    }

    /// Update circuit breaker after a trade resolves
    async fn update_circuit_breaker(&self, won: bool, pnl: f64) {
        let mut losses = self.consecutive_losses.lock().await;
        if won {
            *losses = 0;
        } else {
            *losses += 1;
        }
        *self.daily_pnl.lock().await += pnl;
    }

    // ── Process Trade (BUY = open position, SELL = close position) ──

    async fn process_trade(
        &self,
        trade: &WalletTrade,
        mode: &str,
        _min_trade_size: f64,
        max_per_trade: f64,
        _slippage_pct: f64,
        _max_drift_pct: f64,
    ) -> Result<()> {
        if self.state.db.trade_exists(&trade.transaction_hash).await? {
            return Ok(());
        }

        let market_key = self.resolve_market_slug(trade).await;

        // ── SELL = bot is exiting → close our position (no min size filter) ──
        if trade.side.eq_ignore_ascii_case("SELL") {
            return self.process_sell(trade, &market_key).await;
        }

        // ── BUY = bot is entering → copy the exact trade ──

        // Only require valid slug with timestamp
        if Self::parse_slug_timestamp(&market_key).is_none() && !market_key.is_empty() {
            // Allow non-5m markets too (hourly, daily) — copy everything
        }

        // Capital check only
        let balance = self.state.db.get_balance_info().await?;
        if balance.available_capital < max_per_trade {
            return Ok(());
        }

        // Use trade price directly — copy exactly what they paid
        let sim_entry_price = trade.price.min(0.99).max(0.01);
        let mut sim_size_shares = max_per_trade / sim_entry_price;
        // Polymarket minimum: 5 shares
        if sim_size_shares < 5.0 { sim_size_shares = 5.0; }
        let sim_cost = sim_size_shares * sim_entry_price;

        let mut copy = self.build_copy(trade, sim_entry_price, sim_size_shares, sim_cost, mode, "OPEN");
        if copy.market_slug.is_empty() && !market_key.is_empty() {
            copy.market_slug = market_key.clone();
        }

        // Execute real order if in live mode — use limit order at their price
        if let Some(ref exec) = self.executor {
            if !trade.asset.is_empty() {
                match exec.buy_limit(&trade.asset, sim_size_shares, sim_entry_price).await {
                    Ok(r) if r.success => {}
                    Ok(_) => { warn!("Copy order not filled"); }
                    Err(e) => { warn!(error = %e, "Copy order error"); }
                }
            }
        }

        let id = self.state.db.insert_copy(&copy).await?;

        if !market_key.is_empty() {
            self.open_market_slugs.lock().await.insert(market_key.clone(), trade.outcome.clone());
        }
        if !trade.asset.is_empty() {
            self.subscribed_assets.lock().await.insert(trade.asset.clone());
        }

        // ▶ ENTRY log
        {
            let short_market = Self::short_market(&trade.title);
            let remaining = balance.available_capital - sim_cost;
            println!("{} {} {} {} @{:.2} (bot @{:.2}) {} {}",
                "▶ ENTRY".green().bold(),
                trade.outcome.white().bold(),
                short_market.cyan(),
                format!("${:.2}", sim_cost).yellow(),
                sim_entry_price,
                trade.price,
                format!("[bal ${:.0}]", remaining).dimmed(),
                format!("#{}", id).dimmed(),
            );
        }

        let mut copy_with_id = copy;
        copy_with_id.id = id;
        self.state.broadcast(WsEvent::TradeDetected(copy_with_id));

        if let Ok(stats) = self.state.db.get_trade_stats(None).await {
            self.state.broadcast(WsEvent::StatsUpdate(stats));
        }
        if let Ok(bal) = self.state.db.get_balance_info().await {
            self.state.broadcast(WsEvent::BalanceUpdate(bal));
        }

        Ok(())
    }

    /// Whale SELL detected → close our matching open position at current price
    async fn process_sell(&self, trade: &WalletTrade, market_key: &str) -> Result<()> {
        // Find open copies on this market
        let open = self.state.db.get_open_copies().await?;
        let matching: Vec<&SimulatedCopy> = open.iter()
            .filter(|c| {
                let key = self.market_key_from_copy(c);
                key == market_key && c.outcome.eq_ignore_ascii_case(&trade.outcome)
            })
            .collect();

        if matching.is_empty() {
            return Ok(());
        }

        // Use bot sell price (with slippage) as our exit price
        let slippage_pct = self.get_config_f64("slippage_estimate_pct").await.unwrap_or(2.0);
        let slippage_mult = 1.0 - (slippage_pct / 100.0); // sell slippage = lower price
        let exit_price = trade.price * slippage_mult;

        // Free the market slot
        if !market_key.is_empty() {
            self.open_market_slugs.lock().await.remove(market_key);
        }

        for c in &matching {
            // P&L = (exit_price * shares) - cost
            let exit_value = exit_price * c.sim_size_shares;
            let pnl = exit_value - c.sim_cost_usdc;

            self.state.db.resolve_copy(c.id, &format!("SOLD@{:.3}", exit_price), pnl).await?;

            // ◀ EXIT log
            {
                let short_market = Self::short_market(&c.market_title);
                let pnl_str = if pnl >= 0.0 {
                    format!("+${:.2}", pnl).green().bold()
                } else {
                    format!("-${:.2}", pnl.abs()).red().bold()
                };
                println!("{} {} {} @{:.2} (entry @{:.2}) {} {}",
                    "◀ EXIT".yellow().bold(),
                    c.outcome.white(),
                    short_market.cyan(),
                    exit_price,
                    c.sim_entry_price,
                    pnl_str,
                    format!("#{}", c.id).dimmed(),
                );
            }

            let mut resolved = (*c).clone();
            resolved.market_resolved = true;
            resolved.winning_outcome = Some(format!("SOLD@{:.3}", exit_price));
            resolved.sim_pnl = Some(pnl);
            resolved.status = "RESOLVED".to_string();
            self.state.broadcast(WsEvent::TradeResolved(resolved));
        }

        // Clean up subscribed assets
        for c in &matching {
            self.subscribed_assets.lock().await.remove(&c.asset_id);
        }

        if let Ok(stats) = self.state.db.get_trade_stats(None).await {
            self.state.broadcast(WsEvent::StatsUpdate(stats));
        }
        if let Ok(bal) = self.state.db.get_balance_info().await {
            self.state.broadcast(WsEvent::BalanceUpdate(bal));
        }

        Ok(())
    }

    fn build_copy(
        &self,
        trade: &WalletTrade,
        sim_entry_price: f64,
        sim_size_shares: f64,
        sim_cost: f64,
        mode: &str,
        status: &str,
    ) -> SimulatedCopy {
        SimulatedCopy {
            id: 0,
            whale_wallet: trade.proxy_wallet.clone(),
            whale_tx_hash: trade.transaction_hash.clone(),
            market_slug: trade.slug.clone(),
            market_title: trade.title.clone(),
            condition_id: trade.condition_id.clone(),
            asset_id: trade.asset.clone(),
            outcome: trade.outcome.clone(),
            side: trade.side.clone(),
            whale_price: trade.price,
            whale_size: trade.size,
            sim_entry_price,
            sim_size_shares,
            sim_cost_usdc: sim_cost,
            detection_time: Utc::now(),
            market_resolved: false,
            winning_outcome: None,
            sim_pnl: None,
            status: status.to_string(),
            mode: mode.to_string(),
            created_at: Utc::now(),
            signal_ts: 0, orderbook_ts: 0, order_sent_ts: 0, order_filled_ts: 0,
            intended_price: 0.0, fill_price: 0.0, latency_total_ms: 0, latency_exec_ms: 0, slippage_bps: 0.0,
            strategy: "copy".to_string(),
        }
    }

    // ── Startup Position Sync ──
    // When the bot restarts, check every OPEN position to see if the tracked
    // wallet already exited while we were offline.  If so, close ours too —
    // in live mode we execute a real SELL, in test mode we just mark resolved.

    async fn sync_open_positions_on_startup(&self, open_copies: &[SimulatedCopy]) {
        let mode = self.state.db.get_bot_status().await
            .map(|s| s.mode).unwrap_or_else(|_| "test".to_string());

        let mut closed_count = 0u32;
        let mut checked_keys: HashSet<String> = HashSet::new();

        for copy in open_copies {
            let market_key = self.market_key_from_copy(copy);
            if market_key.is_empty() || checked_keys.contains(&market_key) {
                continue;
            }
            checked_keys.insert(market_key.clone());

            // Check if the whale still holds this position
            let position = self.check_tracked_position(&copy.whale_wallet, &copy.market_slug).await;

            match position {
                TrackedPosition::Exited => {
                    // Whale exited while bot was offline — close our position
                    info!(
                        market = %copy.market_title,
                        wallet = %copy.whale_wallet,
                        "Whale exited while bot was offline — closing position"
                    );

                    // Try to get current orderbook price for a better exit estimate
                    let exit_price = self.get_current_exit_price(copy).await
                        .unwrap_or(copy.sim_entry_price * 0.95);

                    // In live mode, execute real sell order
                    if mode == "live" {
                        if let Some(ref executor) = self.executor {
                            if !copy.asset_id.is_empty() {
                                match executor.sell(&copy.asset_id, copy.sim_size_shares, exit_price).await {
                                    Ok(result) if result.success => {
                                        info!(copy_id = copy.id, "Live SELL executed on startup sync");
                                    }
                                    Ok(_) => {
                                        warn!(copy_id = copy.id, "Live SELL failed — marking as bot exit");
                                    }
                                    Err(e) => {
                                        warn!(copy_id = copy.id, error = %e, "Live SELL error — marking as bot exit");
                                    }
                                }
                            }
                        }
                    }

                    // Close all copies for this market key
                    for c in open_copies.iter().filter(|c2| self.market_key_from_copy(c2) == market_key) {
                        let exit_value = exit_price * c.sim_size_shares;
                        let pnl = exit_value - c.sim_cost_usdc;

                        let _ = self.state.db.resolve_copy(c.id, "WHALE_EXITED_OFFLINE", pnl).await;
                        closed_count += 1;

                        let short_market = Self::short_market(&c.market_title);
                        let pnl_str = if pnl >= 0.0 {
                            format!("+${:.2}", pnl).green().bold()
                        } else {
                            format!("-${:.2}", pnl.abs()).red().bold()
                        };
                        println!("{} {} {} @{:.3} {} {}",
                            "◀ SYNC".yellow().bold(),
                            c.outcome.white(),
                            short_market.cyan(),
                            exit_price,
                            pnl_str,
                            format!("whale saiu offline #{}", c.id).dimmed(),
                        );

                        let mut resolved = c.clone();
                        resolved.market_resolved = true;
                        resolved.winning_outcome = Some("WHALE_EXITED_OFFLINE".to_string());
                        resolved.sim_pnl = Some(pnl);
                        resolved.status = "RESOLVED".to_string();
                        self.state.broadcast(WsEvent::TradeResolved(resolved));
                    }
                }
                TrackedPosition::Resolved => {
                    info!(
                        market = %copy.market_title,
                        "Market resolved while bot was offline — will be handled by resolution loop"
                    );
                }
                TrackedPosition::StillHolding => {
                    info!(
                        market = %Self::short_market(&copy.market_title),
                        wallet = %&copy.whale_wallet[..8],
                        "Whale still holding — keeping position open"
                    );
                }
                TrackedPosition::Unknown => {
                    warn!(
                        market = %copy.market_title,
                        "Could not check whale position — keeping open, will retry in resolution loop"
                    );
                }
            }

            // Small delay to avoid rate-limiting the API
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        }

        if closed_count > 0 {
            info!(closed = closed_count, "Startup sync: closed positions where whale exited offline");
            if let Ok(stats) = self.state.db.get_trade_stats(None).await {
                self.state.broadcast(WsEvent::StatsUpdate(stats));
            }
            if let Ok(bal) = self.state.db.get_balance_info().await {
                self.state.broadcast(WsEvent::BalanceUpdate(bal));
            }
        } else {
            info!("Startup sync: all open positions still active");
        }
    }

    /// Try to get a realistic exit price from the orderbook
    async fn get_current_exit_price(&self, copy: &SimulatedCopy) -> Option<f64> {
        if copy.asset_id.is_empty() {
            return None;
        }
        // Get best bid from orderbook as exit price
        let url = format!(
            "https://clob.polymarket.com/book?token_id={}",
            urlencoding::encode(&copy.asset_id)
        );
        let resp = self.state.http_client.get(&url).send().await.ok()?;
        let book: serde_json::Value = resp.json().await.ok()?;
        let best_bid = book.get("bids")?
            .as_array()?
            .first()?
            .get("price")?
            .as_str()?
            .parse::<f64>().ok()?;
        if best_bid > 0.01 {
            Some(best_bid)
        } else {
            None
        }
    }

    // ── Resolution Check ──

    async fn check_resolutions(&self) -> Result<()> {
        let open = match self.state.db.get_open_copies().await {
            Ok(o) => o,
            Err(e) => {
                warn!(error = %e, "Failed to get open copies");
                return Ok(());
            }
        };
        if open.is_empty() {
            return Ok(());
        }

        debug!(open_count = open.len(), "Checking resolutions");

        let now_ts = Utc::now().timestamp();
        let mut checked_keys: HashSet<String> = HashSet::new();
        let mut any_resolved = false;

        for copy in &open {
            let market_key = self.market_key_from_copy(copy);
            if market_key.is_empty() || checked_keys.contains(&market_key) {
                continue;
            }
            checked_keys.insert(market_key.clone());

            // Check if market time has passed
            let market_ended = if let Some(start_ts) = Self::parse_slug_timestamp(&copy.market_slug) {
                let is_15m = copy.market_slug.contains("15m");
                let duration = if is_15m { 900 } else { 300 };
                now_ts > start_ts + duration + 15
            } else {
                now_ts > copy.created_at.timestamp() + 315
            };

            if !market_ended {
                // Market still active → only check positions (detect early exit)
                let bot_position = self.check_tracked_position(&copy.whale_wallet, &copy.market_slug).await;
                match bot_position {
                    TrackedPosition::Exited => {
                        self.close_as_bot_exit(&open, &market_key, copy).await;
                        any_resolved = true;
                    }
                    TrackedPosition::Resolved => {
                        // Resolved early — fall through below
                    }
                    _ => continue, // Still holding — wait
                }
            }

            // Market ended OR resolved → find winner

            let winner = self.find_winner(&copy.market_title, &copy.market_slug, &copy.condition_id).await;

            if winner.is_none() {
                let age = now_ts - copy.created_at.timestamp();
                // Only log short-term markets (<30min). Long-term markets resolve silently.
                if age < 1800 {
                    // Only log every ~60s (when age is divisible by ~60)
                    if age % 60 < 12 {
                        println!("  {} {} {}",
                            "⏳".dimmed(),
                            Self::short_market(&copy.market_title).dimmed(),
                            format!("aguardando resolução ({}s)", age).dimmed(),
                        );
                    }
                }
            }

            if let Some(ref winner_outcome) = winner {
                // Free this market key for future trades
                self.open_market_slugs.lock().await.remove(&market_key);

                // Resolve all copies for this market
                for c in open.iter().filter(|c2| self.market_key_from_copy(c2) == market_key) {
                    let won = c.outcome.eq_ignore_ascii_case(winner_outcome);
                    let pnl = if won { c.sim_size_shares - c.sim_cost_usdc } else { -c.sim_cost_usdc };

                    self.state.db.resolve_copy(c.id, winner_outcome, pnl).await?;
                    self.update_circuit_breaker(won, pnl).await;

                    // Colored resolution log
                    {
                        let short_market = Self::short_market(&c.market_title);
                        let pnl_str = if pnl >= 0.0 {
                            format!("+${:.2}", pnl).green().bold()
                        } else {
                            format!("-${:.2}", pnl.abs()).red().bold()
                        };
                        let label = if won {
                            "◀ WIN ".green().bold()
                        } else {
                            "◀ LOSS".red().bold()
                        };
                        println!("{} {} {} winner={} {} {}",
                            label,
                            c.outcome.white(),
                            short_market.cyan(),
                            winner_outcome.white().bold(),
                            pnl_str,
                            format!("#{}", c.id).dimmed(),
                        );
                    }

                    let mut resolved = c.clone();
                    resolved.market_resolved = true;
                    resolved.winning_outcome = Some(winner_outcome.clone());
                    resolved.sim_pnl = Some(pnl);
                    resolved.status = "RESOLVED".to_string();
                    self.state.broadcast(WsEvent::TradeResolved(resolved));
                    any_resolved = true;
                }
            }
        }

        if any_resolved {
            if let Ok(stats) = self.state.db.get_trade_stats(None).await {
                self.state.broadcast(WsEvent::StatsUpdate(stats));
            }
            if let Ok(balance) = self.state.db.get_balance_info().await {
                self.state.broadcast(WsEvent::BalanceUpdate(balance));
            }
        }

        Ok(())
    }

    /// Check if the tracked bot still holds a position on this market
    async fn check_tracked_position(&self, wallet: &str, slug: &str) -> TrackedPosition {
        if slug.is_empty() || wallet.is_empty() {
            return TrackedPosition::Unknown;
        }

        let url = format!(
            "https://data-api.polymarket.com/positions?user={}&slug={}&sizeThreshold=0.1&limit=1",
            wallet, urlencoding::encode(slug)
        );

        let resp = match self.state.http_client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return TrackedPosition::Unknown,
        };

        let positions: Vec<serde_json::Value> = match resp.json().await {
            Ok(p) => p,
            Err(_) => return TrackedPosition::Unknown,
        };

        if positions.is_empty() {
            // No position found — bot exited or market resolved and redeemed
            // Check if market should have ended to distinguish exit vs resolved
            let now_ts = Utc::now().timestamp();
            let market_ended = Self::parse_slug_timestamp(slug)
                .map(|ts| {
                    let dur = if slug.contains("15m") { 900 } else if slug.contains("4h") { 14400 } else { 300 };
                    now_ts > ts + dur
                })
                .unwrap_or(false);

            if market_ended {
                TrackedPosition::Resolved
            } else {
                TrackedPosition::Exited
            }
        } else {
            let pos = &positions[0];
            let redeemable = pos.get("redeemable").and_then(|v| v.as_bool()).unwrap_or(false);
            if redeemable {
                TrackedPosition::Resolved
            } else {
                TrackedPosition::StillHolding
            }
        }
    }

    /// Whale exited early — close our position at estimated price
    async fn close_as_bot_exit(&self, open: &[SimulatedCopy], market_key: &str, _copy: &SimulatedCopy) {
        self.open_market_slugs.lock().await.remove(market_key);

        for c in open.iter().filter(|c2| self.market_key_from_copy(c2) == market_key) {
            // Estimate exit P&L: bot exited, use entry price as rough exit
            // Conservative: assume we'd get slightly less than entry price
            let exit_price = c.sim_entry_price * 0.95;
            let exit_value = exit_price * c.sim_size_shares;
            let pnl = exit_value - c.sim_cost_usdc;

            let _ = self.state.db.resolve_copy(c.id, "BOT_EXITED", pnl).await;

            let short_market = Self::short_market(&c.market_title);
            let pnl_str = if pnl >= 0.0 {
                format!("+${:.2}", pnl).green().bold()
            } else {
                format!("-${:.2}", pnl.abs()).red().bold()
            };
            println!("{} {} {} {} {}",
                "◀ EXIT".yellow().bold(),
                c.outcome.white(),
                short_market.cyan(),
                pnl_str,
                format!("bot saiu #{}", c.id).dimmed(),
            );

            let mut resolved = c.clone();
            resolved.market_resolved = true;
            resolved.winning_outcome = Some("BOT_EXITED".to_string());
            resolved.sim_pnl = Some(pnl);
            resolved.status = "RESOLVED".to_string();
            self.state.broadcast(WsEvent::TradeResolved(resolved));
        }

        for c in open.iter().filter(|c2| self.market_key_from_copy(c2) == market_key) {
            self.subscribed_assets.lock().await.remove(&c.asset_id);
        }

        if let Ok(stats) = self.state.db.get_trade_stats(None).await {
            self.state.broadcast(WsEvent::StatsUpdate(stats));
        }
        if let Ok(bal) = self.state.db.get_balance_info().await {
            self.state.broadcast(WsEvent::BalanceUpdate(bal));
        }
    }

    /// Shorten market title for compact display: "Bitcoin Up or Down - April 13, 2:30PM-2:35PM ET" → "BTC 2:30-2:35"
    fn short_market(title: &str) -> String {
        let t = title
            .replace("Bitcoin", "BTC")
            .replace("Ethereum", "ETH")
            .replace("Solana", "SOL")
            .replace(" Up or Down", "")
            .replace(" - ", " ");
        // Extract just the time range if present
        if let Some(pos) = t.find(", ") {
            let after = &t[pos + 2..];
            let coin = t.split_whitespace().next().unwrap_or("");
            let time = after.replace(" ET", "").replace("PM", "").replace("AM", "a");
            format!("{} {}", coin, time)
        } else {
            if t.len() > 30 { t[..30].to_string() } else { t }
        }
    }

    fn parse_slug_timestamp(slug: &str) -> Option<i64> {
        slug.split('-').rev().find_map(|part| {
            let n: i64 = part.parse().ok()?;
            if n > 1_577_836_800 && n < 1_893_456_000 { Some(n) } else { None }
        })
    }

    // ── Find Winner ──

    async fn find_winner(&self, _title: &str, slug: &str, _condition_id: &str) -> Option<String> {
        if slug.is_empty() {
            return None;
        }

        // Primary: GET /events?slug= (1 call, works for 5m/15m)
        let event = self.fetch_event_by_slug(slug).await;

        // Fallback: positions API → eventId → GET /events/{id}
        let event = match event {
            Some(e) => e,
            None => self.fetch_event_via_positions(slug).await?,
        };

        // Find our market inside the event and check resolution
        let markets = event.get("markets").and_then(|m| m.as_array())?;

        for m in markets {
            let m_slug = m.get("slug").and_then(|v| v.as_str()).unwrap_or("");
            if m_slug != slug {
                continue;
            }

            let closed = m.get("closed").and_then(|v| v.as_bool()).unwrap_or(false);
            if !closed {
                return None;
            }

            let prices = m.get("outcomePrices")
                .or_else(|| m.get("outcome_prices"))
                .and_then(|v| Self::parse_json_string_array(v))?;
            let outcomes = m.get("outcomes")
                .and_then(|v| Self::parse_json_string_array(v))?;

            for (i, price_str) in prices.iter().enumerate() {
                if let Ok(p) = price_str.parse::<f64>() {
                    if p > 0.95 {
                        return outcomes.get(i).cloned();
                    }
                }
            }

            return None;
        }
        None
    }

    async fn fetch_event_by_slug(&self, slug: &str) -> Option<serde_json::Value> {
        let url = format!("https://gamma-api.polymarket.com/events/slug/{}", slug);
        let resp = self.rate_limited_get(&url).await.ok()?;
        let body: serde_json::Value = resp.json().await.ok()?;
        if body.get("id").is_some() { Some(body) } else { None }
    }

    /// Fallback: positions API → eventId → GET /events/{id}
    async fn fetch_event_via_positions(&self, slug: &str) -> Option<serde_json::Value> {
        // Check cache
        let cached_id = self.condition_slug_cache.lock().await.get(slug).cloned();

        let event_id = if let Some(id) = cached_id {
            id
        } else {
            let wallets = self.state.db.get_enabled_wallets().await.ok()?;
            let mut found = None;
            for w in &wallets {
                let url = format!(
                    "https://data-api.polymarket.com/positions?user={}&slug={}&sizeThreshold=0.1&limit=1",
                    w.address, urlencoding::encode(slug)
                );
                if let Ok(resp) = self.state.http_client.get(&url).send().await {
                    if let Ok(positions) = resp.json::<Vec<serde_json::Value>>().await {
                        if let Some(eid) = positions.first()
                            .and_then(|p| p.get("eventId"))
                            .and_then(|v| v.as_str())
                        {
                            found = Some(eid.to_string());
                            break;
                        }
                    }
                }
            }
            let id = found?;
            self.condition_slug_cache.lock().await.insert(slug.to_string(), id.clone());
            id
        };

        let url = format!("https://gamma-api.polymarket.com/events/{}", event_id);
        let resp = self.state.http_client.get(&url).send().await.ok()?;
        resp.json().await.ok()
    }

    fn parse_json_string_array(val: &serde_json::Value) -> Option<Vec<String>> {
        if let Some(arr) = val.as_array() {
            return Some(arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());
        }
        if let Some(s) = val.as_str() {
            if let Ok(arr) = serde_json::from_str::<Vec<String>>(s) {
                return Some(arr);
            }
        }
        None
    }

}
