//! PumpSwap AMM Solana instruction builders for buy/sell operations.
//!
//! Direct Rust port of `pumpswap_market.py` verified against `pump_swap_idl.json`.

use protocol_pump::addresses::{
    ASSOCIATED_TOKEN_PROGRAM, SYSTEM_PROGRAM, is_sol_paired, pick_buyback_fee_recipient,
    pick_fee_recipient,
};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;

use crate::addresses::{
    PUMP_AMM_PROGRAM, PUMP_FEE_PROGRAM, find_creator_vault, find_event_authority, find_fee_config,
    find_global_config, find_global_volume_accumulator, find_pool_v2, find_user_volume_accumulator,
};
use crate::pool::PumpSwapPoolState;

/// Discriminator for PumpSwap `buy`: `[102, 6, 61, 18, 1, 218, 235, 234]`
pub const PUMPSWAP_BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];

/// Discriminator for PumpSwap `sell`: `[51, 230, 133, 164, 1, 127, 131, 173]`
pub const PUMPSWAP_SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

/// Build a PumpSwap AMM `buy` instruction sequence.
///
/// Parameters:
/// - `pool`: The canonical PumpSwap pool state
/// - `user`: The buyer's wallet pubkey
/// - `base_amount_out`: Exact minimum base tokens to receive
/// - `max_quote_in`: Maximum quote tokens (or lamports) to spend (slippage limit)
/// - `base_program`: Token program owning base token (SPL Token or Token-2022)
/// - `quote_program`: Token program owning quote token
pub fn build_pumpswap_buy_instructions(
    pool: &PumpSwapPoolState,
    user: &Pubkey,
    base_amount_out: u64,
    max_quote_in: u64,
    base_program: &Pubkey,
    quote_program: &Pubkey,
) -> Vec<Instruction> {
    let mut ixs = Vec::with_capacity(6);

    let user_base_ata =
        get_associated_token_address_with_program_id(user, &pool.base_mint, base_program);
    let user_quote_ata =
        get_associated_token_address_with_program_id(user, &pool.quote_mint, quote_program);

    // 1. If quote is WSOL/native SOL, wrap SOL into user's WSOL ATA
    if is_sol_paired(&pool.quote_mint) {
        // Idempotently create user quote ATA (WSOL)
        ixs.push(create_associated_token_account_idempotent(
            user,
            user,
            &pool.quote_mint,
            quote_program,
        ));
        // Transfer native SOL to WSOL ATA
        ixs.push(solana_sdk::system_instruction::transfer(
            user,
            &user_quote_ata,
            max_quote_in,
        ));
        // Sync native balance
        ixs.push(
            spl_token::instruction::sync_native(
                quote_program,
                &user_quote_ata,
            )
            .expect("Valid sync_native instruction"),
        );
    } else {
        // Idempotently create user quote ATA if quote is not native SOL
        ixs.push(create_associated_token_account_idempotent(
            user,
            user,
            &pool.quote_mint,
            quote_program,
        ));
    }

    // 2. Idempotently create user base ATA for receiving purchased tokens
    ixs.push(create_associated_token_account_idempotent(
        user,
        user,
        &pool.base_mint,
        base_program,
    ));

    let (global_config, _) = find_global_config();
    let (event_authority, _) = find_event_authority();
    let (fee_config, _) = find_fee_config();

    let fee_recipient = pick_fee_recipient(pool.is_mayhem_mode);
    let fee_recipient_quote_ata = get_associated_token_address_with_program_id(
        &fee_recipient,
        &pool.quote_mint,
        quote_program,
    );

    let (creator_vault, _) = find_creator_vault(&pool.coin_creator);
    let creator_vault_quote_ata = get_associated_token_address_with_program_id(
        &creator_vault,
        &pool.quote_mint,
        quote_program,
    );

    let (global_vol_acc, _) = find_global_volume_accumulator();
    let (user_vol_acc, _) = find_user_volume_accumulator(user);

    let buyback_recipient = pick_buyback_fee_recipient();
    let buyback_quote_ata = get_associated_token_address_with_program_id(
        &buyback_recipient,
        &pool.quote_mint,
        quote_program,
    );

    let mut accounts = vec![
        AccountMeta::new(pool.pool_address, false),
        AccountMeta::new(*user, true), // signer
        AccountMeta::new_readonly(global_config, false),
        AccountMeta::new_readonly(pool.base_mint, false),
        AccountMeta::new_readonly(pool.quote_mint, false),
        AccountMeta::new(user_base_ata, false),
        AccountMeta::new(user_quote_ata, false),
        AccountMeta::new(pool.base_account, false),
        AccountMeta::new(pool.quote_account, false),
        AccountMeta::new_readonly(fee_recipient, false),
        AccountMeta::new(fee_recipient_quote_ata, false),
        AccountMeta::new_readonly(*base_program, false),
        AccountMeta::new_readonly(*quote_program, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new_readonly(PUMP_AMM_PROGRAM, false),
        AccountMeta::new(creator_vault_quote_ata, false),
        AccountMeta::new_readonly(creator_vault, false),
        AccountMeta::new_readonly(global_vol_acc, false),
        AccountMeta::new(user_vol_acc, false),
        AccountMeta::new_readonly(fee_config, false),
        AccountMeta::new_readonly(PUMP_FEE_PROGRAM, false),
    ];

    // Conditional: cashback accumulator
    if pool.is_cashback_coin {
        let (accumulator, _) = find_user_volume_accumulator(user);
        let acc_quote_ata = get_associated_token_address_with_program_id(
            &accumulator,
            &pool.quote_mint,
            quote_program,
        );
        accounts.push(AccountMeta::new(acc_quote_ata, false));
    }

    // Conditional: canonical pool with creator requires pool-v2 PDA
    if pool.coin_creator != Pubkey::default() {
        let (pool_v2, _) = find_pool_v2(&pool.base_mint);
        accounts.push(AccountMeta::new_readonly(pool_v2, false));
    }

    // Trailing buyback recipient accounts
    accounts.push(AccountMeta::new_readonly(buyback_recipient, false));
    accounts.push(AccountMeta::new(buyback_quote_ata, false));

    // Data: Discriminator (8) + base_amount_out (8) + max_quote_amount_in (8) + track_volume bool (1)
    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&PUMPSWAP_BUY_DISCRIMINATOR);
    data.extend_from_slice(&base_amount_out.to_le_bytes());
    data.extend_from_slice(&max_quote_in.to_le_bytes());
    data.push(0u8); // track_volume: false

    ixs.push(Instruction {
        program_id: PUMP_AMM_PROGRAM,
        accounts,
        data,
    });

    // 3. For SOL-quoted pools, unwrap any excess WSOL back to native SOL in the user's wallet
    if is_sol_paired(&pool.quote_mint) {
        ixs.push(
            spl_token::instruction::close_account(
                quote_program,
                &user_quote_ata,
                user, // destination: native SOL credited to user wallet
                user, // authority
                &[],
            )
            .expect("Valid close_account instruction"),
        );
    }

    ixs
}

