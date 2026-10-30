//! Fee configuration and computation types.
//!
//! Ported from the architecture plan's fee policy. Fee calculation uses
//! integer arithmetic only — no floating-point token math.

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

/// Platform fee rate tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeTier {
    /// Default RPC path — 10 bps (0.10%) per side.
    #[serde(alias = "Default")]
    Default,
    /// User-supplied RPC — 5 bps (0.05%) per side.
    #[serde(alias = "UserRpc")]
    UserRpc,
}

impl FeeTier {
    /// Fee rate in basis points for this tier.
    pub const fn rate_bps(&self) -> u16 {
        match self {
            FeeTier::Default => 20,
            FeeTier::UserRpc => 10,
        }
    }
}

/// What the fee percentage is applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeBasis {
    /// Fee is calculated on the quote input amount (buy) or expected output (sell).
    QuoteAmount,
}

/// A fully computed fee ready for transaction inclusion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputedFee {
    /// Fee rate in basis points.
    pub rate_bps: u16,
    /// What the rate was applied to.
    pub basis: FeeBasis,
    /// Platform fee wallet receiving the transfer.
    pub recipient: Pubkey,
    /// Exact fee amount in atomic units (lamports for SOL).
    pub amount_atomic: u64,
}

/// Configured platform treasury wallet that must receive platform fees.
pub const CONFIGURED_PLATFORM_TREASURY: Pubkey =
    solana_sdk::pubkey!("AnWnLUpa1ohdgp3WRmhPpxXEusmcEDZNtYTsCM8qq9eH");

/// Fee configuration published by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeConfig {
    /// Config version for forward compatibility.
    pub version: u32,
    /// Platform fee wallet address.
    pub fee_wallet: Pubkey,
    /// Active fee tier for this client.
    pub tier: FeeTier,
}

#[derive(Debug, Error)]
pub enum FeeError {
    #[error("fee amount overflow for input {input_atomic} at {rate_bps} bps")]
    Overflow { input_atomic: u64, rate_bps: u16 },
    #[error("fee amount is zero — trade too small")]
    ZeroFee,
}

/// Compute the platform fee in atomic units.
///
/// Uses integer arithmetic: `fee = input * rate_bps / 10_000`.
/// Returns an error if the result overflows u64 or rounds to zero.
pub fn compute_fee(input_atomic: u64, rate_bps: u16) -> Result<u64, FeeError> {
    let fee = (input_atomic as u128)
        .checked_mul(rate_bps as u128)
        .and_then(|v| v.checked_div(10_000))
        .and_then(|v| u64::try_from(v).ok())
        .ok_or(FeeError::Overflow {
            input_atomic,
            rate_bps,
        })?;

    if fee == 0 {
        return Err(FeeError::ZeroFee);
    }

    Ok(fee)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tier_rate() {
        assert_eq!(FeeTier::Default.rate_bps(), 20);
    }

    #[test]
    fn test_user_rpc_tier_rate() {
        assert_eq!(FeeTier::UserRpc.rate_bps(), 10);
    }

    #[test]
    fn test_fee_tier_accepts_frontend_pascal_case_and_serializes_snake_case() {
        let default: FeeTier = serde_json::from_str("\"Default\"").unwrap();
        let user_rpc: FeeTier = serde_json::from_str("\"UserRpc\"").unwrap();
        assert_eq!(default, FeeTier::Default);
        assert_eq!(user_rpc, FeeTier::UserRpc);
        assert_eq!(serde_json::to_string(&user_rpc).unwrap(), "\"user_rpc\"");
    }

    #[test]
    fn test_compute_fee_default_1_sol() {
        // 1 SOL = 1_000_000_000 lamports, 20 bps = 0.20%
        let fee = compute_fee(1_000_000_000, 20).unwrap();
        assert_eq!(fee, 2_000_000); // 0.002 SOL
    }

    #[test]
    fn test_compute_fee_user_rpc_1_sol() {
        // 1 SOL, 10 bps = 0.10%
        let fee = compute_fee(1_000_000_000, 10).unwrap();
        assert_eq!(fee, 1_000_000); // 0.001 SOL
    }

    #[test]
    fn test_compute_fee_small_amount() {
        // 100 lamports at 10 bps → 0 → ZeroFee error
        let result = compute_fee(100, 10);
        assert!(matches!(result, Err(FeeError::ZeroFee)));
    }

    #[test]
    fn test_compute_fee_exact_threshold() {
        // 10_000 lamports at 10 bps → 10 lamports
        let fee = compute_fee(10_000, 10).unwrap();
        assert_eq!(fee, 10);
    }
}
