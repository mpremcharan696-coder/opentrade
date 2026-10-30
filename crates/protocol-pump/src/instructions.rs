//! Pump.fun Solana instruction builders for buy/sell operations.
//!
//! Direct Rust port of `pumpfun-bonkfun-bot/src/platforms/pumpfun/instruction_builder.py`
//! verified against `idl/pump_fun_idl.json`.

use shared_types::token::TokenInfo;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;

use crate::addresses::{PUMP_PROGRAM, ResolvedV2Accounts, is_sol_paired, resolve_v2_accounts};

/// Discriminator for `buy` (legacy): `sha256("global:buy")[..8]`
pub const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];

/// Discriminator for `buy_v2`: `sha256("global:buy_v2")[..8]`
pub const BUY_V2_DISCRIMINATOR: [u8; 8] = [184, 23, 238, 97, 103, 197, 211, 61];

/// Discriminator for `sell` (legacy): `sha256("global:sell")[..8]`
pub const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

/// Discriminator for `sell_v2`: `sha256("global:sell_v2")[..8]`
pub const SELL_V2_DISCRIMINATOR: [u8; 8] = [93, 246, 130, 60, 231, 233, 64, 178];

// ────────────────────────────────────────────────────────────────────────────
// Buy V2 Builder
// ────────────────────────────────────────────────────────────────────────────

/// Build `buy_v2` instructions including idempotent ATA creation.
///
/// Parameters:
/// - `token_info`: Token metadata and curve info
/// - `user`: Signer wallet public key
/// - `amount_tokens_out`: Exact or minimum base tokens to receive
/// - `max_quote_in`: Maximum quote currency (lamports for SOL) to spend (slippage protection)
pub fn build_buy_v2_instructions(
    token_info: &TokenInfo,
    user: &Pubkey,
    amount_tokens_out: u64,
    max_quote_in: u64,
) -> Vec<Instruction> {
    let accounts = resolve_v2_accounts(token_info, user);
    let mut ixs = Vec::with_capacity(3);

    // 1. Base-token ATA for the buyer (idempotent create)
    ixs.push(create_associated_token_account_idempotent(
        user,
        user,
        &accounts.base_mint,
        &accounts.base_token_program,
    ));

    // 2. Quote ATA for the buyer if quote is NOT native SOL
    if !is_sol_paired(&accounts.quote_mint) {
        ixs.push(create_associated_token_account_idempotent(
            user,
            user,
            &accounts.quote_mint,
            &accounts.quote_token_program,
        ));
    }

    // 3. buy_v2 instruction
    // Args: amount: u64 (base tokens out), max_sol_cost: u64 (max quote in)
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&BUY_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount_tokens_out.to_le_bytes());
    data.extend_from_slice(&max_quote_in.to_le_bytes());

    let account_metas = vec![
        AccountMeta::new_readonly(accounts.global, false),
        AccountMeta::new_readonly(accounts.base_mint, false),
        AccountMeta::new_readonly(accounts.quote_mint, false),
        AccountMeta::new_readonly(accounts.base_token_program, false),
        AccountMeta::new_readonly(accounts.quote_token_program, false),
        AccountMeta::new_readonly(accounts.associated_token_program, false),
        AccountMeta::new(accounts.fee_recipient, false),
        AccountMeta::new(accounts.associated_quote_fee_recipient, false),
        AccountMeta::new(accounts.buyback_fee_recipient, false),
        AccountMeta::new(accounts.associated_quote_buyback_fee_recipient, false),
        AccountMeta::new(accounts.bonding_curve, false),
        AccountMeta::new(accounts.associated_base_bonding_curve, false),
        AccountMeta::new(accounts.associated_quote_bonding_curve, false),
        AccountMeta::new(*user, true), // signer
        AccountMeta::new(accounts.associated_base_user, false),
        AccountMeta::new(accounts.associated_quote_user, false),
        AccountMeta::new(accounts.creator_vault, false),
        AccountMeta::new(accounts.associated_creator_vault, false),
        AccountMeta::new_readonly(accounts.sharing_config, false),
        AccountMeta::new_readonly(accounts.global_volume_accumulator, false),
        AccountMeta::new(accounts.user_volume_accumulator, false),
        AccountMeta::new(accounts.associated_user_volume_accumulator, false),
        AccountMeta::new_readonly(accounts.fee_config, false),
        AccountMeta::new_readonly(accounts.fee_program, false),
        AccountMeta::new_readonly(accounts.system_program, false),
        AccountMeta::new_readonly(accounts.event_authority, false),
        AccountMeta::new_readonly(accounts.program, false),
    ];

    ixs.push(Instruction {
        program_id: accounts.program,
        accounts: account_metas,
        data,
    });

    ixs
}

// ────────────────────────────────────────────────────────────────────────────
// Sell V2 Builder
// ────────────────────────────────────────────────────────────────────────────

