//! Axum Server entrypoint.

use solana_sdk::pubkey;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use trading_server::create_app;
use trading_server::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "trading_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Default platform treasury wallet configured for mainnet fee collection
    let default_fee_wallet = pubkey!("AnWnLUpa1ohdgp3WRmhPpxXEusmcEDZNtYTsCM8qq9eH");
    let rpc_url = std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| {
        "https://mainnet.helius-rpc.com/?api-key=eda6e938-7c34-4332-bf0e-3722786de677".to_string()
    });

    let state = AppState::new(default_fee_wallet, rpc_url);
    let app = create_app(state);

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .unwrap_or(8080);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Starting trading strategy server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
