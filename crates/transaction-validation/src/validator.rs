//! Independent transaction decoder and validator for the desktop client.
//!
//! Validates:
//! 1. Signer authorization matches expected wallet pubkey.
//! 2. All program IDs are strictly present in the allowlist.
//! 3. Platform fee transfer matches fee config and recipient.
//! 4. Trading parameters (mint, amounts, slippage) match the TradeIntent.
//! 5. Compute budget parameters are within safe bounds.
//! 6. Rejects any token approvals, authority delegates, durable nonces, or extra signers.
//! 7. Enforces configured treasury from native constant; never trusts server or UI fee recipient.

use base64::Engine;
use serde::{Deserialize, Serialize};
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_program;
use solana_sdk::transaction::VersionedTransaction;
use thiserror::Error;

use shared_types::fee::{CONFIGURED_PLATFORM_TREASURY, FeeConfig};
use shared_types::intent::{TradeIntent, TradeSide};
use shared_types::platform::TradeRoute;
use shared_types::swap::{
    HumanReadableReviewSummary, PreparedSwapCandidate, SwapValidationResult,
};

use crate::allowlist::{COMPUTE_BUDGET_PROGRAM, is_program_allowed};
use crate::simulator::simulate_transaction_rpc;
use protocol_pump::addresses::WRAPPED_SOL_MINT;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use solana_sdk::address_lookup_table::AddressLookupTableAccount;
use solana_sdk::address_lookup_table::program::id as address_lookup_table_program_id;
use solana_sdk::address_lookup_table::state::AddressLookupTable;

pub const WSOL_MINT_STR: &str = "So11111111111111111111111111111111111111112";

/// Jupiter V6 instruction discriminators
pub const JUPITER_ROUTE_DISCRIMINATOR: [u8; 8] = [0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a];
pub const JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR: [u8; 8] = [0xc1, 0x20, 0x9b, 0x33, 0x41, 0xd6, 0x9c, 0x81];
pub const JUPITER_ROUTE_WITH_TOKEN_LEDGER_DISCRIMINATOR: [u8; 8] = [0x96, 0x56, 0x47, 0x74, 0xa7, 0x5d, 0x0e, 0x68];
pub const JUPITER_SHARED_ACCOUNTS_ROUTE_WITH_TOKEN_LEDGER_DISCRIMINATOR: [u8; 8] = [0xe6, 0x79, 0x8f, 0x50, 0x77, 0x9f, 0x6a, 0xaa];
pub const JUPITER_ROUTE_V2_DISCRIMINATOR: [u8; 8] = [0xbb, 0x64, 0xfa, 0xcc, 0x31, 0xc4, 0xaf, 0x14];
pub const JUPITER_ROUTE_V2_WITH_TOKEN_LEDGER_DISCRIMINATOR: [u8; 8] = [0x50, 0xc2, 0x9a, 0x86, 0x3d, 0x1f, 0x9b, 0xf6];
pub const JUPITER_SHARED_ACCOUNTS_ROUTE_V2_DISCRIMINATOR: [u8; 8] = [0xd1, 0x98, 0x53, 0x93, 0x7c, 0xfe, 0xd8, 0xe9];
pub const JUPITER_SHARED_ACCOUNTS_ROUTE_V2_WITH_TOKEN_LEDGER_DISCRIMINATOR: [u8; 8] = [0x4b, 0x57, 0x67, 0x12, 0x1d, 0xd2, 0xe7, 0x79];

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    #[error("Base64 decoding failed: {0}")]
    Base64Decode(String),
    #[error("Failed to deserialize transaction: {0}")]
    TransactionDeserialize(String),
    #[error("Signer mismatch: expected {expected}, got {actual}")]
    SignerMismatch { expected: Pubkey, actual: Pubkey },
    #[error("Unexpected extra signer in transaction: {0}")]
    UnexpectedSigner(Pubkey),
    #[error("Forbidden or unknown program ID in transaction: {0}")]
    ForbiddenProgram(Pubkey),
    #[error("Target mint mismatch: expected {expected}, got {actual}")]
    MintMismatch { expected: Pubkey, actual: Pubkey },
    #[error("Intent spend amount exceeded: max {max_spend}, actual {actual}")]
    SpendAmountExceeded { max_spend: u64, actual: u64 },
    #[error("Intent minimum output unsatisfied: min {min_out}, actual {actual}")]
    OutputTooLow { min_out: u64, actual: u64 },
    #[error("Fee recipient mismatch: expected {expected}, got {actual}")]
    FeeRecipientMismatch { expected: Pubkey, actual: Pubkey },
    #[error("Fee amount mismatch: expected at least {expected}, got {actual}")]
    FeeAmountMismatch { expected: u64, actual: u64 },
    #[error("No valid trading instruction found in transaction")]
    MissingTradeInstruction,
    #[error("Transaction contains unresolved Address Lookup Tables: {0}")]
    UnknownLookupTable(String),
    #[error("Invalid Address Lookup Table: {0}")]
    InvalidLookupTable(String),
    #[error("Dangerous instruction detected: {0}")]
    DangerousInstruction(String),
    #[error("Unexpected system transfer: {0}")]
    UnexpectedSystemTransfer(String),
    #[error("Quote is expired: {0}")]
    ExpiredQuote(String),
    #[error("RPC simulation error: {0}")]
    SimulationError(String),
    #[error("Intent validation error: {0}")]
    InvalidIntent(String),
    #[error("Privilege escalation: account {0} cannot be writable")]
    UnauthorizedWritableAccount(Pubkey),
    #[error("Writable account mismatch: {0}")]
    WritableAccountMismatch(String),
    #[error("Instruction ordering violation: {0}")]
    InstructionOrderingViolation(String),
    #[error("Blockhash mismatch: candidate declares {expected}, but transaction message contains {actual}")]
    BlockhashMismatch { expected: String, actual: String },
}

/// Sanitized, human-readable trade preview presented to the user before signing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradePreview {
    pub intent_id: String,
    pub side: TradeSide,
    pub mint: Pubkey,
    pub route: TradeRoute,
    pub input_amount_atomic: u64,
    pub minimum_output_atomic: u64,
    pub slippage_bps: u16,
    pub platform_fee_atomic: u64,
    pub platform_fee_recipient: Pubkey,
    pub compute_unit_limit: u32,
    pub priority_fee_micro_lamports: u64,
    pub total_sol_required: u64,
    pub instructions_count: usize,
    pub warnings: Vec<String>,
}