/// Build `sell_v2` instructions including idempotent ATA creation.
///
/// Parameters:
/// - `token_info`: Token metadata and curve info
/// - `user`: Signer wallet public key
/// - `amount_tokens_in`: Amount of base tokens to sell
/// - `min_quote_out`: Minimum quote currency (lamports) to receive (slippage protection)
pub fn build_sell_v2_instructions(
    token_info: &TokenInfo,
    user: &Pubkey,
    amount_tokens_in: u64,
    min_quote_out: u64,
) -> Vec<Instruction> {
    let accounts = resolve_v2_accounts(token_info, user);
    let mut ixs = Vec::with_capacity(2);

    // 1. Non-SOL quote ATA if needed
    if !is_sol_paired(&accounts.quote_mint) {
        ixs.push(create_associated_token_account_idempotent(
            user,
            user,
            &accounts.quote_mint,
            &accounts.quote_token_program,
        ));
    }

    // 2. sell_v2 instruction
    // Args: amount: u64 (base tokens in), min_sol_output: u64 (min quote out)
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&SELL_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount_tokens_in.to_le_bytes());
    data.extend_from_slice(&min_quote_out.to_le_bytes());

    // sell_v2 is buy_v2 layout without global_volume_accumulator (26 accounts)
    let account_metas = vec![
        AccountMeta::new_readonly(accounts.global, false),
        AccountMeta::new_readonly(accounts.base_mint, false),
        AccountMeta::new_readonly(accounts.quote_mint, false),
        AccountMeta::new_readonly(accounts.base_token_program, false),
        AccountMeta::new_readonly(accounts.quote_token_program, false),
        AccountMeta::new_readonly(accounts.associated_token_program, false),
        AccountMeta::new(accounts.fee_recipient, false),
        AccountMeta::new(accounts.associated_quote_fee_recipient, false),
        AccountMeta::new(accounts.buyback_fee_recipient, false),
        AccountMeta::new(accounts.associated_quote_buyback_fee_recipient, false),
        AccountMeta::new(accounts.bonding_curve, false),
        AccountMeta::new(accounts.associated_base_bonding_curve, false),
        AccountMeta::new(accounts.associated_quote_bonding_curve, false),
        AccountMeta::new(*user, true), // signer
        AccountMeta::new(accounts.associated_base_user, false),
        AccountMeta::new(accounts.associated_quote_user, false),
        AccountMeta::new(accounts.creator_vault, false),
        AccountMeta::new(accounts.associated_creator_vault, false),
        AccountMeta::new_readonly(accounts.sharing_config, false),
        // Notice: global_volume_accumulator is omitted for sell_v2!
        AccountMeta::new(accounts.user_volume_accumulator, false),
        AccountMeta::new(accounts.associated_user_volume_accumulator, false),
        AccountMeta::new_readonly(accounts.fee_config, false),
        AccountMeta::new_readonly(accounts.fee_program, false),
        AccountMeta::new_readonly(accounts.system_program, false),
        AccountMeta::new_readonly(accounts.event_authority, false),
        AccountMeta::new_readonly(accounts.program, false),
    ];

    ixs.push(Instruction {
        program_id: accounts.program,
        accounts: account_metas,
        data,
    });

    ixs
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::platform::Platform;

    fn sample_token() -> TokenInfo {
        TokenInfo {
            name: "Test Pump".to_string(),
            symbol: "TEST".to_string(),
            uri: "https://test.com".to_string(),
            mint: Pubkey::new_unique(),
            platform: Platform::PumpFun,
            bonding_curve: None,
            quote_mint: Pubkey::default(),
            quote_token_program: spl_token::id(),
            base_token_program: spl_token::id(),
            creator: Some(Pubkey::new_unique()),
            creator_vault: None,
            is_mayhem_mode: false,
            is_cashback_coin: false,
            is_holder_reward: false,
            creator_fee_bps: 0,
            transfer_fee_bps: 0,
            virtual_token_reserves: None,
            virtual_quote_reserves: None,
            real_token_reserves: None,
        }
    }

    #[test]
    fn test_build_buy_v2_structure() {
        let user = Pubkey::new_unique();
        let token = sample_token();
        let ixs = build_buy_v2_instructions(&token, &user, 1_000_000, 50_000_000);

        // Expect ATA create + buy_v2
        assert_eq!(ixs.len(), 2);
        let buy_ix = &ixs[1];
        assert_eq!(buy_ix.accounts.len(), 27);
        assert_eq!(&buy_ix.data[..8], &BUY_V2_DISCRIMINATOR);
    }

    #[test]
    fn test_build_sell_v2_structure() {
        let user = Pubkey::new_unique();
        let token = sample_token();
        let ixs = build_sell_v2_instructions(&token, &user, 1_000_000, 45_000_000);

        // For SOL quote, no quote ATA creation needed
        assert_eq!(ixs.len(), 1);
        let sell_ix = &ixs[0];
        assert_eq!(sell_ix.accounts.len(), 26);
        assert_eq!(&sell_ix.data[..8], &SELL_V2_DISCRIMINATOR);
    }
}
