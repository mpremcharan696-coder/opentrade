//! Platform identifiers for supported trading venues.

use serde::{Deserialize, Serialize};

/// Supported trading platforms / launch pads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// Pump.fun bonding curve (pre-graduation).
    PumpFun,
    /// PumpSwap AMM (post-graduation).
    PumpSwap,
    /// Raydium AMM / CLMM / CPMM
    Raydium,
    /// Meteora DLMM / Dynamic AMM
    Meteora,
    /// Orca Whirlpool
    Orca,
    /// OKX DEX Router aggregation across all 30+ Solana DEX venues
    OkxRouter,
}

/// Which instruction generation to use for a bonding-curve trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PumpInstructionVersion {
    /// Legacy 18-account buy / 16-17-account sell. SOL-paired only.
    Legacy,
    /// V2: 27-account buy / 26-account sell. All coin types.
    V2,
    /// V3: 17-account. Latest, streamlined. Fails on cashback coins.
    V3,
}

/// Trading route selected by the strategy engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeRoute {
    /// Buy/sell on the bonding curve (pre-graduation).
    PumpCurve { version: PumpInstructionVersion },
    /// Buy/sell on the PumpSwap AMM (post-graduation).
    PumpSwapAmm,
    /// Buy/sell routed through OKX DEX Router V1 program.
    OkxRouter { dex_id: u8 },
}

/// Transaction submission strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionRoute {
    /// Standard RPC sendTransaction.
    Rpc,
    /// Jito block engine bundle.
    JitoBundle,
}