/// Decode and validate a proposed unsigned transaction against the approved trade intent.
pub fn validate_transaction(
    intent: &TradeIntent,
    expected_signer: &Pubkey,
    fee_config: &FeeConfig,
) -> Result<TradePreview, ValidationError> {
    // 1. Verify wallet pubkey in intent matches the local signer
    if intent.wallet_pubkey != *expected_signer {
        return Err(ValidationError::SignerMismatch {
            expected: *expected_signer,
            actual: intent.wallet_pubkey,
        });
    }

    // 2. Decode serialized transaction bytes
    let tx_bytes = bs58::decode(&intent.unsigned_tx_base64)
        .into_vec()
        .or_else(|_| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(&intent.unsigned_tx_base64)
                .map_err(|e| ValidationError::Base64Decode(e.to_string()))
        })?;

    let tx: VersionedTransaction = bincode::deserialize(&tx_bytes)
        .map_err(|e| ValidationError::TransactionDeserialize(e.to_string()))?;

    let message = &tx.message;
    let account_keys = message.static_account_keys();

    // 3. Verify the primary signer is the expected wallet
    if account_keys.is_empty() {
        return Err(ValidationError::SignerMismatch {
            expected: *expected_signer,
            actual: Pubkey::default(),
        });
    }
    let fee_payer = account_keys[0];
    if fee_payer != *expected_signer {
        return Err(ValidationError::SignerMismatch {
            expected: *expected_signer,
            actual: fee_payer,
        });
    }

    // 4. Verify all instruction program IDs are on the allowlist
    let instructions = match message {
        VersionedMessage::Legacy(m) => &m.instructions,
        VersionedMessage::V0(m) => &m.instructions,
    };

    let mut warnings = Vec::new();
    let mut cu_limit = 200_000u32;
    let mut priority_fee = 0u64;
    let mut fee_transfer_found = false;
    let mut fee_amount = 0u64;
    let mut trade_instruction_found = false;

    for ix in instructions {
        let program_id = account_keys[ix.program_id_index as usize];
        if !is_program_allowed(&program_id) {
            return Err(ValidationError::ForbiddenProgram(program_id));
        }

        // Check Compute Budget program instructions
        if program_id == COMPUTE_BUDGET_PROGRAM {
            if !ix.data.is_empty() {
                match ix.data[0] {
                    2 if ix.data.len() >= 5 => {
                        cu_limit = u32::from_le_bytes(ix.data[1..5].try_into().unwrap_or_default());
                    }
                    3 if ix.data.len() >= 9 => {
                        priority_fee =
                            u64::from_le_bytes(ix.data[1..9].try_into().unwrap_or_default());
                        if priority_fee > 5_000_000 {
                            warnings.push(format!(
                                "High priority fee: {} micro-lamports per CU",
                                priority_fee
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }

        // Check System Program transfers (Platform fee collection)
        if program_id == system_program::ID && ix.data.len() >= 12 {
            let type_idx = u32::from_le_bytes(ix.data[0..4].try_into().unwrap_or_default());
            if type_idx == 2 && ix.accounts.len() >= 2 {
                let to_pubkey = account_keys[ix.accounts[1] as usize];
                let lamports = u64::from_le_bytes(ix.data[4..12].try_into().unwrap_or_default());

                if to_pubkey == fee_config.fee_wallet {
                    fee_transfer_found = true;
                    fee_amount = lamports;
                    if lamports != intent.fee.amount_atomic {
                        return Err(ValidationError::FeeAmountMismatch {
                            expected: intent.fee.amount_atomic,
                            actual: lamports,
                        });
                    }
                }
            }
        }

        // Check Trade instructions (Pump.fun, PumpSwap, or OKX DEX Router)
        if program_id == protocol_pump::addresses::PUMP_PROGRAM
            || program_id == protocol_pumpswap::addresses::PUMP_AMM_PROGRAM
        {
            if trade_instruction_found {
                return Err(ValidationError::DangerousInstruction(
                    "Duplicate trade instruction detected in transaction candidate".into(),
                ));
            }
            trade_instruction_found = true;
            let has_mint = ix
                .accounts
                .iter()
                .any(|idx| account_keys[*idx as usize] == intent.mint);
            if !has_mint {
                return Err(ValidationError::MintMismatch {
                    expected: intent.mint,
                    actual: Pubkey::default(),
                });
            }
        } else if program_id == protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID
            || program_id == protocol_okx_router::addresses::OKX_ROUTER_DEVNET_PROGRAM_ID
        {
            if trade_instruction_found {
                return Err(ValidationError::DangerousInstruction(
                    "Duplicate trade instruction detected in transaction candidate".into(),
                ));
            }
            trade_instruction_found = true;
            let has_mint = ix
                .accounts
                .iter()
                .any(|idx| account_keys[*idx as usize] == intent.mint);
            if !has_mint {
                return Err(ValidationError::MintMismatch {
                    expected: intent.mint,
                    actual: Pubkey::default(),
                });
            }

            if ix.accounts.len() > 6 {
                let platform_fee_account = account_keys[ix.accounts[6] as usize];
                if platform_fee_account != fee_config.fee_wallet {
                    return Err(ValidationError::FeeRecipientMismatch {
                        expected: fee_config.fee_wallet,
                        actual: platform_fee_account,
                    });
                }
                fee_transfer_found = true;
                fee_amount = intent.fee.amount_atomic;
            }
        }
    }

    if !trade_instruction_found {
        return Err(ValidationError::MissingTradeInstruction);
    }

    if !fee_transfer_found && intent.fee.amount_atomic > 0 {
        warnings.push("Platform fee transfer instruction was not found in transaction".to_string());
    }

    // Compute total SOL required from user wallet
    let mut total_sol_required = 5_000;
    if priority_fee > 0 {
        total_sol_required += (priority_fee * cu_limit as u64) / 1_000_000;
    }
    if intent.side == TradeSide::Buy {
        total_sol_required += intent.input_amount_atomic;
    }
    total_sol_required += fee_amount;

    Ok(TradePreview {
        intent_id: intent.intent_id.clone(),
        side: intent.side,
        mint: intent.mint,
        route: intent.route,
        input_amount_atomic: intent.input_amount_atomic,
        minimum_output_atomic: intent.minimum_output_atomic,
        slippage_bps: intent.slippage_bps,
        platform_fee_atomic: fee_amount,
        platform_fee_recipient: fee_config.fee_wallet,
        compute_unit_limit: cu_limit,
        priority_fee_micro_lamports: priority_fee,
        total_sol_required,
        instructions_count: instructions.len(),
        warnings,
    })
}

/// Independently validate an unsigned swap candidate according to strict Phase 3 rules.
///
/// Security bounds:
/// 1. Primary signer is expected local wallet; no other signers allowed.
/// 2. All program IDs are strictly on the native allowlist.
/// 3. Platform fee recipient matches hardcoded native constant CONFIGURED_PLATFORM_TREASURY.
/// 4. Rejects durable nonces, token approvals, authority delegation, and unexpected transfers.
/// 5. Rejects unresolved address lookup tables.
/// 6. Rejects expired quotes (> 60s).
pub fn validate_swap_candidate(
    candidate: &PreparedSwapCandidate,
    expected_signer: &Pubkey,
    fee_config: &FeeConfig,
) -> Result<HumanReadableReviewSummary, ValidationError> {
    let alts = candidate
        .parsed_address_lookup_tables()
        .map_err(ValidationError::InvalidLookupTable)?;
    validate_swap_candidate_with_alts(candidate, expected_signer, fee_config, &alts)
}

/// Fully validates a swap transaction candidate with explicitly provided Address Lookup Table accounts.
pub fn validate_swap_candidate_with_alts(
    candidate: &PreparedSwapCandidate,
    expected_signer: &Pubkey,
    fee_config: &FeeConfig,
    alts: &[AddressLookupTableAccount],
) -> Result<HumanReadableReviewSummary, ValidationError> {
    // Reject unsupported routes (OKX router)
    if candidate.selected_route_source == "okx_router" || candidate.selected_route_source == "okx" {
        return Err(ValidationError::InvalidIntent(
            "OKX DEX Router candidates are unsupported in Phase 3. Live candidate preparation and validation are restricted to Pump.fun, PumpSwap, and Jupiter/Raydium routes.".into(),
        ));
    }

    // Enforce configured treasury policy against candidate summary and client config: never trust UI or server fee recipient
    if candidate.summary.platform_fee_recipient != CONFIGURED_PLATFORM_TREASURY.to_string() {
        return Err(ValidationError::FeeRecipientMismatch {
            expected: CONFIGURED_PLATFORM_TREASURY,
            actual: candidate.summary.platform_fee_recipient.parse().unwrap_or_default(),
        });
    }

    if fee_config.fee_wallet != CONFIGURED_PLATFORM_TREASURY {
        return Err(ValidationError::FeeRecipientMismatch {
            expected: CONFIGURED_PLATFORM_TREASURY,
            actual: fee_config.fee_wallet,
        });
    }

    // Independently enforce fee tier rate in basis points against FeeConfig
    let allowed_rate_bps = fee_config.tier.rate_bps();
    if candidate.summary.platform_fee_bps != allowed_rate_bps {
        return Err(ValidationError::FeeAmountMismatch {
            expected: allowed_rate_bps as u64,
            actual: candidate.summary.platform_fee_bps as u64,
        });
    }

    // 1. Check quote freshness
    if candidate.summary.quote_freshness_ms > 60_000 {
        return Err(ValidationError::ExpiredQuote(format!(
            "Quote freshness is {}ms (exceeds 60s threshold)",
            candidate.summary.quote_freshness_ms
        )));
    }

    // 2. Decode serialized transaction bytes
    let tx_bytes = base64::engine::general_purpose::STANDARD
        .decode(&candidate.unsigned_tx_base64)
        .map_err(|e| ValidationError::Base64Decode(e.to_string()))?;

    let tx: VersionedTransaction = bincode::deserialize(&tx_bytes)
        .map_err(|e| ValidationError::TransactionDeserialize(e.to_string()))?;

    let message = &tx.message;

    // Verify candidate.recent_blockhash matches the blockhash in the transaction message
    let message_blockhash = message.recent_blockhash().to_string();
    if candidate.recent_blockhash != message_blockhash {
        return Err(ValidationError::BlockhashMismatch {
            expected: candidate.recent_blockhash.clone(),
            actual: message_blockhash,
        });
    }

    // 3. Resolve Address Lookup Tables (ALTs)
    let is_jupiter = candidate.selected_route_source.starts_with("jupiter")
        || candidate.selected_route_source.starts_with("raydium");
    let is_pumpswap = candidate.selected_route_source == "pumpswap"
        || candidate.selected_route_source == "pumpswap_canonical"
        || candidate.selected_route_source == "pumpswap_amm";
    let mut loaded_writable: Vec<Pubkey> = Vec::new();
    let mut loaded_readonly: Vec<Pubkey> = Vec::new();

    if let VersionedMessage::V0(v0_msg) = message {
        if !v0_msg.address_table_lookups.is_empty() && !is_jupiter {
            return Err(ValidationError::UnknownLookupTable(format!(
                "Transaction uses {} address lookup tables which cannot be independently verified",
                v0_msg.address_table_lookups.len()
            )));
        }

        for lookup in &v0_msg.address_table_lookups {
            let table = alts
                .iter()
                .find(|a| a.key == lookup.account_key)
                .ok_or_else(|| {
                    ValidationError::UnknownLookupTable(format!(
                        "Address lookup table {} not found or not provided for transaction validation",
                        lookup.account_key
                    ))
                })?;

            for &idx in &lookup.writable_indexes {
                let pk = table.addresses.get(idx as usize).ok_or_else(|| {
                    ValidationError::InvalidLookupTable(format!(
                        "Lookup table index {} out of bounds for table {} (len {})",
                        idx, lookup.account_key, table.addresses.len()
                    ))
                })?;
                loaded_writable.push(*pk);
            }

            for &idx in &lookup.readonly_indexes {
                let pk = table.addresses.get(idx as usize).ok_or_else(|| {
                    ValidationError::InvalidLookupTable(format!(
                        "Lookup table index {} out of bounds for table {} (len {})",
                        idx, lookup.account_key, table.addresses.len()
                    ))
                })?;
                loaded_readonly.push(*pk);
            }
        }
    }

    let static_keys = message.static_account_keys();
    if static_keys.is_empty() {
        return Err(ValidationError::SignerMismatch {
            expected: *expected_signer,
            actual: Pubkey::default(),
        });
    }

    // Full account list accessible to compiled instructions:
    // [static_keys, loaded_writable, loaded_readonly]
    let mut all_account_keys = Vec::with_capacity(
        static_keys.len() + loaded_writable.len() + loaded_readonly.len(),
    );
    all_account_keys.extend_from_slice(static_keys);
    all_account_keys.extend_from_slice(&loaded_writable);
    all_account_keys.extend_from_slice(&loaded_readonly);

    // 4. Verify primary fee payer and ensure NO other signers exist
    let fee_payer = static_keys[0];
    if fee_payer != *expected_signer {
        return Err(ValidationError::SignerMismatch {
            expected: *expected_signer,
            actual: fee_payer,
        });
    }

    for (idx, key) in static_keys.iter().enumerate() {
        if message.is_signer(idx) && key != expected_signer {
            return Err(ValidationError::UnexpectedSigner(*key));
        }
    }

    let is_buy = candidate.input_mint == WSOL_MINT_STR;
    let target_mint = if is_buy {
        candidate.output_mint.parse::<Pubkey>().map_err(|e| {
            ValidationError::InvalidIntent(format!("Invalid candidate output_mint: {}", e))
        })?
    } else {
        candidate.input_mint.parse::<Pubkey>().map_err(|e| {
            ValidationError::InvalidIntent(format!("Invalid candidate input_mint: {}", e))
        })?
    };

    let user_wsol_ata = get_associated_token_address_with_program_id(
        expected_signer,
        &WRAPPED_SOL_MINT,
        &spl_token::id(),
    );

    // Independently calculate allowed platform fee
    let mut expected_platform_fee = if is_buy {
        match shared_types::fee::compute_fee(candidate.input_amount_atomic, allowed_rate_bps) {
            Ok(f) => f,
            Err(shared_types::fee::FeeError::ZeroFee) => 0,
            Err(e) => return Err(ValidationError::InvalidIntent(format!("Platform fee computation failed: {}", e))),
        }
    } else {
        0u64
    };

    // 5. Enforce writable account rules (privilege escalation prevention and candidate summary consistency)
    let is_forbidden_writable = |key: &Pubkey| -> bool {
        *key == system_program::ID
            || *key == COMPUTE_BUDGET_PROGRAM
            || *key == spl_token::id()
            || *key == protocol_pump::addresses::TOKEN_2022_PROGRAM
            || *key == protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM
            || *key == protocol_pump::addresses::PUMP_PROGRAM
            || *key == protocol_pumpswap::addresses::PUMP_AMM_PROGRAM
            || *key == protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID
            || *key == protocol_okx_router::addresses::OKX_ROUTER_DEVNET_PROGRAM_ID
            || *key == crate::allowlist::JUPITER_V6_PROGRAM
            || *key == address_lookup_table_program_id()
            || *key == crate::allowlist::RAYDIUM_AMM_V4_PROGRAM
            || *key == crate::allowlist::RAYDIUM_CLMM_PROGRAM
            || *key == crate::allowlist::RAYDIUM_CPMM_PROGRAM
    };

    let mut actual_writable_keys = Vec::new();
    for (idx, key) in static_keys.iter().enumerate() {
        if message.is_maybe_writable(idx, None) {
            if is_forbidden_writable(key) || *key == target_mint {
                return Err(ValidationError::UnauthorizedWritableAccount(*key));
            }
            if !actual_writable_keys.contains(&key.to_string()) {
                actual_writable_keys.push(key.to_string());
            }
        }
    }
    for key in &loaded_writable {
        if is_forbidden_writable(key) || *key == target_mint {
            return Err(ValidationError::UnauthorizedWritableAccount(*key));
        }
        if !actual_writable_keys.contains(&key.to_string()) {
            actual_writable_keys.push(key.to_string());
        }
    }

    // Verify that every writable account in the message was declared in candidate.summary.writable_accounts
    for writable in &actual_writable_keys {
        if !candidate.summary.writable_accounts.contains(writable) {
            return Err(ValidationError::WritableAccountMismatch(format!(
                "Account {} is writable in transaction message but not declared in candidate summary",
                writable
            )));
        }
    }
    // Verify that every account declared writable in candidate.summary is actually writable
    for declared in &candidate.summary.writable_accounts {
        if !actual_writable_keys.contains(declared) {
            return Err(ValidationError::WritableAccountMismatch(format!(
                "Account {} declared writable in candidate summary but is not writable in transaction message",
                declared
            )));
        }
    }

    // 6. Inspect compiled instructions
    let instructions = match message {
        VersionedMessage::Legacy(m) => &m.instructions,
        VersionedMessage::V0(m) => &m.instructions,
    };

    let resolve_acc = |acc_idx: u8| -> Result<Pubkey, ValidationError> {
        all_account_keys
            .get(acc_idx as usize)
            .copied()
            .ok_or_else(|| {
                ValidationError::DangerousInstruction(format!(
                    "Instruction references account index {} which is out of bounds (total accounts: {})",
                    acc_idx,
                    all_account_keys.len()
                ))
            })
    };

    let mut cu_limit = 200_000u32;
    let mut priority_fee = 0u64;
    let mut fee_transfer_found = false;
    let mut fee_amount = 0u64;
    let mut trade_instruction_found = false;
    let mut trade_ix_idx: Option<usize> = None;
    let mut fee_transfer_ix_idx: Option<usize> = None;
    let mut wsol_unwrap_ix_idx: Option<usize> = None;
    let mut closed_wsol_account: Option<Pubkey> = None;
    let mut pumpswap_sell_user_quote_ata: Option<Pubkey> = None;

    for (ix_idx, ix) in instructions.iter().enumerate() {
        let program_id = resolve_acc(ix.program_id_index)?;
        if !is_program_allowed(&program_id) {
            return Err(ValidationError::ForbiddenProgram(program_id));
        }

        // Check Compute Budget program
        if program_id == COMPUTE_BUDGET_PROGRAM && !ix.data.is_empty() {
            match ix.data[0] {
                2 if ix.data.len() >= 5 => {
                    cu_limit = u32::from_le_bytes(ix.data[1..5].try_into().unwrap_or_default());
                    if cu_limit > 1_400_000 {
                        return Err(ValidationError::InvalidIntent(format!(
                            "Compute unit limit {} exceeds safe bound 1,400,000",
                            cu_limit
                        )));
                    }
                }
                3 if ix.data.len() >= 9 => {
                    priority_fee =
                        u64::from_le_bytes(ix.data[1..9].try_into().unwrap_or_default());
                    if priority_fee > 5_000_000 {
                        return Err(ValidationError::InvalidIntent(format!(
                            "Priority fee {} exceeds safe bound 5,000,000",
                            priority_fee
                        )));
                    }
                }
                _ => {}
            }
        }

        // Check System Program transfers (enforce CONFIGURED_PLATFORM_TREASURY)
        if program_id == system_program::ID && ix.data.len() >= 4 {
            let type_idx = u32::from_le_bytes(ix.data[0..4].try_into().unwrap_or_default());
            if type_idx == 2 && ix.accounts.len() >= 2 {
                let from_pubkey = resolve_acc(ix.accounts[0])?;
                let to_pubkey = resolve_acc(ix.accounts[1])?;
                let lamports = if ix.data.len() >= 12 {
                    u64::from_le_bytes(ix.data[4..12].try_into().unwrap_or_default())
                } else {
                    0
                };

                if to_pubkey == CONFIGURED_PLATFORM_TREASURY {
                    if fee_transfer_found {
                        return Err(ValidationError::DangerousInstruction(
                            "Duplicate platform fee transfer instruction detected".into(),
                        ));
                    }
                    if from_pubkey != *expected_signer {
                        return Err(ValidationError::SignerMismatch {
                            expected: *expected_signer,
                            actual: from_pubkey,
                        });
                    }
                    fee_transfer_found = true;
                    fee_transfer_ix_idx = Some(ix_idx);
                    fee_amount = lamports;
                } else if is_buy && (is_jupiter || is_pumpswap) && (to_pubkey == user_wsol_ata || to_pubkey == target_mint) {
                    // Jupiter and PumpSwap buy routes wrap native SOL by transferring user funds into their own WSOL ATA
                    if from_pubkey != *expected_signer {
                        return Err(ValidationError::SignerMismatch {
                            expected: *expected_signer,
                            actual: from_pubkey,
                        });
                    }
                    if lamports > candidate.input_amount_atomic {
                        return Err(ValidationError::SpendAmountExceeded {
                            max_spend: candidate.input_amount_atomic,
                            actual: lamports,
                        });
                    }
                } else if from_pubkey == *expected_signer {
                    // Any unexpected transfer from the user wallet is rejected
                    return Err(ValidationError::UnexpectedSystemTransfer(format!(
                        "Unauthorized transfer of {} lamports to {}",
                        lamports, to_pubkey
                    )));
                }
            } else if type_idx == 4 {
                // AdvanceNonceAccount instruction rejected
                return Err(ValidationError::DangerousInstruction(
                    "Durable nonce instruction is forbidden".into(),
                ));
            }
        }

        // Check SPL Token & Token-2022 programs: reject approvals and authority delegation
        if (program_id == spl_token::id()
            || program_id == protocol_pump::addresses::TOKEN_2022_PROGRAM)
            && !ix.data.is_empty()
        {
            match ix.data[0] {
                4 | 14 => {
                    return Err(ValidationError::DangerousInstruction(
                        "Token Approve instruction is strictly forbidden".into(),
                    ));
                }
                6 => {
                    return Err(ValidationError::DangerousInstruction(
                        "Token SetAuthority instruction is strictly forbidden".into(),
                    ));
                }
                9 => {
                    // CloseAccount instruction: must strictly unwrap the user's WSOL ATA and credit back to user wallet
                    if ix.accounts.len() < 3 {
                        return Err(ValidationError::DangerousInstruction(
                            "Token CloseAccount instruction has insufficient accounts (minimum 3 required)".into(),
                        ));
                    }

                    if is_buy && !is_jupiter && !is_pumpswap {
                        return Err(ValidationError::DangerousInstruction(
                            "Token CloseAccount instruction is forbidden in non-Jupiter and non-PumpSwap buy transactions".into(),
                        ));
                    }

                    if candidate.output_mint != WSOL_MINT_STR && !is_jupiter && !is_pumpswap {
                        return Err(ValidationError::DangerousInstruction(
                            "Token CloseAccount instruction is only permitted when output_mint is WSOL or for WSOL buy unwraps".into(),
                        ));
                    }

                    if wsol_unwrap_ix_idx.is_some() {
                        return Err(ValidationError::DangerousInstruction(
                            "Multiple Token CloseAccount instructions detected in swap candidate".into(),
                        ));
                    }

                    let closed_account = resolve_acc(ix.accounts[0])?;
                    let dest_key = resolve_acc(ix.accounts[1])?;
                    let authority_key = resolve_acc(ix.accounts[2])?;

                    if dest_key != *expected_signer {
                        return Err(ValidationError::DangerousInstruction(format!(
                            "Token CloseAccount destination {} does not match user wallet {}",
                            dest_key, expected_signer
                        )));
                    }

                    if authority_key != *expected_signer {
                        return Err(ValidationError::DangerousInstruction(format!(
                            "Token CloseAccount authority {} does not match user wallet {}",
                            authority_key, expected_signer
                        )));
                    }

                    // Confirm that the account being closed is the expected WSOL ATA for expected_signer
                    let expected_wsol_ata = get_associated_token_address_with_program_id(
                        expected_signer,
                        &WRAPPED_SOL_MINT,
                        &program_id,
                    );
                    if closed_account != expected_wsol_ata {
                        return Err(ValidationError::DangerousInstruction(format!(
                            "Token CloseAccount target {} does not match expected WSOL ATA {}",
                            closed_account, expected_wsol_ata
                        )));
                    }

                    wsol_unwrap_ix_idx = Some(ix_idx);
                    closed_wsol_account = Some(closed_account);
                }
                _ => {}
            }
        }

        // Check Trade instructions
        if program_id == protocol_pump::addresses::PUMP_PROGRAM {
            if trade_instruction_found {
                return Err(ValidationError::DangerousInstruction(
                    "Duplicate trade instruction detected in transaction candidate".into(),
                ));
            }
            trade_instruction_found = true;
            trade_ix_idx = Some(ix_idx);
            let mut has_mint = false;
            for &idx in &ix.accounts {
                if resolve_acc(idx)? == target_mint {
                    has_mint = true;
                    break;
                }
            }
            if !has_mint {
                return Err(ValidationError::MintMismatch {
                    expected: target_mint,
                    actual: Pubkey::default(),
                });
            }

            if ix.data.len() < 24 {
                return Err(ValidationError::DangerousInstruction(
                    "Pump.fun instruction data is too short".into(),
                ));
            }

            let disc: [u8; 8] = ix.data[0..8].try_into().unwrap_or_default();
            if disc == protocol_pump::instructions::BUY_DISCRIMINATOR
                || disc == protocol_pump::instructions::BUY_V2_DISCRIMINATOR
            {
                if !is_buy {
                    return Err(ValidationError::InvalidIntent(
                        "Expected sell trade, but transaction contains Pump.fun buy instruction".into(),
                    ));
                }
                let tokens_out = u64::from_le_bytes(ix.data[8..16].try_into().unwrap_or_default());
                let max_sol_cost = u64::from_le_bytes(ix.data[16..24].try_into().unwrap_or_default());

                if tokens_out < candidate.minimum_output_atomic {
                    return Err(ValidationError::OutputTooLow {
                        min_out: candidate.minimum_output_atomic,
                        actual: tokens_out,
                    });
                }
                if max_sol_cost > candidate.input_amount_atomic {
                    return Err(ValidationError::SpendAmountExceeded {
                        max_spend: candidate.input_amount_atomic,
                        actual: max_sol_cost,
                    });
                }
            } else if disc == protocol_pump::instructions::SELL_DISCRIMINATOR
                || disc == protocol_pump::instructions::SELL_V2_DISCRIMINATOR
            {
                if is_buy {
                    return Err(ValidationError::InvalidIntent(
                        "Expected buy trade, but transaction contains Pump.fun sell instruction".into(),
                    ));
                }
                let tokens_in = u64::from_le_bytes(ix.data[8..16].try_into().unwrap_or_default());
                let min_sol_out = u64::from_le_bytes(ix.data[16..24].try_into().unwrap_or_default());

                if tokens_in != candidate.input_amount_atomic {
                    return Err(ValidationError::SpendAmountExceeded {
                        max_spend: candidate.input_amount_atomic,
                        actual: tokens_in,
                    });
                }

                // Independently calculate allowed platform fee from the trade's gross SOL output bound
                let sell_fee = match shared_types::fee::compute_fee(min_sol_out, allowed_rate_bps) {
                    Ok(f) => f,
                    Err(shared_types::fee::FeeError::ZeroFee) => 0,
                    Err(e) => return Err(ValidationError::InvalidIntent(format!("Platform fee computation failed: {}", e))),
                };
                expected_platform_fee = sell_fee;

                // In a sell, trade proceeds must fund BOTH the promised net minimum output AND the independently calculated platform fee,
                // ensuring the fee transfer can never be supplemented by SOL already in the wallet.
                let required_gross_min_out = candidate
                    .minimum_output_atomic
                    .checked_add(expected_platform_fee)
                    .ok_or_else(|| {
                        ValidationError::InvalidIntent(
                            "Overflow in required gross minimum output calculation".into(),
                        )
                    })?;

                if min_sol_out < required_gross_min_out {
                    return Err(ValidationError::OutputTooLow {
                        min_out: required_gross_min_out,
                        actual: min_sol_out,
                    });
                }
            } else {
                return Err(ValidationError::DangerousInstruction(
                    "Unrecognized Pump.fun instruction discriminator".into(),
                ));
            }
        } else if program_id == protocol_pumpswap::addresses::PUMP_AMM_PROGRAM {
            if trade_instruction_found {
                return Err(ValidationError::DangerousInstruction(
                    "Duplicate trade instruction detected in transaction candidate".into(),
                ));
            }
            trade_instruction_found = true;
            trade_ix_idx = Some(ix_idx);
            let mut has_mint = false;
            for &idx in &ix.accounts {
                if resolve_acc(idx)? == target_mint {
                    has_mint = true;
                    break;
                }
            }
            if !has_mint {
                return Err(ValidationError::MintMismatch {
                    expected: target_mint,
                    actual: Pubkey::default(),
                });
            }

            if ix.data.len() < 24 {
                return Err(ValidationError::DangerousInstruction(
                    "PumpSwap AMM instruction data is too short".into(),
                ));
            }

            let disc: [u8; 8] = ix.data[0..8].try_into().unwrap_or_default();
            if disc == protocol_pumpswap::instructions::PUMPSWAP_BUY_DISCRIMINATOR {
                if !is_buy {
                    return Err(ValidationError::InvalidIntent(
                        "Expected sell trade, but transaction contains PumpSwap buy instruction".into(),
                    ));
                }
                let base_amount_out = u64::from_le_bytes(ix.data[8..16].try_into().unwrap_or_default());
                let max_quote_in = u64::from_le_bytes(ix.data[16..24].try_into().unwrap_or_default());

                if base_amount_out < candidate.minimum_output_atomic {
                    return Err(ValidationError::OutputTooLow {
                        min_out: candidate.minimum_output_atomic,
                        actual: base_amount_out,
                    });
                }
                if max_quote_in > candidate.input_amount_atomic {
                    return Err(ValidationError::SpendAmountExceeded {
                        max_spend: candidate.input_amount_atomic,
                        actual: max_quote_in,
                    });
                }
            } else if disc == protocol_pumpswap::instructions::PUMPSWAP_SELL_DISCRIMINATOR {
                if is_buy {
                    return Err(ValidationError::InvalidIntent(
                        "Expected buy trade, but transaction contains PumpSwap sell instruction".into(),
                    ));
                }

                if ix.accounts.len() < 13 {
                    return Err(ValidationError::DangerousInstruction(
                        "PumpSwap sell instruction has insufficient accounts (minimum 13 required)".into(),
                    ));
                }

                let amm_user = resolve_acc(ix.accounts[1])?;
                if amm_user != *expected_signer {
                    return Err(ValidationError::SignerMismatch {
                        expected: *expected_signer,
                        actual: amm_user,
                    });
                }

                let amm_quote_mint = resolve_acc(ix.accounts[4])?;
                let amm_user_quote_ata = resolve_acc(ix.accounts[6])?;
                let amm_quote_program = resolve_acc(ix.accounts[12])?;

                if amm_quote_mint == WRAPPED_SOL_MINT {
                    let expected_quote_ata = get_associated_token_address_with_program_id(
                        expected_signer,
                        &WRAPPED_SOL_MINT,
                        &amm_quote_program,
                    );
                    if amm_user_quote_ata != expected_quote_ata {
                        return Err(ValidationError::DangerousInstruction(format!(
                            "PumpSwap sell user quote account {} does not match expected WSOL ATA {}",
                            amm_user_quote_ata, expected_quote_ata
                        )));
                    }
                }
                pumpswap_sell_user_quote_ata = Some(amm_user_quote_ata);

                let tokens_in = u64::from_le_bytes(ix.data[8..16].try_into().unwrap_or_default());
                let min_quote_out = u64::from_le_bytes(ix.data[16..24].try_into().unwrap_or_default());

                if tokens_in != candidate.input_amount_atomic {
                    return Err(ValidationError::SpendAmountExceeded {
                        max_spend: candidate.input_amount_atomic,
                        actual: tokens_in,
                    });
                }

                // Independently calculate allowed platform fee from the trade's gross quote output bound
                let sell_fee = match shared_types::fee::compute_fee(min_quote_out, allowed_rate_bps) {
                    Ok(f) => f,
                    Err(shared_types::fee::FeeError::ZeroFee) => 0,
                    Err(e) => return Err(ValidationError::InvalidIntent(format!("Platform fee computation failed: {}", e))),
                };
                expected_platform_fee = sell_fee;

                // In a sell, trade proceeds must fund BOTH the promised net minimum output AND the independently calculated platform fee,
                // ensuring the fee transfer can never be supplemented by SOL already in the wallet.
                let required_gross_min_out = candidate
                    .minimum_output_atomic
                    .checked_add(expected_platform_fee)
                    .ok_or_else(|| {
                        ValidationError::InvalidIntent(
                            "Overflow in required gross minimum output calculation".into(),
                        )
                    })?;

                if min_quote_out < required_gross_min_out {
                    return Err(ValidationError::OutputTooLow {
                        min_out: required_gross_min_out,
                        actual: min_quote_out,
                    });
                }
            } else {
                return Err(ValidationError::DangerousInstruction(
                    "Unrecognized PumpSwap AMM instruction discriminator".into(),
                ));
            }
        } else if program_id == protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID
            || program_id == protocol_okx_router::addresses::OKX_ROUTER_DEVNET_PROGRAM_ID
        {
            return Err(ValidationError::DangerousInstruction(
                "OKX DEX Router instructions are unsupported in Phase 3 swap candidate validation".into(),
            ));
        } else if program_id == crate::allowlist::JUPITER_V6_PROGRAM {
            if trade_instruction_found {
                return Err(ValidationError::DangerousInstruction(
                    "Duplicate trade instruction detected in transaction candidate".into(),
                ));
            }
            trade_instruction_found = true;
            trade_ix_idx = Some(ix_idx);

            if ix.data.len() < 26 {
                return Err(ValidationError::DangerousInstruction(
                    "Jupiter instruction data is too short to encode route arguments".into(),
                ));
            }

            let disc: [u8; 8] = ix.data[0..8].try_into().unwrap_or_default();
            let is_shared_accounts_v1 = disc == JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR
                || disc == JUPITER_SHARED_ACCOUNTS_ROUTE_WITH_TOKEN_LEDGER_DISCRIMINATOR;
            let is_direct_route_v1 = disc == JUPITER_ROUTE_DISCRIMINATOR
                || disc == JUPITER_ROUTE_WITH_TOKEN_LEDGER_DISCRIMINATOR;
            let is_shared_accounts_v2 = disc == JUPITER_SHARED_ACCOUNTS_ROUTE_V2_DISCRIMINATOR
                || disc == JUPITER_SHARED_ACCOUNTS_ROUTE_V2_WITH_TOKEN_LEDGER_DISCRIMINATOR;
            let is_direct_route_v2 = disc == JUPITER_ROUTE_V2_DISCRIMINATOR
                || disc == JUPITER_ROUTE_V2_WITH_TOKEN_LEDGER_DISCRIMINATOR;

            if !is_shared_accounts_v1 && !is_direct_route_v1 && !is_shared_accounts_v2 && !is_direct_route_v2 {
                return Err(ValidationError::DangerousInstruction(
                    "Unrecognized Jupiter instruction discriminator".into(),
                ));
            }

            let (token_prog, authority, user_source, user_dest, in_amount, quoted_out_amount, ix_slippage_bps) = if is_shared_accounts_v2 {
                if ix.accounts.len() < 6 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter shared accounts route v2 instruction has insufficient accounts (minimum 6 required)".into(),
                    ));
                }
                if ix.data.len() < 27 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter shared accounts route v2 instruction data is too short".into(),
                    ));
                }
                let token_prog = if ix.accounts.len() > 8 {
                    resolve_acc(ix.accounts[8])?
                } else {
                    spl_token::id()
                };
                let authority = resolve_acc(ix.accounts[1])?;
                let user_source = resolve_acc(ix.accounts[2])?;
                let user_dest = resolve_acc(ix.accounts[5])?;
                let in_amount = u64::from_le_bytes(ix.data[9..17].try_into().unwrap_or_default());
                let quoted_out_amount = u64::from_le_bytes(ix.data[17..25].try_into().unwrap_or_default());
                let ix_slippage_bps = u16::from_le_bytes(ix.data[25..27].try_into().unwrap_or_default());
                (token_prog, authority, user_source, user_dest, in_amount, quoted_out_amount, ix_slippage_bps)
            } else if is_direct_route_v2 {
                if ix.accounts.len() < 3 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter route v2 instruction has insufficient accounts (minimum 3 required)".into(),
                    ));
                }
                if ix.data.len() < 26 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter route v2 instruction data is too short".into(),
                    ));
                }
                let token_prog = if ix.accounts.len() > 5 {
                    resolve_acc(ix.accounts[5])?
                } else {
                    spl_token::id()
                };
                let authority = resolve_acc(ix.accounts[0])?;
                let user_source = resolve_acc(ix.accounts[1])?;
                let user_dest = resolve_acc(ix.accounts[2])?;
                let in_amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap_or_default());
                let quoted_out_amount = u64::from_le_bytes(ix.data[16..24].try_into().unwrap_or_default());
                let ix_slippage_bps = u16::from_le_bytes(ix.data[24..26].try_into().unwrap_or_default());
                (token_prog, authority, user_source, user_dest, in_amount, quoted_out_amount, ix_slippage_bps)
            } else if is_shared_accounts_v1 {
                if ix.accounts.len() < 7 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter shared accounts route instruction has insufficient accounts (minimum 7 required)".into(),
                    ));
                }
                let len = ix.data.len();
                if len < 27 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter instruction data is too short to encode route arguments".into(),
                    ));
                }
                let in_amount = u64::from_le_bytes(ix.data[len - 19 .. len - 11].try_into().unwrap_or_default());
                let quoted_out_amount = u64::from_le_bytes(ix.data[len - 11 .. len - 3].try_into().unwrap_or_default());
                let ix_slippage_bps = u16::from_le_bytes(ix.data[len - 3 .. len - 1].try_into().unwrap_or_default());
                (
                    resolve_acc(ix.accounts[0])?,
                    resolve_acc(ix.accounts[2])?,
                    resolve_acc(ix.accounts[3])?,
                    resolve_acc(ix.accounts[6])?,
                    in_amount,
                    quoted_out_amount,
                    ix_slippage_bps,
                )
            } else {
                if ix.accounts.len() < 4 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter route instruction has insufficient accounts (minimum 4 required)".into(),
                    ));
                }
                let len = ix.data.len();
                if len < 27 {
                    return Err(ValidationError::DangerousInstruction(
                        "Jupiter instruction data is too short to encode route arguments".into(),
                    ));
                }
                let in_amount = u64::from_le_bytes(ix.data[len - 19 .. len - 11].try_into().unwrap_or_default());
                let quoted_out_amount = u64::from_le_bytes(ix.data[len - 11 .. len - 3].try_into().unwrap_or_default());
                let ix_slippage_bps = u16::from_le_bytes(ix.data[len - 3 .. len - 1].try_into().unwrap_or_default());
                (
                    resolve_acc(ix.accounts[0])?,
                    resolve_acc(ix.accounts[1])?,
                    resolve_acc(ix.accounts[2])?,
                    resolve_acc(ix.accounts[3])?,
                    in_amount,
                    quoted_out_amount,
                    ix_slippage_bps,
                )
            };

            if token_prog != spl_token::id() && token_prog != protocol_pump::addresses::TOKEN_2022_PROGRAM {
                return Err(ValidationError::ForbiddenProgram(token_prog));
            }

            if authority != *expected_signer {
                return Err(ValidationError::SignerMismatch {
                    expected: *expected_signer,
                    actual: authority,
                });
            }

            let expected_user_wsol_ata = get_associated_token_address_with_program_id(
                expected_signer,
                &WRAPPED_SOL_MINT,
                &spl_token::id(),
            );

            let expected_user_target_ata_spl = get_associated_token_address_with_program_id(
                expected_signer,
                &target_mint,
                &spl_token::id(),
            );
            let expected_user_target_ata_t22 = get_associated_token_address_with_program_id(
                expected_signer,
                &target_mint,
                &protocol_pump::addresses::TOKEN_2022_PROGRAM,
            );

            if is_buy {
                if user_source != expected_user_wsol_ata {
                    return Err(ValidationError::DangerousInstruction(format!(
                        "Jupiter buy instruction user source token account {} does not match user WSOL ATA {}",
                        user_source, expected_user_wsol_ata
                    )));
                }
                if user_dest != expected_user_target_ata_spl && user_dest != expected_user_target_ata_t22 {
                    return Err(ValidationError::DangerousInstruction(format!(
                        "Jupiter buy instruction user destination token account {} does not match user token ATA {}",
                        user_dest, expected_user_target_ata_spl
                    )));
                }
            } else {
                if user_source != expected_user_target_ata_spl && user_source != expected_user_target_ata_t22 {
                    return Err(ValidationError::DangerousInstruction(format!(
                        "Jupiter sell instruction user source token account {} does not match user token ATA {}",
                        user_source, expected_user_target_ata_spl
                    )));
                }
                if user_dest != expected_user_wsol_ata {
                    return Err(ValidationError::DangerousInstruction(format!(
                        "Jupiter sell instruction user destination token account {} does not match user WSOL ATA {}",
                        user_dest, expected_user_wsol_ata
                    )));
                }
            }

            if ix_slippage_bps > candidate.slippage_bps {
                return Err(ValidationError::InvalidIntent(format!(
                    "Jupiter instruction slippage tolerance {} bps exceeds candidate slippage {} bps",
                    ix_slippage_bps, candidate.slippage_bps
                )));
            }

            let enforced_min_out = if ix_slippage_bps >= 10_000 {
                0u64
            } else {
                ((quoted_out_amount as u128 * (10_000u128 - ix_slippage_bps as u128)) / 10_000u128) as u64
            };

            if is_buy {
                if in_amount > candidate.input_amount_atomic {
                    return Err(ValidationError::SpendAmountExceeded {
                        max_spend: candidate.input_amount_atomic,
                        actual: in_amount,
                    });
                }
                let expected_swap_in = candidate.input_amount_atomic.saturating_sub(expected_platform_fee);
                if in_amount != expected_swap_in {
                    return Err(ValidationError::InvalidIntent(format!(
                        "Jupiter buy instruction in_amount {} does not match net swap input {} (gross {} - fee {})",
                        in_amount, expected_swap_in, candidate.input_amount_atomic, expected_platform_fee
                    )));
                }
                if enforced_min_out.saturating_add(1) < candidate.minimum_output_atomic {
                    return Err(ValidationError::OutputTooLow {
                        min_out: candidate.minimum_output_atomic,
                        actual: enforced_min_out,
                    });
                }
            } else {
                if in_amount != candidate.input_amount_atomic {
                    return Err(ValidationError::SpendAmountExceeded {
                        max_spend: candidate.input_amount_atomic,
                        actual: in_amount,
                    });
                }

                let sell_fee = match shared_types::fee::compute_fee(enforced_min_out, allowed_rate_bps) {
                    Ok(f) => f,
                    Err(shared_types::fee::FeeError::ZeroFee) => 0,
                    Err(e) => return Err(ValidationError::InvalidIntent(format!("Platform fee computation failed: {}", e))),
                };
                expected_platform_fee = sell_fee;

                let required_gross_min_out = candidate
                    .minimum_output_atomic
                    .checked_add(expected_platform_fee)
                    .ok_or_else(|| {
                        ValidationError::InvalidIntent(
                            "Overflow in required gross minimum output calculation".into(),
                        )
                    })?;

                if enforced_min_out.saturating_add(1) < required_gross_min_out {
                    return Err(ValidationError::OutputTooLow {
                        min_out: required_gross_min_out,
                        actual: enforced_min_out,
                    });
                }
            }
        }
    }

    if !trade_instruction_found {
        return Err(ValidationError::MissingTradeInstruction);
    }

    // Independently enforce fee amount: candidate summary fee and instruction transfer must both match expected_platform_fee
    if candidate.summary.platform_fee_atomic != expected_platform_fee {
        return Err(ValidationError::FeeAmountMismatch {
            expected: expected_platform_fee,
            actual: candidate.summary.platform_fee_atomic,
        });
    }

    if fee_amount != expected_platform_fee {
        return Err(ValidationError::FeeAmountMismatch {
            expected: expected_platform_fee,
            actual: fee_amount,
        });
    }

    if !fee_transfer_found && expected_platform_fee > 0 {
        return Err(ValidationError::FeeRecipientMismatch {
            expected: CONFIGURED_PLATFORM_TREASURY,
            actual: Pubkey::default(),
        });
    }

    if fee_transfer_found && expected_platform_fee == 0 {
        return Err(ValidationError::DangerousInstruction(
            "Unexpected fee transfer instruction when candidate platform fee is 0".into(),
        ));
    }

    // For sells: ensure swap sell instruction executes before the platform fee transfer,
    // establishing that fee is deducted directly from trade proceeds.
    if !is_buy {
        if let (Some(t_idx), Some(f_idx)) = (trade_ix_idx, fee_transfer_ix_idx) {
            if f_idx < t_idx {
                return Err(ValidationError::InstructionOrderingViolation(format!(
                    "Sell transaction orders fee transfer (instruction {}) before trade execution (instruction {}). Fee must be deducted from swap proceeds after the sell completes.",
                    f_idx, t_idx
                )));
            }
        }

        // For SOL-quoted PumpSwap sells: verify that WSOL proceeds are unwrapped to user wallet before fee deduction
        if is_pumpswap && candidate.output_mint == WSOL_MINT_STR {
            let u_idx = wsol_unwrap_ix_idx.ok_or_else(|| {
                ValidationError::InstructionOrderingViolation(
                    "PumpSwap sell outputs wrapped SOL (WSOL). A close_account instruction is required to unwrap WSOL proceeds to native SOL before platform fee transfer.".into()
                )
            })?;
            if let Some(t_idx) = trade_ix_idx {
                if u_idx < t_idx {
                    return Err(ValidationError::InstructionOrderingViolation(format!(
                        "WSOL unwrap (instruction {}) occurs before trade execution (instruction {}). Proceeds must be unwrapped after swap completes.",
                        u_idx, t_idx
                    )));
                }
            }
            if let Some(f_idx) = fee_transfer_ix_idx {
                if f_idx < u_idx {
                    return Err(ValidationError::InstructionOrderingViolation(format!(
                        "Fee transfer (instruction {}) occurs before WSOL proceeds are unwrapped (instruction {}). Platform fee must be deducted from unwrapped proceeds.",
                        f_idx, u_idx
                    )));
                }
            }

            // Confirm that the account closed in unwrap is the exact user_quote_ata from the PumpSwap AMM sell instruction
            if let (Some(closed_acc), Some(amm_quote_ata)) = (closed_wsol_account, pumpswap_sell_user_quote_ata) {
                if closed_acc != amm_quote_ata {
                    return Err(ValidationError::DangerousInstruction(format!(
                        "Token CloseAccount target {} does not match PumpSwap AMM user quote account {}",
                        closed_acc, amm_quote_ata
                    )));
                }
            }
        } else if is_pumpswap && is_buy && candidate.input_mint == WSOL_MINT_STR {
            // For SOL-quoted PumpSwap buys: if WSOL unwrap is present, verify it occurs after trade execution and targets user WSOL ATA
            if let Some(u_idx) = wsol_unwrap_ix_idx {
                if let Some(t_idx) = trade_ix_idx {
                    if u_idx < t_idx {
                        return Err(ValidationError::InstructionOrderingViolation(format!(
                            "WSOL unwrap (instruction {}) occurs before trade execution (instruction {}). Leftover WSOL must be unwrapped after swap completes.",
                            u_idx, t_idx
                        )));
                    }
                }
                if let Some(closed_acc) = closed_wsol_account {
                    if closed_acc != user_wsol_ata {
                        return Err(ValidationError::DangerousInstruction(format!(
                            "Token CloseAccount target {} does not match user WSOL ATA {}",
                            closed_acc, user_wsol_ata
                        )));
                    }
                }
            }
        } else if !is_pumpswap && !is_jupiter && wsol_unwrap_ix_idx.is_some() {
            return Err(ValidationError::DangerousInstruction(
                "Token CloseAccount instruction is only permitted for PumpSwap and Jupiter WSOL trades".into(),
            ));
        }
    }

    let is_buy = candidate.input_mint == WSOL_MINT_STR;
    let input_display = if is_buy {
        format!("{:.4} SOL", candidate.input_amount_atomic as f64 / 1e9)
    } else {
        format!("{:.2} Tokens", candidate.input_amount_atomic as f64 / 1e6)
    };

    let estimated_output_display = if is_buy {
        format!("{:.2} Tokens", candidate.estimated_output_atomic as f64 / 1e6)
    } else {
        format!("{:.6} SOL", candidate.estimated_output_atomic as f64 / 1e9)
    };

    let minimum_output_display = if is_buy {
        format!("{:.2} Tokens", candidate.minimum_output_atomic as f64 / 1e6)
    } else {
        format!("{:.6} SOL", candidate.minimum_output_atomic as f64 / 1e9)
    };

    Ok(HumanReadableReviewSummary {
        venue: candidate.venue.clone(),
        side: if is_buy { "Buy".to_string() } else { "Sell".to_string() },
        input_display,
        estimated_output_display,
        minimum_output_display,
        slippage_display: format!("{:.1}%", candidate.slippage_bps as f64 / 100.0),
        platform_fee_display: format!("{:.6} SOL", fee_amount as f64 / 1e9),
        platform_fee_recipient_verified: true,
        priority_fee_display: format!("{} CU @ {} µ-lamports", cu_limit, priority_fee),
        rent_cost_display: format!(
            "{:.6} SOL",
            candidate.summary.estimated_rent_lamports as f64 / 1e9
        ),
        total_sol_estimated_display: format!(
            "{:.6} SOL",
            candidate.summary.total_sol_required_atomic as f64 / 1e9
        ),
        transaction_size_bytes: candidate.summary.tx_byte_size,
        quote_age_seconds: (candidate.summary.quote_freshness_ms / 1000) as u64,
        authorization_status: "Verified & Simulated (Signing & Broadcast Disabled in Phase 3)"
            .to_string(),
    })
}

