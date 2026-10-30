//! PumpSwap AMM addresses and PDA derivation.
//!
//! Verified against `pAMMBay...` program specifications and Python reference.

use protocol_pump::addresses::PUMP_PROGRAM;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// PumpSwap AMM Program ID (`pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`).
pub const PUMP_AMM_PROGRAM: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");

/// Fee program (`pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ`).
pub const PUMP_FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

/// Derive a PDA under the PumpSwap AMM program.
pub fn find_amm_pda(seeds: &[&[u8]]) -> (Pubkey, u8) {
    Pubkey::find_program_address(seeds, &PUMP_AMM_PROGRAM)
}

/// AMM GlobalConfig PDA: `["global_config"]`.
pub fn find_global_config() -> (Pubkey, u8) {
    find_amm_pda(&[b"global_config"])
}

/// AMM EventAuthority PDA: `["__event_authority"]`.
pub fn find_event_authority() -> (Pubkey, u8) {
    find_amm_pda(&[b"__event_authority"])
}

/// AMM FeeConfig PDA under PUMP_FEE_PROGRAM: `["fee_config", PUMP_AMM_PROGRAM]`.
pub fn find_fee_config() -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"fee_config", PUMP_AMM_PROGRAM.as_ref()],
        &PUMP_FEE_PROGRAM,
    )
}

/// Pool authority PDA under the original PUMP_PROGRAM: `["pool-authority", base_mint]`.
pub fn find_pool_authority(base_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pool-authority", base_mint.as_ref()], &PUMP_PROGRAM)
}

/// Canonical pool address for a graduated token.
///
/// Seeds: `["pool", index: u16(0), pool_authority, base_mint, quote_mint]`.
pub fn find_canonical_pool(base_mint: &Pubkey, quote_mint: &Pubkey) -> (Pubkey, u8) {
    let (pool_authority, _) = find_pool_authority(base_mint);
    let index_bytes = 0u16.to_le_bytes();
    find_amm_pda(&[
        b"pool",
        &index_bytes,
        pool_authority.as_ref(),
        base_mint.as_ref(),
        quote_mint.as_ref(),
    ])
}

/// Creator vault PDA: `["creator_vault", coin_creator]`.
pub fn find_creator_vault(coin_creator: &Pubkey) -> (Pubkey, u8) {
    find_amm_pda(&[b"creator_vault", coin_creator.as_ref()])
}

/// Pool V2 PDA: `["pool-v2", base_mint]`.
pub fn find_pool_v2(base_mint: &Pubkey) -> (Pubkey, u8) {
    find_amm_pda(&[b"pool-v2", base_mint.as_ref()])
}

/// User volume accumulator PDA: `["user_volume_accumulator", user]`.
pub fn find_user_volume_accumulator(user: &Pubkey) -> (Pubkey, u8) {
    find_amm_pda(&[b"user_volume_accumulator", user.as_ref()])
}

/// Global volume accumulator PDA: `["global_volume_accumulator"]`.
pub fn find_global_volume_accumulator() -> (Pubkey, u8) {
    find_amm_pda(&[b"global_volume_accumulator"])
}