/// Build a PumpSwap AMM `sell` instruction sequence.
///
/// Parameters:
/// - `pool`: The canonical PumpSwap pool state
/// - `user`: The seller's wallet pubkey
/// - `amount_in`: Amount of base tokens to sell
/// - `min_quote_out`: Minimum quote tokens (or lamports) to receive (slippage limit)
/// - `base_program`: Token program owning base token (SPL Token or Token-2022)
/// - `quote_program`: Token program owning quote token
pub fn build_pumpswap_sell_instructions(
    pool: &PumpSwapPoolState,
    user: &Pubkey,
    amount_in: u64,
    min_quote_out: u64,
    base_program: &Pubkey,
    quote_program: &Pubkey,
) -> Vec<Instruction> {
    let mut ixs = Vec::with_capacity(3);

    let user_base_ata =
        get_associated_token_address_with_program_id(user, &pool.base_mint, base_program);
    let user_quote_ata =
        get_associated_token_address_with_program_id(user, &pool.quote_mint, quote_program);

    // Idempotently create user quote ATA to receive swap output
    ixs.push(create_associated_token_account_idempotent(
        user,
        user,
        &pool.quote_mint,
        quote_program,
    ));

    let (global_config, _) = find_global_config();
    let (event_authority, _) = find_event_authority();
    let (fee_config, _) = find_fee_config();

    let fee_recipient = pick_fee_recipient(pool.is_mayhem_mode);
    let fee_recipient_quote_ata = get_associated_token_address_with_program_id(
        &fee_recipient,
        &pool.quote_mint,
        quote_program,
    );

    let buyback_recipient = pick_buyback_fee_recipient();
    let buyback_quote_ata = get_associated_token_address_with_program_id(
        &buyback_recipient,
        &pool.quote_mint,
        quote_program,
    );

    let (creator_vault, _) = find_creator_vault(&pool.coin_creator);
    let creator_vault_quote_ata = get_associated_token_address_with_program_id(
        &creator_vault,
        &pool.quote_mint,
        quote_program,
    );

    let mut accounts = vec![
        AccountMeta::new(pool.pool_address, false),
        AccountMeta::new(*user, true), // signer
        AccountMeta::new_readonly(global_config, false),
        AccountMeta::new_readonly(pool.base_mint, false),
        AccountMeta::new_readonly(pool.quote_mint, false),
        AccountMeta::new(user_base_ata, false),
        AccountMeta::new(user_quote_ata, false),
        AccountMeta::new(pool.base_account, false),
        AccountMeta::new(pool.quote_account, false),
        AccountMeta::new_readonly(fee_recipient, false),
        AccountMeta::new(fee_recipient_quote_ata, false),
        AccountMeta::new_readonly(*base_program, false),
        AccountMeta::new_readonly(*quote_program, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new_readonly(PUMP_AMM_PROGRAM, false),
        AccountMeta::new(creator_vault_quote_ata, false),
        AccountMeta::new_readonly(creator_vault, false),
        AccountMeta::new_readonly(fee_config, false),
        AccountMeta::new_readonly(PUMP_FEE_PROGRAM, false),
    ];

    // Conditional: cashback accumulator
    if pool.is_cashback_coin {
        let (accumulator, _) = find_user_volume_accumulator(user);
        let acc_quote_ata = get_associated_token_address_with_program_id(
            &accumulator,
            &pool.quote_mint,
            quote_program,
        );
        accounts.push(AccountMeta::new(acc_quote_ata, false));
        accounts.push(AccountMeta::new(accumulator, false));
    }

    // Conditional: canonical pool with creator requires pool-v2 PDA
    if pool.coin_creator != Pubkey::default() {
        let (pool_v2, _) = find_pool_v2(&pool.base_mint);
        accounts.push(AccountMeta::new_readonly(pool_v2, false));
    }

    // Trailing buyback recipient accounts
    accounts.push(AccountMeta::new_readonly(buyback_recipient, false));
    accounts.push(AccountMeta::new(buyback_quote_ata, false));

    // Data: Discriminator (8) + amount_in (8) + min_quote_out (8)
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&PUMPSWAP_SELL_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_quote_out.to_le_bytes());

    ixs.push(Instruction {
        program_id: PUMP_AMM_PROGRAM,
        accounts,
        data,
    });

    // 3. For SOL-quoted pools, unwrap the WSOL proceeds directly into native SOL in the user's wallet.
    // This delivers the swap proceeds to the wallet before platform fee deduction and reclaims ATA rent.
    if is_sol_paired(&pool.quote_mint) {
        ixs.push(
            spl_token::instruction::close_account(
                quote_program,
                &user_quote_ata,
                user, // destination: native SOL credited to user wallet
                user, // authority
                &[],
            )
            .expect("Valid close_account instruction"),
        );
    }

    ixs
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey;

    #[test]
    fn test_build_pumpswap_buy_and_sell_layouts() {
        let pool = PumpSwapPoolState {
            pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
            base_mint: pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R"),
            quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
            base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
            quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
            coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            virtual_quote_reserves: 30_000_000_000,
            creator_fee_bps: 0,
        };

        let user = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
        let spl_token = spl_token::id();

        // 1. Test Buy Instructions
        let buy_ixs = build_pumpswap_buy_instructions(
            &pool,
            &user,
            1_000_000,
            100_000_000,
            &spl_token,
            &spl_token,
        );
        // Includes WSOL ATA + wrap SOL + sync_native + Base ATA + AMM buy + unwrap WSOL
        assert_eq!(buy_ixs.len(), 6);
        let buy_amm_ix = &buy_ixs[4];
        assert_eq!(buy_amm_ix.program_id, PUMP_AMM_PROGRAM);
        assert_eq!(buy_amm_ix.accounts.len(), 26);
        assert_eq!(&buy_amm_ix.data[0..8], &PUMPSWAP_BUY_DISCRIMINATOR);
        assert_eq!(buy_amm_ix.accounts[0].pubkey, pool.pool_address);
        assert_eq!(buy_amm_ix.accounts[1].pubkey, user);
        assert!(buy_amm_ix.accounts[1].is_signer);

        // 2. Test Sell Instructions
        let sell_ixs = build_pumpswap_sell_instructions(
            &pool,
            &user,
            500_000,
            50_000_000,
            &spl_token,
            &spl_token,
        );
        // 3 instructions: 1. Create WSOL ATA + 2. AMM Sell + 3. Unwrap WSOL to native SOL in user wallet
        assert_eq!(sell_ixs.len(), 3);
        let sell_ata_ix = &sell_ixs[0];
        assert_eq!(sell_ata_ix.program_id, ASSOCIATED_TOKEN_PROGRAM);

        let sell_amm_ix = &sell_ixs[1];
        assert_eq!(sell_amm_ix.program_id, PUMP_AMM_PROGRAM);
        assert_eq!(&sell_amm_ix.data[0..8], &PUMPSWAP_SELL_DISCRIMINATOR);
        assert_eq!(sell_amm_ix.accounts[1].pubkey, user);
        assert!(sell_amm_ix.accounts[1].is_signer);

        let unwrap_ix = &sell_ixs[2];
        assert_eq!(unwrap_ix.program_id, spl_token);
        assert_eq!(unwrap_ix.data, vec![9]); // CloseAccount instruction
        assert_eq!(unwrap_ix.accounts[1].pubkey, user); // Destination: user wallet
    }
}

