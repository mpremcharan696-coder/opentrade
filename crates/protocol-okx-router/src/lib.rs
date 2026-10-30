//! Client instruction generation and integration for OKX DEX Router Solana V1.
//!
//! Provides multi-DEX routing (Raydium, Meteora, Orca, Pump.fun, and 25+ more venues)
//! with on-chain platform fee collection.

pub mod addresses;
pub mod instructions;
pub mod types;

pub use addresses::*;
pub use instructions::*;
pub use types::*;
