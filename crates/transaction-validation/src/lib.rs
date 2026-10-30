//! Local transaction validation and trade preview generation.
//!
//! Mandatory security boundary for the desktop client:
//! Independently parses and validates cloud-proposed unsigned transactions
//! against user trade intents, ensuring no arbitrary transactions are signed.

pub mod allowlist;
pub mod simulator;
pub mod validator;

pub use allowlist::is_program_allowed;
pub use simulator::{SimulationError, SimulationResult, simulate_transaction_rpc};
pub use validator::{
    TradePreview, ValidationError, validate_and_simulate_swap_candidate, validate_swap_candidate,
    validate_transaction,
};
