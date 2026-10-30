//! Unsigned swap transaction preparation and validation contract types.
//!
//! Enforces strong types for Phase 3: Safe Swap Transaction Preparation and Validation.
//! Binds the candidate to exact mints, atomic amounts, route provider, slippage limit,
//! native platform treasury policy, and quote freshness.

use serde::{Deserialize, Serialize};
use crate::fee::FeeTier;
use crate::intent::TradeSide;

/// Request to prepare an unsigned swap transaction candidate from a selected route quote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepareSwapRequest {
    /// Expected customer wallet public key (must match the transaction fee payer and signer).
    pub wallet_pubkey: String,

    /// Trade side: Buy or Sell.
    pub side: TradeSide,

    /// Exact input mint address (WSOL for buy, token mint for sell).
    pub input_mint: String,

    /// Exact output mint address (token mint for buy, WSOL for sell).
    pub output_mint: String,

    /// Atomic input amount (lamports for buy, token base units for sell).
    pub input_amount_atomic: u64,

    /// Selected route provider source, e.g. "pump_bonding_curve", "pumpswap", "jupiter", "okx_router".
    pub selected_route_source: String,

    /// Human-readable venue name, e.g. "Pump.fun Bonding Curve", "PumpSwap AMM".
    pub venue: String,

    /// Pool address if applicable for the route.
    #[serde(default)]
    pub pool_address: Option<String>,

    /// Slippage tolerance in basis points (e.g. 100 = 1.0%).
    pub slippage_bps: u16,

    /// Strictly required minimum output bound selected by the user. Candidate preparation verifies
    /// fresh on-chain curve math against this bound to prevent adverse slippage and bind candidate instructions.
    pub expected_minimum_output_atomic: u64,

    /// Unix timestamp in milliseconds when the route quote was obtained.
    pub quoted_at_unix_ms: u128,

    /// On-chain state slot when the route quote was queried.
    #[serde(default)]
    pub state_slot: Option<u64>,

    /// Optional fee tier override (defaults to FeeTier::Default).
    #[serde(default)]
    pub fee_tier: Option<FeeTier>,

    /// Priority fee in micro-lamports per Compute Unit (defaults to 50_000).
    #[serde(default)]
    pub priority_fee_micro_lamports: Option<u64>,
}

/// Metadata and privilege summary for an account involved in an instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapAccountMetaSummary {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
    pub label: Option<String>,
}

/// Structured inspection detail for a single instruction in the transaction candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapInstructionDetail {
    pub program_id: String,
    pub program_name: String,
    pub accounts: Vec<SwapAccountMetaSummary>,
    pub data_len: usize,
    pub description: String,
}

/// Complete structural summary of the unsigned swap candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapCandidateSummary {
    /// Detailed list of all instructions in execution order.
    pub instructions: Vec<SwapInstructionDetail>,

    /// All distinct program IDs called by the transaction.
    pub program_ids: Vec<String>,

    /// All writable account addresses.
    pub writable_accounts: Vec<String>,

    /// All signer account addresses (must ONLY contain the customer wallet).
    pub signer_accounts: Vec<String>,

    /// Configured compute unit limit.
    pub compute_unit_limit: u32,

    /// Priority fee in micro-lamports per compute unit.
    pub priority_fee_micro_lamports: u64,

    /// Platform fee rate in basis points.
    pub platform_fee_bps: u16,

    /// Atomic platform fee amount collected.
    pub platform_fee_atomic: u64,

    /// Verified recipient address for the platform fee transfer.
    pub platform_fee_recipient: String,

    /// Venue or liquidity pool swap fee if explicitly reported.
    #[serde(default)]
    pub venue_fee_atomic: Option<u64>,

    /// Estimated rent costs for newly created accounts (e.g. Associated Token Account).
    pub estimated_rent_lamports: u64,

    /// Total SOL required from the user's wallet (including input if buy, rent, fee, priority).
    pub total_sol_required_atomic: u64,

    /// Total byte size of the serialized transaction.
    pub tx_byte_size: usize,

    /// Elapsed time in milliseconds between quote creation and preparation.
    pub quote_freshness_ms: u128,
}

/// An unsigned, reviewable swap transaction candidate returned by the preparation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedSwapCandidate {
    /// Unique candidate identifier.
    pub candidate_id: String,

    /// Selected route provider source.
    pub selected_route_source: String,

    /// Human-readable venue name.
    pub venue: String,

    /// Pool address if applicable.
    #[serde(default)]
    pub pool_address: Option<String>,

    /// Exact input mint address.
    pub input_mint: String,

    /// Exact output mint address.
    pub output_mint: String,

    /// Input amount in atomic units.
    pub input_amount_atomic: u64,

    /// Estimated gross output amount before slippage.
    pub estimated_output_atomic: u64,

    /// Guaranteed minimum output amount enforced by transaction instruction slippage limit.
    pub minimum_output_atomic: u64,

    /// Slippage limit in basis points.
    pub slippage_bps: u16,

    /// Fresh recent blockhash from Solana RPC.
    pub recent_blockhash: String,

    /// Last valid block height for the recent blockhash.
    pub last_valid_block_height: u64,

    /// Current RPC slot when candidate was constructed.
    pub state_slot: u64,

    /// ISO-8601 creation timestamp.
    pub created_at: String,

    /// ISO-8601 expiration timestamp.
    pub expires_at: String,

    /// Base64-encoded serialized unsigned Solana VersionedTransaction.
    pub unsigned_tx_base64: String,

    /// Structured summary of instructions, accounts, fees, and rent effects.
    pub summary: SwapCandidateSummary,

    /// Optional Address Lookup Tables required to resolve v0 transactions offline/locally.
    #[serde(default)]
    pub address_lookup_tables: Vec<AddressLookupTableEntry>,
}

