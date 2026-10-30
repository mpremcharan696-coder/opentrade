//! PumpSwap AMM Pool state decoding and AMM swap calculations.
//!
//! Handles virtual quote reserves (i128), vault balance addition, dynamic fee tiers, and fee math.

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

/// Anchor discriminator for the PumpSwap `Pool` account:
/// `sha256("account:Pool")[..8]`
pub const PUMPSWAP_POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];

/// Anchor discriminator for FeeConfig: `sha256("account:FeeConfig")[..8]`
pub const PUMP_FEE_CONFIG_DISCRIMINATOR: [u8; 8] = [143, 52, 146, 187, 219, 123, 76, 155];

// Offsets within decoded account data (after 8-byte discriminator)
pub const POOL_BASE_ACCOUNT_OFFSET: usize = 139;
pub const POOL_QUOTE_ACCOUNT_OFFSET: usize = 171;
pub const POOL_COIN_CREATOR_OFFSET: usize = 211;
pub const POOL_MAYHEM_OFFSET: usize = 243;
pub const POOL_CASHBACK_OFFSET: usize = 244;
pub const POOL_VIRTUAL_QUOTE_OFFSET: usize = 245;
pub const POOL_CREATOR_FEE_OFFSET: usize = 261;

#[derive(Error, Debug)]
pub enum PoolError {
    #[error("Account data too short: expected at least {expected} bytes, got {actual}")]
    DataTooShort { expected: usize, actual: usize },
    #[error("Invalid pool discriminator")]
    InvalidDiscriminator,
    #[error("Zero or invalid reserves in AMM")]
    InvalidReserves,
    #[error("Math overflow")]
    MathOverflow,
}

/// Fees structure inside FeeConfig and FeeTier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fees {
    pub lp_fee_bps: u64,
    pub protocol_fee_bps: u64,
    pub creator_fee_bps: u64,
}

/// Dynamic fee tier with market cap activation threshold.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeeTier {
    pub market_cap_lamports_threshold: u128,
    pub fees: Fees,
}

/// Decoded FeeConfig PDA account owned by PUMP_FEE_PROGRAM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PumpSwapFeeConfig {
    pub bump: u8,
    pub admin: Pubkey,
    pub flat_fees: Fees,
    pub fee_tiers: Vec<FeeTier>,
}

impl PumpSwapFeeConfig {
    /// Try parsing a FeeConfig account, verifying discriminator and length bounds.
    pub fn try_from_slice(data: &[u8]) -> Result<Self, PoolError> {
        if data.len() < 8 + 1 + 32 + 24 + 4 {
            return Err(PoolError::DataTooShort {
                expected: 8 + 1 + 32 + 24 + 4,
                actual: data.len(),
            });
        }

        let disc: [u8; 8] = data[0..8]
            .try_into()
            .map_err(|_| PoolError::InvalidDiscriminator)?;
        if disc != PUMP_FEE_CONFIG_DISCRIMINATOR {
            return Err(PoolError::InvalidDiscriminator);
        }

        let mut offset = 8;
        let bump = data[offset];
        offset += 1;

        let admin = Pubkey::new_from_array(
            data[offset..offset + 32]
                .try_into()
                .map_err(|_| PoolError::MathOverflow)?,
        );
        offset += 32;

        let flat_fees = parse_fees(data, &mut offset)?;

        if offset + 4 > data.len() {
            return Err(PoolError::DataTooShort {
                expected: offset + 4,
                actual: data.len(),
            });
        }

        let tier_count = u32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PoolError::MathOverflow)?,
        ) as usize;
        offset += 4;

        if tier_count > 100 {
            return Err(PoolError::MathOverflow);
        }

        let mut fee_tiers = Vec::with_capacity(tier_count);
        for _ in 0..tier_count {
            if offset + 16 > data.len() {
                return Err(PoolError::DataTooShort {
                    expected: offset + 16,
                    actual: data.len(),
                });
            }
            let threshold = u128::from_le_bytes(
                data[offset..offset + 16]
                    .try_into()
                    .map_err(|_| PoolError::MathOverflow)?,
            );
            offset += 16;
            let fees = parse_fees(data, &mut offset)?;
            fee_tiers.push(FeeTier {
                market_cap_lamports_threshold: threshold,
                fees,
            });
        }

        Ok(Self {
            bump,
            admin,
            flat_fees,
            fee_tiers,
        })
    }

    /// Select active (base_fee_bps, creator_fee_bps) based on pool market cap in lamports.
    pub fn select_fee_tier(&self, market_cap_lamports: u64) -> (u64, u64) {
        if self.fee_tiers.is_empty() {
            let base = self.flat_fees.lp_fee_bps.saturating_add(self.flat_fees.protocol_fee_bps);
            return (base, self.flat_fees.creator_fee_bps);
        }

        let first = &self.fee_tiers[0];
        if (market_cap_lamports as u128) < first.market_cap_lamports_threshold {
            let base = first.fees.lp_fee_bps.saturating_add(first.fees.protocol_fee_bps);
            return (base, first.fees.creator_fee_bps);
        }

        for tier in self.fee_tiers.iter().rev() {
            if (market_cap_lamports as u128) >= tier.market_cap_lamports_threshold {
                let base = tier.fees.lp_fee_bps.saturating_add(tier.fees.protocol_fee_bps);
                return (base, tier.fees.creator_fee_bps);
            }
        }

        let base = first.fees.lp_fee_bps.saturating_add(first.fees.protocol_fee_bps);
        (base, first.fees.creator_fee_bps)
    }
}