/// Fetches and fully validates on-chain Address Lookup Tables from Solana RPC.
/// Enforces:
/// 1. Account owner is strictly AddressLookupTab1e1111111111111111111111111
/// 2. State deserializes with official SDK AddressLookupTable
/// 3. Lookup table is active (deactivation_slot == u64::MAX)
pub async fn fetch_and_validate_address_lookup_tables(
    rpc_url: &str,
    table_addresses: &[Pubkey],
) -> Result<Vec<AddressLookupTableAccount>, ValidationError> {
    if table_addresses.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(5000))
        .build()
        .map_err(|e| ValidationError::SimulationError(format!("Failed to build HTTP client for ALT fetch: {}", e)))?;

    let table_keys_strings: Vec<String> = table_addresses.iter().map(|k| k.to_string()).collect();

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getMultipleAccounts",
        "params": [
            table_keys_strings,
            { "encoding": "base64", "commitment": "confirmed" }
        ]
    });

    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ValidationError::SimulationError(format!("RPC ALT request failed: {}", e)))?;

    let json = res
        .json::<serde_json::Value>()
        .await
        .map_err(|e| ValidationError::SimulationError(format!("Failed to parse RPC ALT response: {}", e)))?;

    let accounts = json["result"]["value"]
        .as_array()
        .ok_or_else(|| ValidationError::SimulationError("Missing result.value in getMultipleAccounts for ALTs".into()))?;

    let mut lookup_tables = Vec::new();
    for (idx, acc_val) in accounts.iter().enumerate() {
        let table_key = table_addresses[idx];
        if acc_val.is_null() {
            return Err(ValidationError::UnknownLookupTable(format!(
                "ALT account {} not found on-chain", table_key
            )));
        }

        let owner_str = acc_val["owner"]
            .as_str()
            .ok_or_else(|| ValidationError::InvalidLookupTable(format!("Missing owner for ALT {}", table_key)))?;

        let expected_owner = address_lookup_table_program_id().to_string();
        if owner_str != expected_owner {
            return Err(ValidationError::InvalidLookupTable(format!(
                "ALT account {} has invalid owner: expected {}, got {}",
                table_key, expected_owner, owner_str
            )));
        }

        let b64 = acc_val["data"][0]
            .as_str()
            .ok_or_else(|| ValidationError::InvalidLookupTable(format!("Missing base64 data for ALT {}", table_key)))?;

        let data = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| ValidationError::Base64Decode(format!("Failed to decode base64 for ALT {}: {}", table_key, e)))?;

        let lookup_table = AddressLookupTable::deserialize(&data)
            .map_err(|e| ValidationError::InvalidLookupTable(format!("Failed to deserialize ALT {}: {:?}", table_key, e)))?;

        if lookup_table.meta.deactivation_slot != u64::MAX {
            return Err(ValidationError::InvalidLookupTable(format!(
                "ALT {} is inactive or deactivated (deactivation_slot: {})",
                table_key, lookup_table.meta.deactivation_slot
            )));
        }

        lookup_tables.push(AddressLookupTableAccount {
            key: table_key,
            addresses: lookup_table.addresses.to_vec(),
        });
    }

    Ok(lookup_tables)
}

