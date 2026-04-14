-- Analytics: latency + slippage tracking per trade

ALTER TABLE simulated_copies
    ADD COLUMN IF NOT EXISTS signal_ts BIGINT NOT NULL DEFAULT 0,        -- epoch ms: price signal detected
    ADD COLUMN IF NOT EXISTS orderbook_ts BIGINT NOT NULL DEFAULT 0,     -- epoch ms: orderbook fetched
    ADD COLUMN IF NOT EXISTS order_sent_ts BIGINT NOT NULL DEFAULT 0,    -- epoch ms: order sent to API
    ADD COLUMN IF NOT EXISTS order_filled_ts BIGINT NOT NULL DEFAULT 0,  -- epoch ms: fill confirmed
    ADD COLUMN IF NOT EXISTS intended_price DOUBLE PRECISION NOT NULL DEFAULT 0, -- price we wanted
    ADD COLUMN IF NOT EXISTS fill_price DOUBLE PRECISION NOT NULL DEFAULT 0,     -- price we actually got
    ADD COLUMN IF NOT EXISTS latency_total_ms BIGINT NOT NULL DEFAULT 0,        -- signal → fill (ms)
    ADD COLUMN IF NOT EXISTS latency_exec_ms BIGINT NOT NULL DEFAULT 0,         -- order sent → fill (ms)
    ADD COLUMN IF NOT EXISTS slippage_bps DOUBLE PRECISION NOT NULL DEFAULT 0;  -- (fill - intended) / intended * 10000
