//! Server state holding fee policy, strategy config, and active routes.

use shared_types::fee::{FeeConfig, FeeTier};
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub fee_config: FeeConfig,
    pub rpc_url: String,
}

impl AppState {
    pub fn new(fee_wallet: Pubkey, rpc_url: String) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                fee_config: FeeConfig {
                    version: 1,
                    fee_wallet,
                    tier: FeeTier::Default,
                },
                rpc_url,
            }),
        }
    }
}
