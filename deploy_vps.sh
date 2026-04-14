#!/bin/bash
# === DEPLOY POLYMARKET BOT TO VPS ===
# Rode na VPS: bash /opt/bot/deploy_vps.sh

set -e

echo "=== 1/5 — Instalando dependencias ==="
apt update && apt install -y curl git build-essential pkg-config libssl-dev docker.io docker-compose-v2
systemctl enable docker && systemctl start docker

echo "=== 2/5 — Instalando Rust ==="
if ! command -v cargo &> /dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source ~/.cargo/env
fi
source ~/.cargo/env

echo "=== 3/5 — Subindo PostgreSQL ==="
cd /opt/bot
docker compose up -d

# Wait for DB
echo "Esperando PostgreSQL..."
sleep 5

# Apply migrations
docker exec bot-db-1 psql -U copybot -d copybot -f /docker-entrypoint-initdb.d/001_init.sql 2>/dev/null || true

# Apply analytics migration
docker exec -i bot-db-1 psql -U copybot -d copybot << 'SQL'
ALTER TABLE simulated_copies
    ADD COLUMN IF NOT EXISTS signal_ts BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS orderbook_ts BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS order_sent_ts BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS order_filled_ts BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS intended_price DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS fill_price DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS latency_total_ms BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS latency_exec_ms BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS slippage_bps DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS strategy TEXT NOT NULL DEFAULT 'unknown';
CREATE INDEX IF NOT EXISTS idx_copies_strategy ON simulated_copies(strategy);
SQL

echo "=== 4/5 — Compilando bot (pode demorar ~3min) ==="
cd /opt/bot
cargo build --release

echo "=== 5/5 — Configurando .env ==="
if [ ! -f .env ]; then
  cp .env.example .env 2>/dev/null || cat > .env << 'ENV'
DATABASE_URL=postgres://copybot:copybot123@127.0.0.1:5433/copybot
POLYMARKET_PRIVATE_KEY=SUA_PRIVATE_KEY_AQUI
RUST_LOG=info
ENV
  echo "EDITE o .env com sua private key: nano /opt/bot/.env"
fi

echo ""
echo "=== PRONTO! ==="
echo "1. Edite o .env:  nano /opt/bot/.env"
echo "2. Rode o bot:    cd /opt/bot && ./target/release/polymarket-copybot"
echo "3. Com screen:    screen -S bot -dm ./target/release/polymarket-copybot"
echo "4. Ver logs:      screen -r bot"
echo "5. Frontend:      http://$(hostname -I | awk '{print $1}'):3001"
