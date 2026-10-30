//! Pump.fun bonding curve math, account state decoding, and pricing.
//!
//! Direct Rust port of `pumpfun-bonkfun-bot/src/platforms/pumpfun/curve_manager.py`
//! and verified against the official IDL in `idl/pump_fun_idl.json`.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

/// Anchor discriminator for the `BondingCurve` account:
/// `sha256("account:BondingCurve")[..8]` = `[0x17, 0xb7, 0xf8, 0x37, 0x60, 0xd8, 0xac, 0x60]`
pub const BONDING_CURVE_DISCRIMINATOR: [u8; 8] = [0x17, 0xb7, 0xf8, 0x37, 0x60, 0xd8, 0xac, 0x60];

/// Initial real token reserves on a fresh Pump.fun curve (approx 793.1M tokens with 6 decimals).
pub const INITIAL_REAL_TOKEN_RESERVES: u64 = 793_100_000_000_000;

/// Total token supply for Pump.fun coins (1 billion tokens with 6 decimals).
pub const TOTAL_TOKEN_SUPPLY: u64 = 1_000_000_000_000_000;

#[derive(Error, Debug)]
pub enum CurveError {
    #[error("Account data too short: expected at least {expected} bytes, got {actual}")]
    DataTooShort { expected: usize, actual: usize },
    #[error("Invalid account discriminator")]
    InvalidDiscriminator,
    #[error("Curve is already graduated (complete)")]
    CurveGraduated,
    #[error(
        "Zero or invalid reserves: virtual_token={virtual_token}, virtual_quote={virtual_quote}"
    )]
    InvalidReserves {
        virtual_token: u64,
        virtual_quote: u64,
    },
    #[error("Calculation overflow or division by zero")]
    MathOverflow,
    #[error("Slippage exceeded")]
    SlippageExceeded,
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
}

/// Decoded bonding curve on-chain account state.
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct BondingCurveAccount {
    /// Virtual token reserves (constant product formula x)
    pub virtual_token_reserves: u64,
    /// Virtual quote reserves (constant product formula y: SOL lamports or quote units)
    pub virtual_quote_reserves: u64,
    /// Real token reserves remaining on the curve
    pub real_token_reserves: u64,
    /// Real quote reserves accumulated on the curve
    pub real_quote_reserves: u64,
    /// Total token supply (typically 1B with 6 decimals)
    pub token_total_supply: u64,
    /// True when curve has reached target and migrated to PumpSwap / AMM
    pub complete: bool,
}

impl BondingCurveAccount {
    /// Parse from raw account bytes, validating the 8-byte Anchor discriminator.
    pub fn try_from_slice(data: &[u8]) -> Result<Self, CurveError> {
        if data.len() < 8 + 41 {
            return Err(CurveError::DataTooShort {
                expected: 49,
                actual: data.len(),
            });
        }

        if data[..8] != BONDING_CURVE_DISCRIMINATOR {
            return Err(CurveError::InvalidDiscriminator);
        }

        let mut reader = &data[8..];
        let virtual_token_reserves = u64::from_le_bytes(
            reader[0..8]
                .try_into()
                .map_err(|_| CurveError::MathOverflow)?,
        );
        let virtual_quote_reserves = u64::from_le_bytes(
            reader[8..16]
                .try_into()
                .map_err(|_| CurveError::MathOverflow)?,
        );
        let real_token_reserves = u64::from_le_bytes(
            reader[16..24]
                .try_into()
                .map_err(|_| CurveError::MathOverflow)?,
        );
        let real_quote_reserves = u64::from_le_bytes(
            reader[24..32]
                .try_into()
                .map_err(|_| CurveError::MathOverflow)?,
        );
        let token_total_supply = u64::from_le_bytes(
            reader[32..40]
                .try_into()
                .map_err(|_| CurveError::MathOverflow)?,
        );
        let complete = reader[40] != 0;

        Ok(Self {
            virtual_token_reserves,
            virtual_quote_reserves,
            real_token_reserves,
            real_quote_reserves,
            token_total_supply,
            complete,
        })
    }

