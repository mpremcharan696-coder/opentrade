//! PumpSwap AMM protocol support: addresses, pool state, and instruction builders.
//!
//! Handles post-graduation trading once a Pump.fun coin migrates off the bonding curve.
//! Direct Rust port of `pumpswap_market.py` verified against `pump_swap_idl.json`.

pub mod addresses;
pub mod instructions;
pub mod pool;