fn parse_fees(data: &[u8], offset: &mut usize) -> Result<Fees, PoolError> {
    if *offset + 24 > data.len() {
        return Err(PoolError::DataTooShort {
            expected: *offset + 24,
            actual: data.len(),
        });
    }
    let lp_fee_bps = u64::from_le_bytes(
        data[*offset..*offset + 8]
            .try_into()
            .map_err(|_| PoolError::MathOverflow)?,
    );
    *offset += 8;
    let protocol_fee_bps = u64::from_le_bytes(
        data[*offset..*offset + 8]
            .try_into()
            .map_err(|_| PoolError::MathOverflow)?,
    );
    *offset += 8;
    let creator_fee_bps = u64::from_le_bytes(
        data[*offset..*offset + 8]
            .try_into()
            .map_err(|_| PoolError::MathOverflow)?,
    );
    *offset += 8;

    Ok(Fees {
        lp_fee_bps,
        protocol_fee_bps,
        creator_fee_bps,
    })
}

/// Decoded PumpSwap canonical pool metadata and on-chain state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PumpSwapPoolState {
    pub pool_address: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_account: Pubkey,
    pub quote_account: Pubkey,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    /// IMPORTANT: Virtual quote reserves is an i128 on-chain!
    pub virtual_quote_reserves: i128,
    pub creator_fee_bps: u64,
}

impl PumpSwapPoolState {
    /// Parse the canonical pool state from raw account data.
    pub fn try_from_slice(pool_address: Pubkey, data: &[u8]) -> Result<Self, PoolError> {
        let min_len = POOL_VIRTUAL_QUOTE_OFFSET + 16;
        if data.len() < min_len {
            return Err(PoolError::DataTooShort {
                expected: min_len,
                actual: data.len(),
            });
        }

        let base_account = Pubkey::new_from_array(
            data[POOL_BASE_ACCOUNT_OFFSET..POOL_BASE_ACCOUNT_OFFSET + 32]
                .try_into()
                .map_err(|_| PoolError::MathOverflow)?,
        );
        let quote_account = Pubkey::new_from_array(
            data[POOL_QUOTE_ACCOUNT_OFFSET..POOL_QUOTE_ACCOUNT_OFFSET + 32]
                .try_into()
                .map_err(|_| PoolError::MathOverflow)?,
        );
        let coin_creator = Pubkey::new_from_array(
            data[POOL_COIN_CREATOR_OFFSET..POOL_COIN_CREATOR_OFFSET + 32]
                .try_into()
                .map_err(|_| PoolError::MathOverflow)?,
        );

        let is_mayhem_mode = data[POOL_MAYHEM_OFFSET] != 0;
        let is_cashback_coin = data[POOL_CASHBACK_OFFSET] != 0;

        let virtual_quote_reserves = i128::from_le_bytes(
            data[POOL_VIRTUAL_QUOTE_OFFSET..POOL_VIRTUAL_QUOTE_OFFSET + 16]
                .try_into()
                .map_err(|_| PoolError::MathOverflow)?,
        );

        let creator_fee_bps = if data.len() >= POOL_CREATOR_FEE_OFFSET + 8 {
            u64::from_le_bytes(
                data[POOL_CREATOR_FEE_OFFSET..POOL_CREATOR_FEE_OFFSET + 8]
                    .try_into()
                    .unwrap_or_default(),
            )
        } else {
            0
        };

        // base_mint and quote_mint are at offsets 43 and 75
        let base_mint = Pubkey::new_from_array(data[43..75].try_into().unwrap_or_default());
        let quote_mint = Pubkey::new_from_array(data[75..107].try_into().unwrap_or_default());

        Ok(Self {
            pool_address,
            base_mint,
            quote_mint,
            base_account,
            quote_account,
            coin_creator,
            is_mayhem_mode,
            is_cashback_coin,
            virtual_quote_reserves,
            creator_fee_bps,
        })
    }

