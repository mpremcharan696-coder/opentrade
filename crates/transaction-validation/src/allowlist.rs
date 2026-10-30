//! Program allowlist enforcement for transaction safety.

use protocol_okx_router::addresses::{OKX_ROUTER_DEVNET_PROGRAM_ID, OKX_ROUTER_PROGRAM_ID};
use protocol_pump::addresses::{
    ASSOCIATED_TOKEN_PROGRAM, PUMP_FEE_PROGRAM, PUMP_PROGRAM, SYSTEM_PROGRAM, TOKEN_2022_PROGRAM,
    TOKEN_PROGRAM,
};
use protocol_pumpswap::addresses::PUMP_AMM_PROGRAM;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// Solana Compute Budget Program.
pub const COMPUTE_BUDGET_PROGRAM: Pubkey = pubkey!("ComputeBudget111111111111111111111111111111");

/// Raydium AMM V4 Program.
pub const RAYDIUM_AMM_V4_PROGRAM: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
/// Raydium CLMM Program.
pub const RAYDIUM_CLMM_PROGRAM: Pubkey = pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
/// Raydium CPMM Program.
pub const RAYDIUM_CPMM_PROGRAM: Pubkey = pubkey!("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");
/// Meteora DLMM Program.
pub const METEORA_DLMM_PROGRAM: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
/// Meteora Dynamic Pools Program.
pub const METEORA_DYNAMIC_PROGRAM: Pubkey = pubkey!("Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB");
/// Orca Whirlpool Program.
pub const ORCA_WHIRLPOOL_PROGRAM: Pubkey = pubkey!("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc");
/// Jupiter V6 Swap Program.
pub const JUPITER_V6_PROGRAM: Pubkey = pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");

/// All allowed program IDs that a transaction can invoke.
pub const ALLOWED_PROGRAMS: &[Pubkey] = &[
    SYSTEM_PROGRAM,
    TOKEN_PROGRAM,
    TOKEN_2022_PROGRAM,
    ASSOCIATED_TOKEN_PROGRAM,
    COMPUTE_BUDGET_PROGRAM,
    PUMP_PROGRAM,
    PUMP_FEE_PROGRAM,
    PUMP_AMM_PROGRAM,
    OKX_ROUTER_PROGRAM_ID,
    OKX_ROUTER_DEVNET_PROGRAM_ID,
    RAYDIUM_AMM_V4_PROGRAM,
    RAYDIUM_CLMM_PROGRAM,
    RAYDIUM_CPMM_PROGRAM,
    METEORA_DLMM_PROGRAM,
    METEORA_DYNAMIC_PROGRAM,
    ORCA_WHIRLPOOL_PROGRAM,
    JUPITER_V6_PROGRAM,
];

/// Check whether a program ID is explicitly permitted in transactions.
pub fn is_program_allowed(program_id: &Pubkey) -> bool {
    ALLOWED_PROGRAMS.contains(program_id)
}
