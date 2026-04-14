#[allow(dead_code)]
mod api;
mod config;
#[allow(dead_code)]
mod db;
mod execution;
#[allow(dead_code)]
mod models;
mod report;
mod simulator;
mod web;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::{routing::{delete, get, post, put}, Router};
use clap::{Parser, Subcommand};
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::db::Database;
use crate::web::state::AppState;

#[derive(Parser)]
#[command(name = "polymarket-copybot")]
#[command(about = "Polymarket Copy Trading Bot")]
struct Cli {
    /// PostgreSQL connection URL
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://copybot:copybot123@127.0.0.1:5433/copybot")]
    database_url: String,

    /// Web server port
    #[arg(short, long, default_value = "3001")]
    port: u16,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start web server + bot
    Serve,

    /// Discover top traders (CLI only)
    Discover {
        #[arg(short = 'C', long, default_value = "CRYPTO")]
        category: String,
        #[arg(short, long, default_value = "WEEK")]
        period: String,
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Required by polymarket-client-sdk (rustls)
    let _ = rustls::crypto::ring::default_provider().install_default();

    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    let http_client = api::build_http_client()
        .context("Failed to create HTTP client")?;

    match cli.command.unwrap_or(Commands::Serve) {
        Commands::Serve => {
            let db = Database::connect(&cli.database_url).await?;
            let state = AppState::new(db, http_client);

            // Build API router
            let api_router = Router::new()
                // Config
                .route("/config", get(web::routes::get_config))
                .route("/config", put(web::routes::set_config))
                // Wallets
                .route("/wallets", get(web::routes::get_wallets))
                .route("/wallets", post(web::routes::add_wallet))
                .route("/wallets/{id}", put(web::routes::toggle_wallet))
                .route("/wallets/{id}", delete(web::routes::delete_wallet))
                // Trades
                .route("/trades", get(web::routes::get_trades))
                .route("/trades/stats", get(web::routes::get_trade_stats))
                // Bot control
                .route("/status", get(web::routes::get_status))
                .route("/balance", get(web::routes::get_balance))
                .route("/bot/start", post(web::routes::start_bot))
                .route("/bot/stop", post(web::routes::stop_bot))
                .route("/bot/mode", post(web::routes::set_mode))
                // Leaderboard proxy
                .route("/leaderboard", get(web::routes::get_leaderboard))
                // Analyze wallet
                .route("/analyze/{wallet}", get(web::routes::analyze_wallet))
                // BTC markets
                .route("/markets/btc", get(web::routes::get_btc_markets));

            let app = Router::new()
                .nest("/api", api_router)
                .route("/ws", get(web::ws::ws_handler))
                .layer(CorsLayer::permissive())
                .with_state(state.clone());

            let addr = SocketAddr::from(([0, 0, 0, 0], cli.port));
            info!("API server starting on http://localhost:{}", cli.port);
            info!("WebSocket on ws://localhost:{}/ws", cli.port);

            // Spawn the bot loop in background
            let bot_state = state.clone();
            tokio::spawn(async move {
                // Sync bot_running flag with DB on startup
                if let Ok(db_status) = bot_state.db.get_bot_status().await {
                    if db_status.running {
                        *bot_state.bot_running.write().await = true;
                        info!("Bot was running before restart — resuming");
                    }
                }

                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                    if *bot_state.bot_running.read().await {
                        info!("Bot loop activated");
                        let mut sim = simulator::Simulator::new(bot_state.clone());
                        if let Err(e) = sim.run().await {
                            tracing::error!("Simulator error: {}", e);
                        }
                        info!("Bot loop ended");
                    }
                }
            });

            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app).await?;
        }

        Commands::Discover { category, period, limit } => {
            let data_api = api::data::DataApi::new(http_client, "https://data-api.polymarket.com");
            let entries = data_api.get_leaderboard(&category, &period, "PNL", limit).await?;

            use comfy_table::{presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement, Table};

            let mut table = Table::new();
            table.load_preset(UTF8_FULL).set_content_arrangement(ContentArrangement::Dynamic)
                .set_header(vec![
                    Cell::new("#").add_attribute(Attribute::Bold),
                    Cell::new("Wallet").add_attribute(Attribute::Bold),
                    Cell::new("P&L").add_attribute(Attribute::Bold),
                    Cell::new("Volume").add_attribute(Attribute::Bold),
                ]);

            for e in &entries {
                let w = if e.proxy_wallet.len() > 12 {
                    format!("{}...{}", &e.proxy_wallet[..6], &e.proxy_wallet[e.proxy_wallet.len()-4..])
                } else { e.proxy_wallet.clone() };

                table.add_row(vec![
                    Cell::new(&e.rank),
                    Cell::new(w),
                    Cell::new(format!("+${:.2}", e.pnl)).fg(Color::Green),
                    Cell::new(format!("${:.0}", e.vol)),
                ]);
            }
            println!("{}", table);
        }
    }

    Ok(())
}