    /// Calculate effective quote reserve given raw vault balance.
    pub fn effective_quote_reserve(&self, vault_quote_balance: u64) -> Result<u64, PoolError> {
        let total = (vault_quote_balance as i128)
            .checked_add(self.virtual_quote_reserves)
            .ok_or(PoolError::MathOverflow)?;
        if total <= 0 {
            return Err(PoolError::InvalidReserves);
        }
        u64::try_from(total).map_err(|_| PoolError::MathOverflow)
    }

    /// Derive the active fee bps from pool state and optional base fee.
    pub fn derive_active_fee_bps(&self, base_fee_bps: Option<u64>) -> u64 {
        let base = base_fee_bps.unwrap_or(25); // Canonical default AMM base fee is 25 bps (0.25%)
        base.saturating_add(self.creator_fee_bps)
    }

    /// Derive the dynamic fee bps selecting from on-chain FeeConfig tiers based on pool market cap.
    pub fn derive_dynamic_fee_bps(
        &self,
        fee_config: Option<&PumpSwapFeeConfig>,
        base_reserve: u64,
        effective_quote_reserve: u64,
    ) -> u64 {
        if let Some(cfg) = fee_config {
            let mcap = if base_reserve > 0 {
                let scaled = (effective_quote_reserve as u128)
                    .saturating_mul(1_000_000_000_000_000u128)
                    / (base_reserve as u128);
                u64::try_from(scaled).unwrap_or(u64::MAX)
            } else {
                effective_quote_reserve
            };
            let (base_bps, tier_creator_bps) = cfg.select_fee_tier(mcap);
            let active_creator_bps = if self.creator_fee_bps > 0 {
                self.creator_fee_bps
            } else {
                tier_creator_bps
            };
            base_bps.saturating_add(active_creator_bps)
        } else {
            // Canonical fallback if fee config account is absent: 25 bps base + pool creator fee
            25u64.saturating_add(self.creator_fee_bps)
        }
    }
}

/// Constant-product AMM swap: calculate base tokens out for quote in.
///
/// `fee_bps`: typically ~100-300 bps total (LP + protocol + creator fee)
pub fn calculate_amm_buy(
    quote_in: u64,
    base_reserve: u64,
    effective_quote_reserve: u64,
    fee_bps: u64,
) -> Result<u64, PoolError> {
    if base_reserve == 0 || effective_quote_reserve == 0 {
        return Err(PoolError::InvalidReserves);
    }

    let q_in = quote_in as u128;
    let net_q_in = (q_in * (10_000 - fee_bps as u128)) / 10_000;

    let b_res = base_reserve as u128;
    let q_res = effective_quote_reserve as u128;

    let numerator = net_q_in.checked_mul(b_res).ok_or(PoolError::MathOverflow)?;
    let denominator = q_res.checked_add(net_q_in).ok_or(PoolError::MathOverflow)?;

    let tokens_out = numerator / denominator;
    u64::try_from(tokens_out).map_err(|_| PoolError::MathOverflow)
}

/// Constant-product AMM swap: calculate quote out for base tokens in.
pub fn calculate_amm_sell(
    base_in: u64,
    base_reserve: u64,
    effective_quote_reserve: u64,
    fee_bps: u64,
) -> Result<u64, PoolError> {
    if base_reserve == 0 || effective_quote_reserve == 0 {
        return Err(PoolError::InvalidReserves);
    }

    let b_in = base_in as u128;
    let b_res = base_reserve as u128;
    let q_res = effective_quote_reserve as u128;

    let gross_quote_out = (b_in * q_res) / (b_res + b_in);
    let net_quote_out = (gross_quote_out * (10_000 - fee_bps as u128)) / 10_000;

    u64::try_from(net_quote_out).map_err(|_| PoolError::MathOverflow)
}
