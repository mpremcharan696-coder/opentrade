//! Trade intent and API response types — the contract between cloud and desktop.
//!
//! Implements the versioned request/response schema from the architecture plan
//! section 4.4, using strong types instead of untyped JSON strings.

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::fee::ComputedFee;
use crate::platform::TradeRoute;

/// Trade side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Buy,
    Sell,
}

/// A trade intent created by the cloud strategy engine.
///
/// This is the full server → desktop response. The desktop must validate
/// every field before showing a preview and requesting user approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeIntent {
    /// Schema version for forward compatibility.
    pub api_version: u32,

    /// Unique, single-use intent identifier.
    pub intent_id: String,

    /// When this intent was created (ISO-8601).
    pub created_at: String,

    /// When this intent expires (ISO-8601). Desktop must also check
    /// `last_valid_block_height` — this timestamp alone is insufficient.
    pub expires_at: String,

    /// Solana `lastValidBlockHeight` for the blockhash in the unsigned tx.
    pub last_valid_block_height: u64,

    /// Expected customer wallet public key (must match the signer).
    pub wallet_pubkey: Pubkey,

    /// Buy or sell.
    pub side: TradeSide,

    /// Token mint address.
    pub mint: Pubkey,

    /// Selected trading route.
    pub route: TradeRoute,

    /// Input amount in atomic units (lamports for SOL buys, token units for sells).
    /// Encoded as a string in JSON to avoid floating-point or JS integer issues.
    pub input_amount_atomic: u64,

    /// Minimum output in atomic units (tokens for buys, lamports for sells).
    pub minimum_output_atomic: u64,

    /// Slippage tolerance in basis points.
    pub slippage_bps: u16,

    /// Platform fee details.
    pub fee: ComputedFee,

    /// Slot at which the strategy fetched on-chain state.
    pub state_slot: u64,

    /// Base64-encoded serialized unsigned Solana transaction.
    /// The desktop MUST decode, inspect and validate this before signing.
    pub unsigned_tx_base64: String,
}

/// Outcome of a submitted trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeOutcome {
    /// Transaction confirmed on-chain successfully.
    Confirmed,
    /// Transaction landed but the program reverted.
    Reverted,
    /// Transaction expired (blockhash no longer valid).
    Expired,
    /// Status unknown — could not confirm or deny.
    Unknown,
}

/// Result of a completed trade attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    /// The intent this result corresponds to.
    pub intent_id: String,
    /// Transaction signature (if submitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Final outcome.
    pub outcome: TradeOutcome,
    /// Slot at which the transaction was confirmed (if confirmed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_slot: Option<u64>,
    /// Human-readable error message (if failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