    /// Calculate expected token output when buying with `amount_in_quote` (lamports or atomic quote).
    pub fn calculate_buy_tokens_out(&self, amount_in_quote: u64) -> Result<u64, CurveError> {
        if self.complete {
            return Err(CurveError::CurveGraduated);
        }
        calculate_buy_tokens_out(
            amount_in_quote,
            self.virtual_token_reserves,
            self.virtual_quote_reserves,
        )
    }

    /// Calculate expected quote output (lamports) when selling `amount_in_tokens`.
    pub fn calculate_sell_quote_out(&self, amount_in_tokens: u64) -> Result<u64, CurveError> {
        if self.complete {
            return Err(CurveError::CurveGraduated);
        }
        calculate_sell_quote_out(
            amount_in_tokens,
            self.virtual_token_reserves,
            self.virtual_quote_reserves,
        )
    }

    /// Spot price in quote currency per 1.0 whole token (accounting for standard 6 decimals).
    pub fn spot_price(&self, quote_decimals: u8) -> Result<f64, CurveError> {
        if self.complete {
            return Err(CurveError::CurveGraduated);
        }
        calculate_spot_price(
            self.virtual_token_reserves,
            self.virtual_quote_reserves,
            quote_decimals,
        )
    }

    /// Percentage completion of the bonding curve (0.0 to 100.0%).
    pub fn progress_percent(&self) -> f64 {
        if self.complete {
            return 100.0;
        }
        if self.real_token_reserves >= INITIAL_REAL_TOKEN_RESERVES {
            return 0.0;
        }
        let sold = INITIAL_REAL_TOKEN_RESERVES - self.real_token_reserves;
        let pct = (sold as f64 / INITIAL_REAL_TOKEN_RESERVES as f64) * 100.0;
        pct.clamp(0.0, 100.0)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Pure Math Functions
// ────────────────────────────────────────────────────────────────────────────

/// Calculate tokens received given quote input.
///
/// Formula: `tokens_out = (amount_in * virtual_token_reserves) / (virtual_quote_reserves + amount_in)`
pub fn calculate_buy_tokens_out(
    amount_in_quote: u64,
    virtual_token_reserves: u64,
    virtual_quote_reserves: u64,
) -> Result<u64, CurveError> {
    if virtual_token_reserves == 0 || virtual_quote_reserves == 0 {
        return Err(CurveError::InvalidReserves {
            virtual_token: virtual_token_reserves,
            virtual_quote: virtual_quote_reserves,
        });
    }

    let amount_in = amount_in_quote as u128;
    let v_token = virtual_token_reserves as u128;
    let v_quote = virtual_quote_reserves as u128;

    let numerator = amount_in
        .checked_mul(v_token)
        .ok_or(CurveError::MathOverflow)?;
    let denominator = v_quote
        .checked_add(amount_in)
        .ok_or(CurveError::MathOverflow)?;

    if denominator == 0 {
        return Err(CurveError::MathOverflow);
    }

    let tokens_out = numerator / denominator;
    u64::try_from(tokens_out).map_err(|_| CurveError::MathOverflow)
}

/// Calculate quote needed to purchase exact `tokens_out`.
///
/// Formula: `quote_in = (tokens_out * virtual_quote_reserves) / (virtual_token_reserves - tokens_out) + 1`
pub fn calculate_buy_quote_in(
    tokens_out: u64,
    virtual_token_reserves: u64,
    virtual_quote_reserves: u64,
) -> Result<u64, CurveError> {
    if tokens_out >= virtual_token_reserves {
        return Err(CurveError::MathOverflow);
    }

    let t_out = tokens_out as u128;
    let v_quote = virtual_quote_reserves as u128;
    let v_token = virtual_token_reserves as u128;

    let numerator = t_out.checked_mul(v_quote).ok_or(CurveError::MathOverflow)?;
    let denominator = v_token.checked_sub(t_out).ok_or(CurveError::MathOverflow)?;

    if denominator == 0 {
        return Err(CurveError::MathOverflow);
    }

    let quote_in = (numerator / denominator) + 1;
    u64::try_from(quote_in).map_err(|_| CurveError::MathOverflow)
}

/// Calculate quote received when selling `amount_in_tokens`.
///
/// Formula: `quote_out = (amount_in * virtual_quote_reserves) / (virtual_token_reserves + amount_in)`
pub fn calculate_sell_quote_out(
    amount_in_tokens: u64,
    virtual_token_reserves: u64,
    virtual_quote_reserves: u64,
) -> Result<u64, CurveError> {
    if virtual_token_reserves == 0 || virtual_quote_reserves == 0 {
        return Err(CurveError::InvalidReserves {
            virtual_token: virtual_token_reserves,
            virtual_quote: virtual_quote_reserves,
        });
    }

    let amount_in = amount_in_tokens as u128;
    let v_quote = virtual_quote_reserves as u128;
    let v_token = virtual_token_reserves as u128;

    let numerator = amount_in
        .checked_mul(v_quote)
        .ok_or(CurveError::MathOverflow)?;
    let denominator = v_token
        .checked_add(amount_in)
        .ok_or(CurveError::MathOverflow)?;

    if denominator == 0 {
        return Err(CurveError::MathOverflow);
    }

    let quote_out = numerator / denominator;
    u64::try_from(quote_out).map_err(|_| CurveError::MathOverflow)
}

/// Spot price in quote units per token unit.
pub fn calculate_spot_price(
    virtual_token_reserves: u64,
    virtual_quote_reserves: u64,
    quote_decimals: u8,
) -> Result<f64, CurveError> {
    if virtual_token_reserves == 0 || virtual_quote_reserves == 0 {
        return Err(CurveError::InvalidReserves {
            virtual_token: virtual_token_reserves,
            virtual_quote: virtual_quote_reserves,
        });
    }

    let token_scale = 10f64.powi(6); // 6 decimals for pump tokens
    let quote_scale = 10f64.powi(quote_decimals as i32);

    let price = (virtual_quote_reserves as f64 / virtual_token_reserves as f64)
        * (token_scale / quote_scale);
    Ok(price)
}

/// Apply slippage tolerance to calculate minimum acceptable output.
///
/// `slippage_bps`: 100 = 1.0%
pub fn apply_slippage_min(expected: u64, slippage_bps: u16) -> u64 {
    let bps = (slippage_bps as u64).min(10_000);
    let factor = 10_000 - bps;
    ((expected as u128 * factor as u128) / 10_000) as u64
}

/// Apply slippage tolerance to calculate maximum acceptable spend.
pub fn apply_slippage_max(budget: u64, slippage_bps: u16) -> u64 {
    let bps = slippage_bps as u64;
    let factor = 10_000 + bps;
    ((budget as u128 * factor as u128) / 10_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bonding_curve_buy_math() {
        let v_token = 1_073_000_000_000_000u64; // ~1.073B tokens
        let v_quote = 30_000_000_000u64; // 30 SOL

        // Spend 1 SOL (1_000_000_000 lamports)
        let tokens_out = calculate_buy_tokens_out(1_000_000_000, v_token, v_quote).unwrap();
        assert!(tokens_out > 0);
        // tokens_out = 1 * 1.073B / 31 = ~34.6M tokens
        assert!(tokens_out > 30_000_000_000_000);
        assert!(tokens_out < 40_000_000_000_000);
    }

    #[test]
    fn test_bonding_curve_sell_math() {
        let v_token = 1_073_000_000_000_000u64;
        let v_quote = 30_000_000_000u64;

        let tokens_to_sell = 34_612_903_225_806u64;
        let sol_out = calculate_sell_quote_out(tokens_to_sell, v_token, v_quote).unwrap();
        assert!(sol_out > 900_000_000);
        assert!(sol_out < 1_000_000_000); // slightly less than 1 SOL due to constant product curve
    }

    #[test]
    fn test_slippage_calculation() {
        let expected = 1_000_000;
        // 1% slippage = 100 bps
        let min_out = apply_slippage_min(expected, 100);
        assert_eq!(min_out, 990_000);

        let max_in = apply_slippage_max(expected, 100);
        assert_eq!(max_in, 1_010_000);
    }
}
