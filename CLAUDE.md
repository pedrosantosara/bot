# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
# Backend (Rust)
cargo build --release          # ~3-5 min first build
./target/release/polymarket-copybot          # default: Serve mode
./target/release/polymarket-copybot discover # CLI leaderboard

# Frontend (React + Vite + Tailwind)
cd frontend && npm install && npm run build  # production build to dist/
cd frontend && npx vite preview              # serve production build on :5173
cd frontend && npx vite                      # dev server on :5173

# Database (PostgreSQL via Docker)
docker compose up -d                         # postgres on port 5433
```

## Deployment (VPS)

The bot runs as two systemd services:
- `copybot-backend` — Rust binary (port 3001), auto-starts PostgreSQL via Docker
- `copybot-frontend` — Vite preview (port 5173), proxies /api and /ws to backend

```bash
systemctl restart copybot-backend copybot-frontend
journalctl -u copybot-backend -f   # live logs
```

## Architecture

**Rust backend** (`src/`) — axum web server + copy trading engine:

- `main.rs` — CLI (clap), axum router with session auth middleware, spawns bot loop
- `simulator.rs` — Core trading engine (~2500 lines). Runs multiple concurrent loops:
  - **Activity Poller**: polls `/activity` API every 5s per tracked wallet (primary detection)
  - **RTDS WebSocket**: connects to `wss://ws-live-data.polymarket.com` for real-time trades (only catches trades where proxyWallet matches — which is unreliable since Polymarket uses different proxy wallets per trade)
  - **Resolution Loop**: checks open positions every 5s, detects whale exits and market resolution
  - **CLOB Resolution Listener**: WebSocket for `market_resolved` events
  - **Balance Tracker**: periodic balance/portfolio sync
  - **Startup Sync**: on restart, checks all OPEN positions against whale positions; closes if whale exited offline
- `execution.rs` — Polymarket CLOB order execution (buy/sell/limit via `polymarket-client-sdk`)
- `telegram.rs` — Telegram notifications (entry/exit/resolution), sent in background after trade execution
- `web/` — REST API routes, WebSocket broadcasting, session auth, log streaming
- `api/` — HTTP clients: `data.rs` (Data API), `gamma.rs` (Gamma API), `clob.rs` (CLOB orderbook)
- `db.rs` — PostgreSQL via sqlx (raw queries, no migrations runner)
- `models.rs` — Shared types: `SimulatedCopy`, `WalletTrade`, `TrackedWallet`, `WsEvent`, etc.

**React frontend** (`frontend/src/`) — SPA with real-time WebSocket updates:
- `App.tsx` — Main app with auth gate (Login component), tabs: Dashboard/Analyze/History/Logs/Settings
- `hooks/useApi.ts` — API client with Bearer token from localStorage, auto-logout on 401
- `hooks/useWebSocket.ts` — WebSocket connection for live trade/stats/log events

## Key Polymarket APIs

- **Data API** (`https://data-api.polymarket.com`):
  - `/activity?user={address}` — trades by **profile address** (reliable for copy detection)
  - `/positions?user={address}` — current open positions
  - `/closed-positions?user={address}` — historical closed positions
- **Gamma API** (`https://gamma-api.polymarket.com`):
  - `/events?slug={slug}` — market data, resolution status, outcome prices
  - `/public-search?q={query}` — search markets
- **CLOB API** (`https://clob.polymarket.com`):
  - `/book?token_id={id}` — orderbook (best bid/ask)
  - Order submission via `polymarket-client-sdk`
- **RTDS WebSocket** (`wss://ws-live-data.polymarket.com`):
  - Topic `activity/trades` — all platform trades in real-time
  - Sends `proxyWallet` field which is the **actual proxy wallet**, NOT the profile address

## Important: Proxy Wallet vs Profile Address

Polymarket users have a **profile address** (visible on leaderboard/URLs) and one or more **proxy wallets** (used for on-chain trades). The Data API `/activity` endpoint accepts the profile address and returns trades. The RTDS WebSocket sends the proxy wallet, which is different and unpredictable. The activity poller solves this by polling `/activity` with the profile address.

## Trade Execution Flow (latency-critical path)

1. Activity poller detects trade (~5s polling interval)
2. `process_trade()`: execute order FIRST via CLOB (`buy_limit`), then DB insert
3. Stats/balance update + Telegram notification run in background `tokio::spawn`

## Environment Variables (.env)

- `DATABASE_URL` — PostgreSQL connection string
- `POLYMARKET_PRIVATE_KEY` — wallet private key for live trading
- `BOT_PASSWORD` — session password for web UI auth
- `TELEGRAM_BOT_TOKEN` / `TELEGRAM_CHAT_ID` — Telegram notifications
- `RUST_LOG` — log level (default: info)

## Database

PostgreSQL on port 5433 (Docker). Tables: `config`, `tracked_wallets`, `simulated_copies`, `bot_status`. Migrations in `migrations/` are applied manually via `deploy_vps.sh`, not auto-run.

## Trading Strategies

Configured via `strategy` key in config table:
- `copy` (default) — copy trades from tracked wallets
- `oracle` — oracle lag detection on crypto price movements
- `hedge` — buy Up+Down token pairs when sum < threshold
- `mm` — market making with Stoikov model