/// Combined validation and RPC simulation pipeline for Phase 3 desktop authorization review.
pub async fn validate_and_simulate_swap_candidate(
    candidate: &PreparedSwapCandidate,
    expected_signer: &Pubkey,
    fee_config: &FeeConfig,
    rpc_url: &str,
) -> SwapValidationResult {
    let mut validation_errors = Vec::new();
    let validation_warnings = Vec::new();

    // Check if the transaction message uses address table lookups and resolve them from RPC
    let (alts_res, has_lookups) = match base64::engine::general_purpose::STANDARD.decode(&candidate.unsigned_tx_base64) {
        Ok(tx_bytes) => match bincode::deserialize::<VersionedTransaction>(&tx_bytes) {
            Ok(tx) => match &tx.message {
                VersionedMessage::V0(v0) if !v0.address_table_lookups.is_empty() => {
                    let table_keys: Vec<Pubkey> = v0.address_table_lookups.iter().map(|l| l.account_key).collect();
                    (fetch_and_validate_address_lookup_tables(rpc_url, &table_keys).await, true)
                }
                _ => (Ok(Vec::new()), false),
            },
            Err(e) => (Err(ValidationError::TransactionDeserialize(e.to_string())), false),
        },
        Err(e) => (Err(ValidationError::Base64Decode(e.to_string())), false),
    };

    let alts = match alts_res {
        Ok(tables) => {
            if has_lookups {
                tables
            } else {
                candidate.parsed_address_lookup_tables().unwrap_or_default()
            }
        }
        Err(e) => {
            validation_errors.push(format!("On-chain ALT validation failed: {}", e));
            Vec::new()
        }
    };

    let review_summary = if validation_errors.is_empty() {
        match validate_swap_candidate_with_alts(candidate, expected_signer, fee_config, &alts) {
            Ok(summary) => summary,
            Err(e) => {
                validation_errors.push(format!("Native validation failed: {}", e));
                HumanReadableReviewSummary {
                    venue: candidate.venue.clone(),
                    side: if candidate.input_mint == WSOL_MINT_STR { "Buy".to_string() } else { "Sell".to_string() },
                    input_display: format!("{}", candidate.input_amount_atomic),
                    estimated_output_display: format!("{}", candidate.estimated_output_atomic),
                    minimum_output_display: format!("{}", candidate.minimum_output_atomic),
                    slippage_display: format!("{:.1}%", candidate.slippage_bps as f64 / 100.0),
                    platform_fee_display: format!("{} atomic", candidate.summary.platform_fee_atomic),
                    platform_fee_recipient_verified: false,
                    priority_fee_display: "0".to_string(),
                    rent_cost_display: "0".to_string(),
                    total_sol_estimated_display: "0".to_string(),
                    transaction_size_bytes: candidate.summary.tx_byte_size,
                    quote_age_seconds: (candidate.summary.quote_freshness_ms / 1000) as u64,
                    authorization_status: "REJECTED (Native Validation Failed)".to_string(),
                }
            }
        }
    } else {
        HumanReadableReviewSummary {
            venue: candidate.venue.clone(),
            side: if candidate.input_mint == WSOL_MINT_STR { "Buy".to_string() } else { "Sell".to_string() },
            input_display: format!("{}", candidate.input_amount_atomic),
            estimated_output_display: format!("{}", candidate.estimated_output_atomic),
            minimum_output_display: format!("{}", candidate.minimum_output_atomic),
            slippage_display: format!("{:.1}%", candidate.slippage_bps as f64 / 100.0),
            platform_fee_display: format!("{} atomic", candidate.summary.platform_fee_atomic),
            platform_fee_recipient_verified: false,
            priority_fee_display: "0".to_string(),
            rent_cost_display: "0".to_string(),
            total_sol_estimated_display: "0".to_string(),
            transaction_size_bytes: candidate.summary.tx_byte_size,
            quote_age_seconds: (candidate.summary.quote_freshness_ms / 1000) as u64,
            authorization_status: "REJECTED (Native Validation Failed)".to_string(),
        }
    };

    let mut is_simulated = false;
    let mut simulation_units_consumed = None;
    let mut simulation_logs = Vec::new();

    // If native checks passed, simulate transaction on-chain via Solana RPC (read-only)
    if validation_errors.is_empty() {
        match simulate_transaction_rpc(rpc_url, &candidate.unsigned_tx_base64).await {
            Ok(sim) => {
                is_simulated = true;
                simulation_units_consumed = Some(sim.units_consumed);
                simulation_logs = sim.logs;
            }
            Err(e) => {
                validation_errors.push(format!("RPC Simulation Reverted / Failed: {}", e));
            }
        }
    }

    let is_valid = validation_errors.is_empty() && is_simulated;

    SwapValidationResult {
        is_valid,
        is_simulated,
        candidate_id: candidate.candidate_id.clone(),
        validation_errors,
        validation_warnings,
        simulation_units_consumed,
        simulation_logs,
        candidate_summary: Some(candidate.summary.clone()),
        review_summary,
        candidate: Some(candidate.clone()),
    }
}
