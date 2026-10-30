//! Token information types — Rust port of the Python `TokenInfo` dataclass
//! from pumpfun-bonkfun-bot `interfaces/core.py`.

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::platform::Platform;

/// Core token information discovered from on-chain data or event parsing.
///
/// Mirrors the Python reference's `TokenInfo` but uses strong types and
/// avoids optional fields where the protocol guarantees presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    // ── Core identity ───────────────────────────────────────────────
    /// Human-readable token name.
    pub name: String,
    /// Token ticker symbol.
    pub symbol: String,
    /// Metadata URI (off-chain JSON).
    pub uri: String,
    /// SPL token mint address.
    pub mint: Pubkey,

    // ── Platform routing ────────────────────────────────────────────
    /// Which platform launched this token.
    pub platform: Platform,

    // ── Pump.fun bonding curve fields ───────────────────────────────
    /// Bonding curve PDA (present while token is on the curve).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bonding_curve: Option<Pubkey>,

    // ── Quote asset ─────────────────────────────────────────────────
    /// Quote mint: `Pubkey::default()` means native SOL on-chain;
    /// the address provider normalises this to Wrapped SOL for instructions.
    pub quote_mint: Pubkey,
    /// Token program that owns the quote mint (SPL Token or Token-2022).
    pub quote_token_program: Pubkey,

    // ── Base (token) program ────────────────────────────────────────
    /// Token program that owns the base mint (SPL Token or Token-2022).
    pub base_token_program: Pubkey,

    // ── Creator and fee routing ─────────────────────────────────────
    /// Token creator's public key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<Pubkey>,
    /// Creator vault PDA for fee distribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_vault: Option<Pubkey>,

    // ── Flags ───────────────────────────────────────────────────────
    /// Pump.fun "mayhem mode" flag — affects fee recipient selection.
    #[serde(default)]
    pub is_mayhem_mode: bool,
    /// Cashback coin flag. V3 instructions reject cashback coins (error 6094).
    #[serde(default)]
    pub is_cashback_coin: bool,
    /// Holder-reward coin. Creator fee goes to holders, not a wallet.
    #[serde(default)]
    pub is_holder_reward: bool,
    /// Creator fee in basis points (non-zero only on custom-pair coins).
    #[serde(default)]
    pub creator_fee_bps: u16,

    // ── Transfer fee (Token-2022) ───────────────────────────────────
    /// Transfer fee on the base token in basis points. Withheld on every transfer.
    #[serde(default)]
    pub transfer_fee_bps: u16,

    // ── Reserve snapshot ────────────────────────────────────────────
    /// Virtual token reserves from the bonding curve (used for constant product pricing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_token_reserves: Option<u64>,
    /// Virtual quote reserves from the bonding curve (used for pricing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_quote_reserves: Option<u64>,
    /// Real token reserves remaining on the curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub real_token_reserves: Option<u64>,
}

/// Constant: standard SPL token decimal places.
pub const TOKEN_DECIMALS: u8 = 6;
/// Constant: SOL decimal places (lamports).
pub const SOL_DECIMALS: u8 = 9;
/// Constant: USDC decimal places.
pub const USDC_DECIMALS: u8 = 6;
