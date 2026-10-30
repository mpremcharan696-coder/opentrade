//! Instruction builders for OKX DEX Router V1 on Solana.

use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use thiserror::Error;

use crate::addresses::{OKX_ROUTER_PROGRAM_ID, derive_sa_authority};
use crate::types::{Dex, Route, SwapArgs, SwapV3InstructionData};

#[derive(Error, Debug)]
pub enum OkxRouterError {
    #[error("Serialization failed: {0}")]
    Serialization(String),
}

/// Accounts required to invoke `swap_v3` with platform fee collection on OKX DEX Router.
#[derive(Clone, Debug)]
pub struct OkxSwapV3Accounts {
    pub payer: Pubkey,
    pub source_token_account: Pubkey,
    pub destination_token_account: Pubkey,
    pub source_mint: Pubkey,
    pub destination_mint: Pubkey,
    pub platform_fee_wallet: Pubkey,
    pub pool_remaining_accounts: Vec<AccountMeta>,
}

/// Build an OKX Router `swap_v3` instruction with on-chain platform fee collection.
///
/// This compiles into a single atomic transaction instruction:
/// 1. Swaps source tokens through the optimal route (Raydium, Meteora, Orca, Pump, etc.)
/// 2. Automatically and atomically deducts the platform fee (`fee_rate_bps`, e.g. 20 bps = 0.20%)
///    and routes it directly to `platform_fee_wallet` on-chain.
pub fn build_okx_swap_v3_instruction(
    accounts: &OkxSwapV3Accounts,
    args: SwapArgs,
    platform_fee_rate_bps: u16,
    order_id: u64,
    program_id: Option<Pubkey>,
) -> Result<Instruction, OkxRouterError> {
    let router_program = program_id.unwrap_or(OKX_ROUTER_PROGRAM_ID);
    let (sa_authority, _) = derive_sa_authority(&router_program);

    let data_payload = SwapV3InstructionData::new(args, platform_fee_rate_bps, order_id);
    let data =
        borsh::to_vec(&data_payload).map_err(|e| OkxRouterError::Serialization(e.to_string()))?;

    let mut metas = Vec::with_capacity(14 + accounts.pool_remaining_accounts.len());

    // 1. Payer (Signer, Writable)
    metas.push(AccountMeta::new(accounts.payer, true));

    // 2. Source Token Account (Writable)
    metas.push(AccountMeta::new(accounts.source_token_account, false));

    // 3. Destination Token Account (Writable)
    metas.push(AccountMeta::new(accounts.destination_token_account, false));

    // 4. Source Mint (Readonly)
    metas.push(AccountMeta::new_readonly(accounts.source_mint, false));

    // 5. Destination Mint (Readonly)
    metas.push(AccountMeta::new_readonly(accounts.destination_mint, false));

    // 6. Commission Account (Optional - router program as None sentinel)
    metas.push(AccountMeta::new_readonly(router_program, false));

    // 7. Platform Fee Account (Writable - receives the 0.20% fee directly on-chain)
    metas.push(AccountMeta::new(accounts.platform_fee_wallet, false));

    // 8. SA Authority PDA (Readonly)
    metas.push(AccountMeta::new_readonly(sa_authority, false));

    // 9. Source Token SA (Writable - router program as None sentinel)
    metas.push(AccountMeta::new(router_program, false));

    // 10. Destination Token SA (Writable - router program as None sentinel)
    metas.push(AccountMeta::new(router_program, false));

    // 11. Source Token Program (Readonly)
    metas.push(AccountMeta::new_readonly(spl_token::id(), false));

    // 12. Destination Token Program (Readonly)
    metas.push(AccountMeta::new_readonly(spl_token::id(), false));

    // 13. Associated Token Program (Readonly)
    metas.push(AccountMeta::new_readonly(
        spl_associated_token_account::id(),
        false,
    ));

    // 14. System Program (Readonly)
    metas.push(AccountMeta::new_readonly(
        solana_sdk::system_program::id(),
        false,
    ));

    // 15+ Underlying DEX pool accounts (Raydium / Meteora / Orca / Pump.fun accounts)
    metas.extend(accounts.pool_remaining_accounts.iter().cloned());

    Ok(Instruction {
        program_id: router_program,
        accounts: metas,
        data,
    })
}

/// Helper to create a single-hop SwapArgs for a given DEX venue.
pub fn create_single_hop_swap_args(amount_in: u64, min_return: u64, dex: Dex) -> SwapArgs {
    SwapArgs {
        amount_in,
        expect_amount_out: min_return,
        min_return,
        amounts: vec![amount_in],
        routes: vec![vec![Route {
            dexes: vec![dex],
            weights: vec![100],
        }]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey;

    #[test]
    fn test_build_swap_v3_instruction_layout() {
        let payer = pubkey!("vines1vzrYbzLMRdu58sy5vnpmPDNobnmWBcg6z56CX");
        let fee_wallet = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
        let source_ata = pubkey!("4fRNz63FNz5pA9s9F5K6U8q2uJ3d1R8m4p7k9L1n2m3p");
        let dest_ata = pubkey!("7fRNz63FNz5pA9s9F5K6U8q2uJ3d1R8m4p7k9L1n2m3p");
        let source_mint = pubkey!("So11111111111111111111111111111111111111112");
        let dest_mint = pubkey!("9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump");

        let accounts = OkxSwapV3Accounts {
            payer,
            source_token_account: source_ata,
            destination_token_account: dest_ata,
            source_mint,
            destination_mint: dest_mint,
            platform_fee_wallet: fee_wallet,
            pool_remaining_accounts: vec![],
        };

        let args = create_single_hop_swap_args(1_000_000_000, 800_000_000, Dex::RaydiumSwap);
        let ix = build_okx_swap_v3_instruction(&accounts, args, 20, 1700000000, None).unwrap();

        assert_eq!(ix.program_id, OKX_ROUTER_PROGRAM_ID);
        assert_eq!(ix.accounts[0].pubkey, payer);
        assert_eq!(ix.accounts[6].pubkey, fee_wallet);
        assert!(ix.accounts[6].is_writable);
        assert_eq!(&ix.data[0..8], &crate::types::SWAP_V3_DISCRIMINATOR);
    }
}