/// Address lookup table entry containing table pubkey and loaded addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressLookupTableEntry {
    pub key: String,
    pub addresses: Vec<String>,
}

impl PreparedSwapCandidate {
    /// Deserializes `address_lookup_tables` into Solana SDK `AddressLookupTableAccount`s.
    pub fn parsed_address_lookup_tables(
        &self,
    ) -> Result<Vec<solana_sdk::address_lookup_table::AddressLookupTableAccount>, String> {
        use std::str::FromStr;
        let mut result = Vec::new();
        for entry in &self.address_lookup_tables {
            let key = solana_sdk::pubkey::Pubkey::from_str(&entry.key)
                .map_err(|e| format!("Invalid ALT key {}: {}", entry.key, e))?;
            let mut addresses = Vec::new();
            for addr in &entry.addresses {
                let pk = solana_sdk::pubkey::Pubkey::from_str(addr)
                    .map_err(|e| format!("Invalid address in ALT {}: {}", addr, e))?;
                addresses.push(pk);
            }
            result.push(solana_sdk::address_lookup_table::AddressLookupTableAccount {
                key,
                addresses,
            });
        }
        Ok(result)
    }
}

/// Human-readable approval review summary presented to the user before authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanReadableReviewSummary {
    pub venue: String,
    pub side: String,
    pub input_display: String,
    pub estimated_output_display: String,
    pub minimum_output_display: String,
    pub slippage_display: String,
    pub platform_fee_display: String,
    pub platform_fee_recipient_verified: bool,
    pub priority_fee_display: String,
    pub rent_cost_display: String,
    pub total_sol_estimated_display: String,
    pub transaction_size_bytes: usize,
    pub quote_age_seconds: u64,
    pub authorization_status: String,
}

/// Result of independent native validation and read-only simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapValidationResult {
    /// Whether all native safety checks and RPC simulation succeeded.
    pub is_valid: bool,

    /// Whether RPC read-only simulation succeeded.
    pub is_simulated: bool,

    /// Candidate identifier.
    pub candidate_id: String,

    /// Validation error messages (empty if valid).
    pub validation_errors: Vec<String>,

    /// Informational warnings.
    pub validation_warnings: Vec<String>,

    /// Compute units consumed during RPC simulation if simulated.
    #[serde(default)]
    pub simulation_units_consumed: Option<u64>,

    /// Solana RPC simulation execution logs.
    pub simulation_logs: Vec<String>,

    /// Full candidate summary if available.
    #[serde(default)]
    pub candidate_summary: Option<SwapCandidateSummary>,

    /// Human-readable breakdown for desktop UI review.
    pub review_summary: HumanReadableReviewSummary,

    /// Unsigned prepared swap candidate (populated when returned from desktop preparation).
    #[serde(default)]
    pub candidate: Option<PreparedSwapCandidate>,
}

/// Execution and confirmation status of a swap transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapExecutionStatus {
    Prepared,
    Signed,
    Submitted,
    Confirmed,
    Failed,
    Expired,
    Unknown,
}

impl std::fmt::Display for SwapExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prepared => write!(f, "Prepared"),
            Self::Signed => write!(f, "Signed"),
            Self::Submitted => write!(f, "Submitted"),
            Self::Confirmed => write!(f, "Confirmed"),
            Self::Failed => write!(f, "Failed"),
            Self::Expired => write!(f, "Expired"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Result of transaction signing, RPC submission, and confirmation tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapSubmissionResult {
    /// High-level execution status.
    pub status: SwapExecutionStatus,

    /// Transaction signature in Base58 (present if signed & submitted).
    #[serde(default)]
    pub signature: Option<String>,

    /// Block slot at which the transaction was processed/confirmed.
    #[serde(default)]
    pub slot: Option<u64>,

    /// Number of cluster confirmations if available.
    #[serde(default)]
    pub confirmations: Option<usize>,

    /// Error message if signing, preflight, submission, or on-chain execution failed.
    #[serde(default)]
    pub error: Option<String>,

    /// Transaction simulation or execution logs from RPC.
    #[serde(default)]
    pub logs: Vec<String>,

    /// Solscan explorer URL for verified submitted transaction.
    #[serde(default)]
    pub explorer_url: Option<String>,

    /// True if the transaction blockhash was determined to be expired.
    #[serde(default)]
    pub blockhash_expired: bool,
}

