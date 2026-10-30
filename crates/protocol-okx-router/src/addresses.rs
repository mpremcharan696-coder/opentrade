//! Canonical program IDs, addresses, and PDA seeds for OKX DEX Router Solana V1.

use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

/// OKX DEX Router V1 deployed on Solana Mainnet.
pub const OKX_ROUTER_PROGRAM_ID: Pubkey = pubkey!("proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u");

/// OKX DEX Router V1 staging / devnet deployment.
pub const OKX_ROUTER_DEVNET_ID: Pubkey = pubkey!("6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma");
pub const OKX_ROUTER_DEVNET_PROGRAM_ID: Pubkey = OKX_ROUTER_DEVNET_ID;

/// Wrapped SOL (WSOL) Mint.
pub const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");

/// Seed for Special Authority (SA) PDA.
pub const SEED_SA: &[u8] = b"sa";

/// Derive the Special Authority (SA) PDA for the OKX router.
pub fn derive_sa_authority(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_SA], program_id)
}
