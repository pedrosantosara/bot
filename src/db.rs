use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tracing::info;

use crate::models::{BotStatus, ConfigEntry, SimulatedCopy, TrackedWallet, TradeStats};

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .context("Failed to connect to PostgreSQL")?;

        info!("Connected to PostgreSQL");
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ── Config ──

    pub async fn get_config(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT value FROM config WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("value")))
    }

    pub async fn get_all_config(&self) -> Result<Vec<ConfigEntry>> {
        let rows = sqlx::query_as::<_, ConfigEntry>(
            "SELECT key, value, updated_at FROM config ORDER BY key"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn set_config(&self, key: &str, value: serde_json::Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO config (key, value, updated_at) VALUES ($1, $2, NOW())
             ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = NOW()"
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Tracked Wallets ──

    pub async fn get_wallets(&self) -> Result<Vec<TrackedWallet>> {
        let wallets = sqlx::query_as::<_, TrackedWallet>(
            "SELECT id, address, label, pnl, volume, enabled, added_at
             FROM tracked_wallets ORDER BY pnl DESC"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(wallets)
    }

    pub async fn get_enabled_wallets(&self) -> Result<Vec<TrackedWallet>> {
        let wallets = sqlx::query_as::<_, TrackedWallet>(
            "SELECT id, address, label, pnl, volume, enabled, added_at
             FROM tracked_wallets WHERE enabled = TRUE ORDER BY pnl DESC"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(wallets)
    }

    pub async fn add_wallet(&self, address: &str, label: &str, pnl: f64, volume: f64) -> Result<TrackedWallet> {
        let wallet = sqlx::query_as::<_, TrackedWallet>(
            "INSERT INTO tracked_wallets (address, label, pnl, volume)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (address) DO UPDATE SET label = $2, pnl = $3, volume = $4
             RETURNING id, address, label, pnl, volume, enabled, added_at"
        )
        .bind(address)
        .bind(label)
        .bind(pnl)
        .bind(volume)
        .fetch_one(&self.pool)
        .await?;
        Ok(wallet)
    }

    pub async fn toggle_wallet(&self, id: i32, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE tracked_wallets SET enabled = $2 WHERE id = $1")
            .bind(id)
            .bind(enabled)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_wallet(&self, id: i32) -> Result<()> {
        sqlx::query("DELETE FROM tracked_wallets WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Simulated Copies / Trades ──

    pub async fn trade_exists(&self, tx_hash: &str) -> Result<bool> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM simulated_copies WHERE whale_tx_hash = $1")
            .bind(tx_hash)
            .fetch_one(&self.pool)
            .await?;
        let count: i64 = row.get("cnt");
        Ok(count > 0)
    }

    pub async fn insert_copy(&self, copy: &SimulatedCopy) -> Result<i32> {
        let row = sqlx::query(
            "INSERT INTO simulated_copies (
                whale_wallet, whale_tx_hash, market_slug, market_title,
                condition_id, asset_id, outcome, side,
                whale_price, whale_size, sim_entry_price, sim_size_shares,
                sim_cost_usdc, detection_time, market_resolved,
                winning_outcome, sim_pnl, status, mode,
                signal_ts, orderbook_ts, order_sent_ts, order_filled_ts,
                intended_price, fill_price, latency_total_ms, latency_exec_ms, slippage_bps,
                strategy
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
                      $20,$21,$22,$23,$24,$25,$26,$27,$28,$29)
            RETURNING id"
        )
        .bind(&copy.whale_wallet)
        .bind(&copy.whale_tx_hash)
        .bind(&copy.market_slug)
        .bind(&copy.market_title)
        .bind(&copy.condition_id)
        .bind(&copy.asset_id)
        .bind(&copy.outcome)
        .bind(&copy.side)
        .bind(copy.whale_price)
        .bind(copy.whale_size)
        .bind(copy.sim_entry_price)
        .bind(copy.sim_size_shares)
        .bind(copy.sim_cost_usdc)
        .bind(copy.detection_time)
        .bind(copy.market_resolved)
        .bind(&copy.winning_outcome)
        .bind(copy.sim_pnl)
        .bind(&copy.status)
        .bind(&copy.mode)
        .bind(copy.signal_ts)
        .bind(copy.orderbook_ts)
        .bind(copy.order_sent_ts)
        .bind(copy.order_filled_ts)
        .bind(copy.intended_price)
        .bind(copy.fill_price)
        .bind(copy.latency_total_ms)
        .bind(copy.latency_exec_ms)
        .bind(copy.slippage_bps)
        .bind(&copy.strategy)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn resolve_copy(&self, id: i32, winning_outcome: &str, pnl: f64) -> Result<()> {
        sqlx::query(
            "UPDATE simulated_copies SET market_resolved = TRUE,
             winning_outcome = $2, sim_pnl = $3, status = 'RESOLVED'
             WHERE id = $1"
        )
        .bind(id)
        .bind(winning_outcome)
        .bind(pnl)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_telegram_message_id(&self, id: i32, message_id: i64) -> Result<()> {
        sqlx::query("UPDATE simulated_copies SET telegram_message_id = $2 WHERE id = $1")
            .bind(id)
            .bind(message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_open_copies(&self) -> Result<Vec<SimulatedCopy>> {
        let copies = sqlx::query_as::<_, SimulatedCopy>(
            "SELECT id, whale_wallet, whale_tx_hash, market_slug, market_title,
                    condition_id, asset_id, outcome, side,
                    whale_price, whale_size, sim_entry_price, sim_size_shares,
                    sim_cost_usdc, detection_time, market_resolved,
                    winning_outcome, sim_pnl, status, mode, created_at,
                    signal_ts, orderbook_ts, order_sent_ts, order_filled_ts,
                    intended_price, fill_price, latency_total_ms, latency_exec_ms, slippage_bps, strategy, telegram_message_id
             FROM simulated_copies WHERE status = 'OPEN' ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(copies)
    }

    pub async fn get_trades(&self, limit: i64, offset: i64, mode: Option<&str>) -> Result<Vec<SimulatedCopy>> {
        let copies = if let Some(m) = mode {
            sqlx::query_as::<_, SimulatedCopy>(
                "SELECT id, whale_wallet, whale_tx_hash, market_slug, market_title,
                        condition_id, asset_id, outcome, side,
                        whale_price, whale_size, sim_entry_price, sim_size_shares,
                        sim_cost_usdc, detection_time, market_resolved,
                        winning_outcome, sim_pnl, status, mode, created_at,
                    signal_ts, orderbook_ts, order_sent_ts, order_filled_ts,
                    intended_price, fill_price, latency_total_ms, latency_exec_ms, slippage_bps, strategy, telegram_message_id
                 FROM simulated_copies WHERE mode = $3
                 ORDER BY created_at DESC LIMIT $1 OFFSET $2"
            )
            .bind(limit)
            .bind(offset)
            .bind(m)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, SimulatedCopy>(
                "SELECT id, whale_wallet, whale_tx_hash, market_slug, market_title,
                        condition_id, asset_id, outcome, side,
                        whale_price, whale_size, sim_entry_price, sim_size_shares,
                        sim_cost_usdc, detection_time, market_resolved,
                        winning_outcome, sim_pnl, status, mode, created_at,
                    signal_ts, orderbook_ts, order_sent_ts, order_filled_ts,
                    intended_price, fill_price, latency_total_ms, latency_exec_ms, slippage_bps, strategy, telegram_message_id
                 FROM simulated_copies
                 ORDER BY created_at DESC LIMIT $1 OFFSET $2"
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(copies)
    }

    pub async fn get_trade_stats(&self, mode: Option<&str>) -> Result<TradeStats> {
        let mode_filter = mode.unwrap_or("%");
        let row = sqlx::query(
            "SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE status = 'RESOLVED') as resolved,
                COUNT(*) FILTER (WHERE status = 'OPEN') as open,
                COUNT(*) FILTER (WHERE status LIKE 'SKIPPED%') as skipped,
                COUNT(*) FILTER (WHERE sim_pnl > 0) as wins,
                COUNT(*) FILTER (WHERE sim_pnl <= 0 AND status = 'RESOLVED') as losses,
                COALESCE(SUM(sim_pnl) FILTER (WHERE status = 'RESOLVED'), 0) as total_pnl,
                COALESCE(SUM(sim_cost_usdc) FILTER (WHERE status = 'RESOLVED'), 0) as total_invested,
                COALESCE(SUM(sim_cost_usdc) FILTER (WHERE status = 'OPEN'), 0) as open_invested,
                COALESCE(AVG(sim_entry_price - whale_price) FILTER (WHERE whale_price > 0 AND status != 'SKIPPED'), 0) as avg_slippage,
                COALESCE(AVG(sim_pnl) FILTER (WHERE sim_pnl > 0), 0) as avg_win,
                COALESCE(AVG(sim_pnl) FILTER (WHERE sim_pnl <= 0 AND status = 'RESOLVED'), 0) as avg_loss
             FROM simulated_copies WHERE mode LIKE $1"
        )
        .bind(mode_filter)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.get("total");
        let resolved: i64 = row.get("resolved");
        let wins: i64 = row.get("wins");

        Ok(TradeStats {
            total: total as i32,
            resolved: resolved as i32,
            open: row.get::<i64, _>("open") as i32,
            skipped: row.get::<i64, _>("skipped") as i32,
            wins: wins as i32,
            losses: row.get::<i64, _>("losses") as i32,
            total_pnl: row.get("total_pnl"),
            total_invested: row.get("total_invested"),
            open_invested: row.get("open_invested"),
            win_rate: if resolved > 0 { (wins as f64 / resolved as f64) * 100.0 } else { 0.0 },
            avg_slippage: row.get("avg_slippage"),
            avg_win: row.get("avg_win"),
            avg_loss: row.get("avg_loss"),
            roi: {
                let invested: f64 = row.get("total_invested");
                let pnl: f64 = row.get("total_pnl");
                if invested > 0.0 { (pnl / invested) * 100.0 } else { 0.0 }
            },
        })
    }

    // ── Bot Status ──

    pub async fn get_bot_status(&self) -> Result<BotStatus> {
        let status = sqlx::query_as::<_, BotStatus>(
            "SELECT id, running, mode, started_at, updated_at FROM bot_status WHERE id = 1"
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(status)
    }

    pub async fn set_bot_status(&self, running: bool, mode: &str) -> Result<()> {
        let started = if running { Some(chrono::Utc::now()) } else { None };
        sqlx::query(
            "UPDATE bot_status SET running = $1, mode = $2, started_at = $3, updated_at = NOW() WHERE id = 1"
        )
        .bind(running)
        .bind(mode)
        .bind(started)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Balance Info ──

    pub async fn get_balance_info(&self) -> Result<crate::models::BalanceInfo> {
        // Prefer real USDC balance (fetched by balance tracker from Polymarket API)
        // over the manually-set simulated_capital config value.
        let capital_val = self.get_config("real_usdc_balance").await?
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| 0.0);
        let capital_val = if capital_val > 0.0 {
            capital_val
        } else {
            self.get_config("simulated_capital").await?
                .and_then(|v| v.as_f64())
                .unwrap_or(500.0)
        };

        let row = sqlx::query(
            "SELECT
                COALESCE(SUM(sim_pnl) FILTER (WHERE status = 'RESOLVED'), 0) as realized_pnl,
                COALESCE(SUM(sim_cost_usdc) FILTER (WHERE status = 'OPEN'), 0) as open_cost
             FROM simulated_copies WHERE status != 'SKIPPED'"
        )
        .fetch_one(&self.pool)
        .await?;

        let realized_pnl: f64 = row.get("realized_pnl");
        let open_cost: f64 = row.get("open_cost");
        let current_balance = capital_val + realized_pnl;
        let available = current_balance - open_cost;

        Ok(crate::models::BalanceInfo {
            initial_capital: capital_val,
            current_balance,
            total_pnl: realized_pnl,
            pnl_pct: if capital_val > 0.0 { (realized_pnl / capital_val) * 100.0 } else { 0.0 },
            open_positions_value: open_cost,
            available_capital: available.max(0.0),
        })
    }
}
