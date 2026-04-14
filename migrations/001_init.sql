-- Polymarket Copy Trading Bot - Database Schema

CREATE TABLE IF NOT EXISTS config (
    key TEXT PRIMARY KEY,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS tracked_wallets (
    id SERIAL PRIMARY KEY,
    address TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL DEFAULT '',
    pnl DOUBLE PRECISION NOT NULL DEFAULT 0,
    volume DOUBLE PRECISION NOT NULL DEFAULT 0,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS simulated_copies (
    id SERIAL PRIMARY KEY,
    whale_wallet TEXT NOT NULL,
    whale_tx_hash TEXT NOT NULL UNIQUE,
    market_slug TEXT NOT NULL DEFAULT '',
    market_title TEXT NOT NULL DEFAULT '',
    condition_id TEXT NOT NULL DEFAULT '',
    asset_id TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    side TEXT NOT NULL DEFAULT 'BUY',
    whale_price DOUBLE PRECISION NOT NULL,
    whale_size DOUBLE PRECISION NOT NULL,
    sim_entry_price DOUBLE PRECISION NOT NULL,
    sim_size_shares DOUBLE PRECISION NOT NULL,
    sim_cost_usdc DOUBLE PRECISION NOT NULL,
    detection_time TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    market_resolved BOOLEAN NOT NULL DEFAULT FALSE,
    winning_outcome TEXT,
    sim_pnl DOUBLE PRECISION,
    status TEXT NOT NULL DEFAULT 'OPEN',
    mode TEXT NOT NULL DEFAULT 'test',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS bot_status (
    id INTEGER PRIMARY KEY DEFAULT 1,
    running BOOLEAN NOT NULL DEFAULT FALSE,
    mode TEXT NOT NULL DEFAULT 'test',
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_copies_wallet ON simulated_copies(whale_wallet);
CREATE INDEX IF NOT EXISTS idx_copies_status ON simulated_copies(status);
CREATE INDEX IF NOT EXISTS idx_copies_mode ON simulated_copies(mode);
CREATE INDEX IF NOT EXISTS idx_copies_created ON simulated_copies(created_at DESC);

-- Default config
INSERT INTO config (key, value) VALUES
    ('simulated_capital', '500'::jsonb),
    ('max_per_trade', '50'::jsonb),
    ('slippage_estimate_pct', '2.0'::jsonb),
    ('poll_interval_secs', '5'::jsonb),
    ('max_price_drift_pct', '10.0'::jsonb),
    ('min_trade_size', '5.0'::jsonb),
    ('categories', '["CRYPTO"]'::jsonb)
ON CONFLICT (key) DO NOTHING;

-- Default bot status
INSERT INTO bot_status (id, running, mode) VALUES (1, false, 'test')
ON CONFLICT (id) DO NOTHING;
