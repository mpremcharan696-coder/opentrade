//! Phase 3 Swap Candidate Validation and Adversarial Rejection Tests.
//!
//! Validates:
//! 1. Valid Pump.fun and PumpSwap candidates pass native validation.
//! 2. Adversarial cases are strictly rejected:
//!    - Wrong treasury recipient (diverted fee)
//!    - Altered target mint
//!    - Extra unexpected signer
//!    - Token approve instruction injection
//!    - Token set_authority instruction injection
//!    - Unallowlisted / forbidden program ID
//!    - Unresolved address lookup tables (ALTs)
//!    - Stale / expired quote (> 60s)
//!    - Malformed transaction bytes
//!    - Durable nonce instruction

use base64::Engine;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::{Message, MessageAddressTableLookup};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_instruction;
use solana_sdk::transaction::VersionedTransaction;

use protocol_pump::instructions::build_buy_v2_instructions;
use protocol_pumpswap::instructions::{
    build_pumpswap_buy_instructions, build_pumpswap_sell_instructions,
};
use protocol_pumpswap::pool::PumpSwapPoolState;
use shared_types::fee::{CONFIGURED_PLATFORM_TREASURY, FeeConfig, FeeTier};
use shared_types::swap::{PreparedSwapCandidate, SwapCandidateSummary};
use shared_types::token::TokenInfo;
use transaction_validation::validator::{ValidationError, WSOL_MINT_STR, validate_swap_candidate};

fn mock_token(mint: Pubkey) -> TokenInfo {
    TokenInfo {
        name: "Test Token".to_string(),
        symbol: "TEST".to_string(),
        uri: "https://test.com".to_string(),
        mint,
        platform: shared_types::platform::Platform::PumpFun,
        bonding_curve: None,
        quote_mint: Pubkey::default(),
        quote_token_program: spl_token::id(),
        base_token_program: spl_token::id(),
        creator: None,
        creator_vault: None,
        is_mayhem_mode: false,
        is_cashback_coin: false,
        is_holder_reward: false,
        creator_fee_bps: 0,
        transfer_fee_bps: 0,
        virtual_token_reserves: Some(1_073_000_000_000_000),
        virtual_quote_reserves: Some(30_000_000_000),
        real_token_reserves: Some(793_100_000_000_000),
    }
}

fn extract_msg_writable_accounts_with_alts(
    msg: &solana_sdk::message::v0::Message,
    alts: &[solana_sdk::address_lookup_table::AddressLookupTableAccount],
) -> Vec<String> {
    let mut writables: Vec<String> = msg.account_keys
        .iter()
        .enumerate()
        .filter(|(idx, _)| msg.is_maybe_writable(*idx, None))
        .map(|(_, k)| k.to_string())
        .collect();

    for lookup in &msg.address_table_lookups {
        if let Some(table) = alts.iter().find(|a| a.key == lookup.account_key) {
            for &idx in &lookup.writable_indexes {
                if let Some(pk) = table.addresses.get(idx as usize) {
                    let s = pk.to_string();
                    if !writables.contains(&s) {
                        writables.push(s);
                    }
                }
            }
        }
    }
    writables
}

fn extract_msg_writable_accounts(msg: &solana_sdk::message::v0::Message) -> Vec<String> {
    extract_msg_writable_accounts_with_alts(msg, &[])
}

fn build_valid_pump_candidate(
    signer: &Pubkey,
    mint: &Pubkey,
    treasury: &Pubkey,
) -> PreparedSwapCandidate {
    let token_info = mock_token(*mint);
    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
    instructions.push(system_instruction::transfer(signer, treasury, fee_lamports));
    instructions.extend(build_buy_v2_instructions(
        &token_info,
        signer,
        30_000_000,
        1_000_000_000,
    ));

    let msg = Message::try_compile(signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
            protocol_pump::addresses::PUMP_PROGRAM.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 2_039_280,
        total_sol_required_atomic: 1_000_000_000 + fee_lamports + 2_039_280,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 500, // 500ms old
    };

    PreparedSwapCandidate {
        candidate_id: "candidate_test_pump_1".to_string(),
        selected_route_source: "pump_bonding_curve".to_string(),
        venue: "Pump.fun Bonding Curve".to_string(),
        pool_address: None,
        input_mint: WSOL_MINT_STR.to_string(),
        output_mint: mint.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 33_000_000,
        minimum_output_atomic: 30_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    }
}

#[test]
fn test_valid_pump_candidate_passes_validation() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let candidate = build_valid_pump_candidate(&signer, &mint, &treasury);
    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Valid candidate should pass: {:?}", res.err());
    let review = res.unwrap();
    assert_eq!(review.venue, "Pump.fun Bonding Curve");
    assert_eq!(review.side, "Buy");
    assert!(review.platform_fee_recipient_verified);
}

#[test]
fn test_valid_pumpswap_candidate_passes_validation() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let pool = PumpSwapPoolState {
        pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
        base_mint: mint,
        quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
        base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
        quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
        coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 30_000_000_000,
        creator_fee_bps: 0,
    };

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));
    instructions.extend(build_pumpswap_buy_instructions(
        &pool,
        &signer,
        25_000_000,
        1_000_000_000,
        &spl_token::id(),
        &spl_token::id(),
    ));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string()],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: Some(25_000_000),
        estimated_rent_lamports: 2_039_280,
        total_sol_required_atomic: 1_000_000_000 + fee_lamports + 2_039_280,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 1_200,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pumpswap_1".to_string(),
        selected_route_source: "pumpswap".to_string(),
        venue: "PumpSwap AMM".to_string(),
        pool_address: Some(pool.pool_address.to_string()),
        input_mint: WSOL_MINT_STR.to_string(),
        output_mint: mint.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 27_000_000,
        minimum_output_atomic: 25_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Valid PumpSwap candidate should pass: {:?}", res.err());
}

#[test]
fn test_adversarial_wrong_treasury_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let attacker_treasury = Pubkey::new_unique();

    // Build candidate that transfers platform fee to attacker instead of CONFIGURED_PLATFORM_TREASURY
    let candidate = build_valid_pump_candidate(&signer, &mint, &attacker_treasury);
    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Candidate with wrong treasury must be rejected");
    match res.unwrap_err() {
        ValidationError::UnexpectedSystemTransfer(msg) => {
            assert!(msg.contains("Unauthorized transfer"));
        }
        ValidationError::FeeRecipientMismatch { expected, actual } => {
            assert_eq!(expected, CONFIGURED_PLATFORM_TREASURY);
            assert_ne!(actual, CONFIGURED_PLATFORM_TREASURY);
        }
        other => panic!("Unexpected error type: {:?}", other),
    }
}

#[test]
fn test_adversarial_altered_mint_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let real_mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let tampered_mint = pubkey!("8wXtBvZgSttG6WfB1q2eYoXg5Z5G4vVz7k6e4s8t4Q1A");

    let mut candidate = build_valid_pump_candidate(&signer, &real_mint, &CONFIGURED_PLATFORM_TREASURY);
    // Tamper output mint in candidate metadata
    candidate.output_mint = tampered_mint.to_string();

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::MintMismatch { .. }));
}

#[test]
fn test_adversarial_extra_signer_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let extra_signer = Pubkey::new_unique();

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    // Inject an instruction requiring extra signer
    instructions.push(Instruction {
        program_id: protocol_pump::addresses::PUMP_PROGRAM,
        accounts: vec![
            AccountMeta::new(extra_signer, true), // extra signer!
        ],
        data: vec![0u8; 8],
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let tx = VersionedTransaction {
        signatures: vec![
            solana_sdk::signature::Signature::default(),
            solana_sdk::signature::Signature::default(),
        ],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::UnexpectedSigner(_)));
}

#[test]
fn test_adversarial_token_approval_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let delegate = Pubkey::new_unique();

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    // Inject malicious SPL Token Approve instruction (Instruction discriminator 4 = Approve)
    let approve_ix = spl_token::instruction::approve(
        &spl_token::id(),
        &signer,
        &delegate,
        &signer,
        &[],
        u64::MAX,
    ).unwrap();
    instructions.push(approve_ix);

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    match res.unwrap_err() {
        ValidationError::DangerousInstruction(msg) => {
            assert!(msg.contains("Approve instruction is strictly forbidden"));
        }
        other => panic!("Expected DangerousInstruction, got {:?}", other),
    }
}

#[test]
fn test_adversarial_token_set_authority_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let attacker = Pubkey::new_unique();

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    // Inject malicious SPL Token SetAuthority instruction (discriminator 6)
    let set_auth_ix = spl_token::instruction::set_authority(
        &spl_token::id(),
        &mint,
        Some(&attacker),
        spl_token::instruction::AuthorityType::AccountOwner,
        &signer,
        &[],
    ).unwrap();
    instructions.push(set_auth_ix);

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    match res.unwrap_err() {
        ValidationError::DangerousInstruction(msg) => {
            assert!(msg.contains("SetAuthority instruction is strictly forbidden"));
        }
        ValidationError::UnauthorizedWritableAccount(pk) => {
            assert_eq!(pk, mint);
        }
        other => panic!("Expected DangerousInstruction or UnauthorizedWritableAccount, got {:?}", other),
    }
}

#[test]
fn test_adversarial_forbidden_program_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let drain_program = Pubkey::new_unique();

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    // Inject unauthorized program invocation
    instructions.push(Instruction {
        program_id: drain_program,
        accounts: vec![AccountMeta::new(signer, true)],
        data: vec![1, 2, 3],
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::ForbiddenProgram(_)));
}

#[test]
fn test_adversarial_unknown_lookup_table_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    let mut msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    // Inject an Address Table Lookup
    msg.address_table_lookups = vec![MessageAddressTableLookup {
        account_key: Pubkey::new_unique(),
        writable_indexes: vec![0],
        readonly_indexes: vec![1],
    }];

    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::UnknownLookupTable(_)));
}

#[test]
fn test_adversarial_stale_quote_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    // Quote is 65 seconds old
    candidate.summary.quote_freshness_ms = 65_000;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::ExpiredQuote(_)));
}

#[test]
fn test_adversarial_malformed_bytes_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.unsigned_tx_base64 = "this_is_not_valid_serialized_transaction_base64".to_string();

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(
        res.unwrap_err(),
        ValidationError::Base64Decode(_) | ValidationError::TransactionDeserialize(_)
    ));
}

#[test]
fn test_adversarial_durable_nonce_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let nonce_account = Pubkey::new_unique();

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    // Inject AdvanceNonceAccount instruction (System instruction type 4)
    instructions.push(system_instruction::advance_nonce_account(&nonce_account, &signer));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    match res.unwrap_err() {
        ValidationError::DangerousInstruction(msg) => {
            assert!(msg.contains("Durable nonce instruction is forbidden"));
        }
        other => panic!("Expected DangerousInstruction, got {:?}", other),
    }
}

#[tokio::test]
async fn test_adversarial_simulation_failure_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    // Point to an unreachable RPC port to verify simulation fails closed
    let unreachable_rpc = "http://127.0.0.1:54321";
    let res = transaction_validation::validator::validate_and_simulate_swap_candidate(
        &candidate,
        &signer,
        &fee_config,
        unreachable_rpc,
    )
    .await;

    assert!(!res.is_valid, "Simulation failure must cause candidate to be invalid");
    assert!(!res.is_simulated, "Should not be marked simulated");
    assert!(!res.validation_errors.is_empty());
    assert!(
        res.validation_errors
            .iter()
            .any(|e| e.contains("RPC Simulation Reverted / Failed")),
        "Expected RPC simulation error in validation_errors: {:?}",
        res.validation_errors
    );
}

#[test]
fn test_adversarial_altered_minimum_output_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    // Increase minimum_output_atomic above what is encoded in the instruction (30_000_000)
    candidate.minimum_output_atomic = 35_000_000;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::OutputTooLow { .. }));
}

#[test]
fn test_adversarial_altered_spend_amount_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    // Lower candidate input_amount_atomic below what the instruction spends (1_000_000_000)
    candidate.input_amount_atomic = 900_000_000;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::SpendAmountExceeded { .. }));
}

#[test]
fn test_adversarial_unauthorized_writable_program_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let token_info = mock_token(mint);
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 2_000_000));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    // Try to mark target mint as writable (privilege escalation attempt)
    instructions.push(Instruction {
        program_id: protocol_pump::addresses::PUMP_PROGRAM,
        accounts: vec![AccountMeta::new(mint, false)],
        data: vec![0u8; 8],
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg.clone()),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.unsigned_tx_base64 = b64;
    // Disclose all writable accounts including the escalatory system_program
    candidate.summary.writable_accounts = extract_msg_writable_accounts(&msg);

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::UnauthorizedWritableAccount(_)));
}

#[test]
fn test_adversarial_writable_account_discrepancy_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    // Remove accounts from declared summary to simulate hiding writable state
    candidate.summary.writable_accounts = vec![signer.to_string()];

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), ValidationError::WritableAccountMismatch(_)));
}

#[test]
fn test_adversarial_sell_fee_before_swap_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(30_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    // Malicious ordering for sell: transfer fee BEFORE swap executes (charging upfront SOL instead of from proceeds)
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.extend(protocol_pump::instructions::build_sell_v2_instructions(
        &token_info,
        &signer,
        1_000_000_000,
        30_000_000,
    ));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
            protocol_pump::addresses::PUMP_PROGRAM.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: fee_lamports,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 500,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pump_sell_1".to_string(),
        selected_route_source: "pump_bonding_curve".to_string(),
        venue: "Pump.fun Bonding Curve".to_string(),
        pool_address: None,
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 32_000_000,
        minimum_output_atomic: 29_940_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Sell ordering fee before swap must fail");
    assert!(matches!(res.unwrap_err(), ValidationError::InstructionOrderingViolation(_)));
}

#[test]
fn test_valid_sell_candidate_deducts_fee_from_proceeds() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(30_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    // Correct sell ordering: sell execution FIRST, fee transfer SECOND
    instructions.extend(protocol_pump::instructions::build_sell_v2_instructions(
        &token_info,
        &signer,
        1_000_000_000,
        30_000_000,
    ));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::PUMP_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 500,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pump_sell_valid".to_string(),
        selected_route_source: "pump_bonding_curve".to_string(),
        venue: "Pump.fun Bonding Curve".to_string(),
        pool_address: None,
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 32_000_000,
        minimum_output_atomic: 29_940_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Valid sell candidate with fee deduction must pass: {:?}", res.err());
}

#[test]
fn test_pumpswap_with_token_2022_vault_program() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let pool = PumpSwapPoolState {
        pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
        base_mint: mint,
        quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
        base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
        quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
        coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 30_000_000_000,
        creator_fee_bps: 50, // 50 bps creator fee
    };

    // Test active fee derivation: 25 bps base canonical AMM fee + 50 bps creator fee = 75 bps
    assert_eq!(pool.derive_active_fee_bps(Some(25)), 75);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));

    // Base token uses TOKEN_2022_PROGRAM, Quote (WSOL) uses spl_token::id()
    let token_2022 = protocol_pump::addresses::TOKEN_2022_PROGRAM;
    let spl_token = spl_token::id();

    let pumpswap_ixs = build_pumpswap_buy_instructions(
        &pool,
        &signer,
        25_000_000,
        1_000_000_000,
        &token_2022,
        &spl_token,
    );
    // Verify that the first instruction creates ATA with TOKEN_2022_PROGRAM
    assert_eq!(pumpswap_ixs[0].program_id, protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM);
    // Verify that the swap instruction includes TOKEN_2022_PROGRAM in its account metas
    let swap_ix = pumpswap_ixs
        .iter()
        .find(|ix| ix.program_id == protocol_pumpswap::addresses::PUMP_AMM_PROGRAM)
        .expect("PumpSwap AMM buy instruction present");
    assert!(swap_ix.accounts.iter().any(|meta| meta.pubkey == token_2022));

    instructions.extend(pumpswap_ixs);

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
            protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM.to_string(),
            protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: Some(7_500_000), // 75 bps
        estimated_rent_lamports: 2_039_280,
        total_sol_required_atomic: 1_000_000_000 + fee_lamports + 2_039_280,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 1_200,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pumpswap_token_2022".to_string(),
        selected_route_source: "pumpswap".to_string(),
        venue: "PumpSwap AMM".to_string(),
        pool_address: Some(pool.pool_address.to_string()),
        input_mint: WSOL_MINT_STR.to_string(),
        output_mint: mint.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 27_000_000,
        minimum_output_atomic: 25_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "PumpSwap candidate with Token-2022 vault program must pass validation: {:?}", res.err());
}

#[test]
fn test_unsupported_okx_candidate_rejected_by_validator() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));

    // OKX Router instruction without a valid Pump or PumpSwap trade instruction
    let okx_ix = Instruction {
        program_id: protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(signer, true),
            AccountMeta::new_readonly(mint, false),
        ],
        data: vec![0u8; 32],
    };
    instructions.push(okx_ix);

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
            protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 1_000_000_000 + fee_lamports,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 500,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_okx_rejected".to_string(),
        selected_route_source: "okx_router".to_string(),
        venue: "OKX DEX Router".to_string(),
        pool_address: None,
        input_mint: WSOL_MINT_STR.to_string(),
        output_mint: mint.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 25_000_000,
        minimum_output_atomic: 24_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    // Must be rejected because OKX routes and instructions are unsupported in Phase 3
    assert!(res.is_err());
    match res.unwrap_err() {
        ValidationError::InvalidIntent(_) | ValidationError::DangerousInstruction(_) => {}
        other => panic!("Expected InvalidIntent or DangerousInstruction, got {:?}", other),
    }
}

#[test]
fn test_valid_pumpswap_sell_candidate_unwraps_wsol_and_deducts_fee() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let pool = PumpSwapPoolState {
        pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
        base_mint: mint,
        quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
        base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
        quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
        coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 30_000_000_000,
        creator_fee_bps: 0,
    };

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(51_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));

    // 1. Sell instructions: creates WSOL ATA, executes AMM sell, and unwraps WSOL to native SOL in user wallet
    instructions.extend(build_pumpswap_sell_instructions(
        &pool,
        &signer,
        500_000,
        51_000_000,
        &spl_token::id(),
        &spl_token::id(),
    ));

    // 2. Fee deduction transfer runs AFTER proceeds have arrived in wallet
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM.to_string(),
            protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string(),
            spl_token::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: Some(1_250_000),
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 1_200,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pumpswap_sell_valid".to_string(),
        selected_route_source: "pumpswap".to_string(),
        venue: "PumpSwap AMM".to_string(),
        pool_address: Some(pool.pool_address.to_string()),
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 500_000,
        estimated_output_atomic: 52_000_000,
        minimum_output_atomic: 50_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Valid PumpSwap sell candidate with WSOL unwrap must pass: {:?}", res.err());
}

#[test]
fn test_pumpswap_sell_without_wsol_unwrap_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let pool = PumpSwapPoolState {
        pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
        base_mint: mint,
        quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
        base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
        quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
        coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 30_000_000_000,
        creator_fee_bps: 0,
    };

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(51_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));

    // Manually build instructions WITHOUT close_account unwrap step
    let mut sell_ixs = build_pumpswap_sell_instructions(
        &pool,
        &signer,
        500_000,
        51_000_000,
        &spl_token::id(),
        &spl_token::id(),
    );
    // Remove the close_account instruction (the 3rd instruction)
    sell_ixs.pop();
    instructions.extend(sell_ixs);

    // Fee transfer directly from wallet (which would drain existing SOL)
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM.to_string(),
            protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: Some(1_250_000),
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 1_200,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pumpswap_sell_no_unwrap".to_string(),
        selected_route_source: "pumpswap".to_string(),
        venue: "PumpSwap AMM".to_string(),
        pool_address: Some(pool.pool_address.to_string()),
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 500_000,
        estimated_output_atomic: 52_000_000,
        minimum_output_atomic: 50_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "PumpSwap sell without WSOL unwrap must fail validation");
    match res.unwrap_err() {
        ValidationError::InstructionOrderingViolation(msg) => {
            assert!(msg.contains("close_account"));
        }
        other => panic!("Expected InstructionOrderingViolation, got {:?}", other),
    }
}

#[test]
fn test_pumpswap_fee_config_parsing_and_dynamic_tier_selection() {
    use protocol_pumpswap::pool::{PumpSwapFeeConfig, PUMP_FEE_CONFIG_DISCRIMINATOR};

    let mut data = Vec::new();
    // Discriminator
    data.extend_from_slice(&PUMP_FEE_CONFIG_DISCRIMINATOR);
    // Bump
    data.push(254);
    // Admin (32 bytes)
    let admin = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    data.extend_from_slice(admin.as_ref());
    // Flat fees: lp_fee_bps: 20, protocol_fee_bps: 5, creator_fee_bps: 0
    data.extend_from_slice(&20u64.to_le_bytes());
    data.extend_from_slice(&5u64.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());

    // Fee tiers vector length: 2 tiers
    data.extend_from_slice(&2u32.to_le_bytes());
    // Tier 1: threshold = 100 SOL (100_000_000_000 lamports), lp_fee = 20, proto = 5 (total 25 bps)
    data.extend_from_slice(&100_000_000_000u128.to_le_bytes());
    data.extend_from_slice(&20u64.to_le_bytes());
    data.extend_from_slice(&5u64.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());

    // Tier 2: threshold = 1000 SOL (1_000_000_000_000 lamports), lp_fee = 10, proto = 5 (total 15 bps)
    data.extend_from_slice(&1_000_000_000_000u128.to_le_bytes());
    data.extend_from_slice(&10u64.to_le_bytes());
    data.extend_from_slice(&5u64.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());

    // 1. Verify successful parse
    let fee_cfg = PumpSwapFeeConfig::try_from_slice(&data).expect("FeeConfig parse should succeed");
    assert_eq!(fee_cfg.bump, 254);
    assert_eq!(fee_cfg.admin, admin);
    assert_eq!(fee_cfg.flat_fees.lp_fee_bps, 20);
    assert_eq!(fee_cfg.flat_fees.protocol_fee_bps, 5);
    assert_eq!(fee_cfg.fee_tiers.len(), 2);

    // 2. Test tier selection
    // Below 100 SOL: selects Tier 1 (25 bps)
    let (base_50sol, _) = fee_cfg.select_fee_tier(50_000_000_000);
    assert_eq!(base_50sol, 25);

    // Above 1000 SOL: selects Tier 2 (15 bps)
    let (base_1500sol, _) = fee_cfg.select_fee_tier(1_500_000_000_000);
    assert_eq!(base_1500sol, 15);

    // 3. Test invalid discriminator rejection
    let mut invalid_data = data.clone();
    invalid_data[0] = 0;
    assert!(PumpSwapFeeConfig::try_from_slice(&invalid_data).is_err());
}

#[test]
fn test_adversarial_pumpswap_sell_closing_wrong_account_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let fake_token_account = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let pool = PumpSwapPoolState {
        pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
        base_mint: mint,
        quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
        base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
        quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
        coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 30_000_000_000,
        creator_fee_bps: 0,
    };

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(51_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));

    let mut sell_ixs = build_pumpswap_sell_instructions(
        &pool,
        &signer,
        500_000,
        51_000_000,
        &spl_token::id(),
        &spl_token::id(),
    );
    // Replace the close_account instruction with one targeting a fake token account instead of the WSOL ATA
    sell_ixs.pop();
    sell_ixs.push(
        spl_token::instruction::close_account(
            &spl_token::id(),
            &fake_token_account,
            &signer,
            &signer,
            &[],
        ).unwrap(),
    );
    instructions.extend(sell_ixs);
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM.to_string(),
            protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string(),
            spl_token::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: Some(1_250_000),
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 1_200,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pumpswap_sell_fake_close".to_string(),
        selected_route_source: "pumpswap".to_string(),
        venue: "PumpSwap AMM".to_string(),
        pool_address: Some(pool.pool_address.to_string()),
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 500_000,
        estimated_output_atomic: 52_000_000,
        minimum_output_atomic: 50_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Closing arbitrary token account instead of expected WSOL ATA must fail validation");
    match res.unwrap_err() {
        ValidationError::DangerousInstruction(msg) => {
            assert!(msg.contains("does not match expected WSOL ATA"));
        }
        other => panic!("Expected DangerousInstruction, got {:?}", other),
    }
}

#[test]
fn test_adversarial_pumpswap_sell_insufficient_proceeds_to_cover_fee_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let treasury = CONFIGURED_PLATFORM_TREASURY;

    let pool = PumpSwapPoolState {
        pool_address: pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"),
        base_mint: mint,
        quote_mint: pubkey!("So11111111111111111111111111111111111111112"),
        base_account: pubkey!("9wFFyRfZBsuAha4YcKUcAf3WqHnRgcjCXaftiNVnVip"),
        quote_account: pubkey!("GThUX1Atko4tqhN2NaiTazWSeFWMuiUvfFnyJyUghFMJ"),
        coin_creator: pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        virtual_quote_reserves: 30_000_000_000,
        creator_fee_bps: 0,
    };

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(50_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));

    // AMM sell instruction only guarantees 50_000_000 lamports gross,
    // which cannot satisfy net minimum 50_000_000 + fee 100_000 = 50_100_000 lamports.
    // Executing this would force the fee transfer to supplement 100_000 lamports from pre-existing wallet SOL.
    instructions.extend(build_pumpswap_sell_instructions(
        &pool,
        &signer,
        500_000,
        50_000_000, // Insufficient gross proceeds!
        &spl_token::id(),
        &spl_token::id(),
    ));
    instructions.push(system_instruction::transfer(&signer, &treasury, fee_lamports));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::ASSOCIATED_TOKEN_PROGRAM.to_string(),
            protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string(),
            spl_token::id().to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: Some(1_250_000),
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 1_200,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pumpswap_sell_insufficient_proceeds".to_string(),
        selected_route_source: "pumpswap".to_string(),
        venue: "PumpSwap AMM".to_string(),
        pool_address: Some(pool.pool_address.to_string()),
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 500_000,
        estimated_output_atomic: 52_000_000,
        minimum_output_atomic: 50_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: treasury,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Sell with insufficient proceeds to cover fee and net minimum must be rejected");
    match res.unwrap_err() {
        ValidationError::OutputTooLow { min_out, actual } => {
            assert_eq!(min_out, 50_100_000, "Required gross min out must be 50_100_000");
            assert_eq!(actual, 50_000_000, "Actual encoded gross min out was 50_000_000");
        }
        other => panic!("Expected OutputTooLow, got {:?}", other),
    }
}

#[test]
fn test_adversarial_pump_sell_insufficient_proceeds_to_cover_fee_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(29_000_000, fee_bps).unwrap();
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));

    // Bonding curve sell instruction guarantees 29_000_000 SOL gross,
    // but candidate requires 29_000_000 net min + 58_000 fee = 29_058_000 gross.
    instructions.extend(protocol_pump::instructions::build_sell_v2_instructions(
        &token_info,
        &signer,
        1_000_000_000,
        29_000_000, // Insufficient gross proceeds!
    ));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::PUMP_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 500,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pump_sell_insufficient".to_string(),
        selected_route_source: "pump_bonding_curve".to_string(),
        venue: "Pump.fun Bonding Curve".to_string(),
        pool_address: None,
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 32_000_000,
        minimum_output_atomic: 29_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-27T20:00:00Z".to_string(),
        expires_at: "2026-10-27T20:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Pump sell with insufficient proceeds to cover fee must be rejected");
    match res.unwrap_err() {
        ValidationError::OutputTooLow { min_out, actual } => {
            assert_eq!(min_out, 29_058_000, "Required gross min out must be 29_058_000");
            assert_eq!(actual, 29_000_000, "Actual encoded gross min out was 29_000_000");
        }
        other => panic!("Expected OutputTooLow, got {:?}", other),
    }
}

#[test]
fn test_adversarial_oversized_fee_transfer_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();
    let oversized_fee_lamports = 3_000_000;

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    // Transfer MORE than platform_fee_atomic to treasury
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, oversized_fee_lamports));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.summary.platform_fee_atomic = fee_lamports;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Oversized fee transfer must be rejected");
    match res.unwrap_err() {
        ValidationError::FeeAmountMismatch { expected, actual } => {
            assert_eq!(expected, fee_lamports);
            assert_eq!(actual, oversized_fee_lamports);
        }
        other => panic!("Expected FeeAmountMismatch, got {:?}", other),
    }
}

#[test]
fn test_adversarial_duplicate_fee_transfer_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    // Fee transfer 1
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    // Duplicate fee transfer 2
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.summary.platform_fee_atomic = fee_lamports;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Duplicate fee transfer must be rejected");
    match res.unwrap_err() {
        ValidationError::DangerousInstruction(msg) => {
            assert!(msg.contains("Duplicate platform fee transfer"));
        }
        other => panic!("Expected DangerousInstruction with Duplicate, got {:?}", other),
    }
}

#[test]
fn test_adversarial_wrong_source_fee_transfer_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps();
    let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_bps).unwrap();

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    // Fee transfer compiled initially with signer
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    let mut msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    // Tamper with the fee transfer instruction: change its from_pubkey account index to point to mint
    let mint_idx = msg.account_keys.iter().position(|k| *k == mint).unwrap() as u8;
    msg.instructions[1].accounts[0] = mint_idx;

    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.summary.platform_fee_atomic = fee_lamports;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Fee transfer from wrong source must be rejected");
    match res.unwrap_err() {
        ValidationError::SignerMismatch { expected, actual } => {
            assert_eq!(expected, signer);
            assert_eq!(actual, mint);
        }
        other => panic!("Expected SignerMismatch, got {:?}", other),
    }
}

#[test]
fn test_adversarial_candidate_raised_both_summary_fee_and_transfer_rejected_by_native_fee_calculation() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    // Attacker inflates fee from 2_000_000 (20 bps on 1 SOL) to 50_000_000 (500 bps)
    // in BOTH the transaction transfer AND the candidate summary metadata, attempting to trick
    // a naive validator that only checks transfer == candidate.summary.platform_fee_atomic.
    let inflated_fee = 50_000_000;

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, inflated_fee));
    instructions.extend(build_buy_v2_instructions(&token_info, &signer, 30_000_000, 1_000_000_000));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    candidate.summary.writable_accounts = writable_accounts;
    candidate.summary.platform_fee_atomic = inflated_fee;
    candidate.unsigned_tx_base64 = b64;

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default, // 20 bps -> 2_000_000 lamports for 1 SOL input
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Inflated fee matching summary and transfer must be rejected by native fee calculation");
    match res.unwrap_err() {
        ValidationError::FeeAmountMismatch { expected, actual } => {
            assert_eq!(expected, 2_000_000, "Native validator independently calculated allowed fee as 2_000_000");
            assert_eq!(actual, inflated_fee, "Candidate summary had 50_000_000");
        }
        other => panic!("Expected FeeAmountMismatch, got {:?}", other),
    }
}

#[test]
fn test_adversarial_candidate_tampered_blockhash_mismatch_rejected() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");

    let mut candidate = build_valid_pump_candidate(&signer, &mint, &CONFIGURED_PLATFORM_TREASURY);
    // Tamper metadata blockhash to a fresh blockhash while the serialized transaction contains Hash::default()
    let fake_blockhash = solana_sdk::hash::Hash::new_unique();
    let original_blockhash = candidate.recent_blockhash.clone();
    candidate.recent_blockhash = fake_blockhash.to_string();

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_err(), "Blockhash discrepancy between metadata and message must be rejected");
    match res.unwrap_err() {
        ValidationError::BlockhashMismatch { expected, actual } => {
            assert_eq!(expected, fake_blockhash.to_string());
            assert_eq!(actual, original_blockhash);
        }
        other => panic!("Expected BlockhashMismatch, got {:?}", other),
    }
}

#[test]
fn test_sell_candidate_favorable_price_movement_aligned_fee_passes_native_validation() {
    let signer = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");
    let mint = pubkey!("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R");
    let token_info = mock_token(mint);

    let fee_bps = FeeTier::Default.rate_bps(); // 20 bps

    // 1. User quote requested net minimum: 29_940_000 lamports
    let quoted_net_min = 29_940_000u64;

    // 2. Favorable price movement occurred between quote and preparation:
    // Fresh curve output is higher: fresh gross min = 45_000_000 lamports
    let fresh_gross_min = 45_000_000u64;
    let fresh_fee = shared_types::fee::compute_fee(fresh_gross_min, fee_bps).unwrap();
    let calculated_net_min = fresh_gross_min.saturating_sub(fresh_fee);
    assert!(calculated_net_min >= quoted_net_min, "Price movement is favorable");

    // 3. Align calculation: gross minimum bound is derived directly from the bound net minimum
    let divisor = 10_000u128 - fee_bps as u128;
    let gross_u128 = (quoted_net_min as u128 * 10_000u128 + (divisor - 1)) / divisor;
    let gross_min_bound = u64::try_from(gross_u128).unwrap();
    let aligned_fee = shared_types::fee::compute_fee(gross_min_bound, fee_bps).unwrap();

    // Verify mathematical invariant: gross_min_bound - aligned_fee >= quoted_net_min
    assert!(gross_min_bound - aligned_fee >= quoted_net_min);

    // 4. Build candidate with aligned fee and gross_min_bound
    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.extend(protocol_pump::instructions::build_sell_v2_instructions(
        &token_info,
        &signer,
        1_000_000_000,
        gross_min_bound,
    ));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, aligned_fee));

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            solana_sdk::compute_budget::id().to_string(),
            protocol_pump::addresses::PUMP_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 0,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: aligned_fee,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 0,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 250,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_test_pump_sell_favorable".to_string(),
        selected_route_source: "pump_bonding_curve".to_string(),
        venue: "Pump.fun Bonding Curve".to_string(),
        pool_address: None,
        input_mint: mint.to_string(),
        output_mint: WSOL_MINT_STR.to_string(),
        input_amount_atomic: 1_000_000_000,
        estimated_output_atomic: 48_000_000,
        minimum_output_atomic: quoted_net_min,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    // Native validation MUST pass with aligned fee calculation!
    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Aligned sell calculation must pass native validation after favorable price movement: {:?}", res);
    let preview = res.unwrap();
    assert!(preview.platform_fee_recipient_verified);
    assert_eq!(
        preview.authorization_status,
        "Verified & Simulated (Signing & Broadcast Disabled in Phase 3)"
    );
}

fn create_jupiter_route_ix_data(in_amount: u64, quoted_out_amount: u64, slippage_bps: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(47);
    data.extend_from_slice(&transaction_validation::validator::JUPITER_ROUTE_DISCRIMINATOR);
    // Borsh Vec<RoutePlanStep>: length 1 (4 bytes u32) + 16 bytes step dummy data
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&[0u8; 16]);
    // Trailing parameters: in_amount (8), quoted_out_amount (8), slippage_bps (2), platform_fee_bps (1)
    data.extend_from_slice(&in_amount.to_le_bytes());
    data.extend_from_slice(&quoted_out_amount.to_le_bytes());
    data.extend_from_slice(&slippage_bps.to_le_bytes());
    data.push(0u8);
    data
}

#[test]
fn test_valid_jupiter_candidate_passes_validation() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64; // 20 bps on 10,000,000
    let swap_lamports = 9_980_000u64;

    // Create an Address Lookup Table with pool accounts that are actually referenced by the transaction
    let alt_key = Pubkey::new_unique();
    let pool_state_account = Pubkey::new_unique();
    let amm_authority = Pubkey::new_unique();
    let alt_account = solana_sdk::address_lookup_table::AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![pool_state_account, amm_authority],
    };

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
            // Referenced accounts that get loaded from the ALT
            AccountMeta::new(pool_state_account, false),
            AccountMeta::new_readonly(amm_authority, false),
        ],
        data: create_jupiter_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });
    instructions.push(spl_token::instruction::close_account(
        &spl_token::id(),
        &wsol_ata,
        &signer,
        &signer,
        &[],
    ).unwrap());

    let msg = Message::try_compile(&signer, &instructions, &[alt_account.clone()], Hash::default()).unwrap();
    // Verify that the compiled v0 message actually contains address table lookups with loaded accounts
    assert_eq!(msg.address_table_lookups.len(), 1);
    assert_eq!(msg.address_table_lookups[0].account_key, alt_key);
    assert_eq!(msg.address_table_lookups[0].writable_indexes, vec![0]);
    assert_eq!(msg.address_table_lookups[0].readonly_indexes, vec![1]);

    let writable_accounts = extract_msg_writable_accounts_with_alts(&msg, &[alt_account.clone()]);
    assert!(writable_accounts.contains(&pool_state_account.to_string()), "Loaded writable account must be present in writable_accounts");

    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            transaction_validation::allowlist::COMPUTE_BUDGET_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
            transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
            spl_token::id().to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: 20,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 10_000_000 + fee_lamports,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 100,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_test".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: vec![
            shared_types::swap::AddressLookupTableEntry {
                key: alt_account.key.to_string(),
                addresses: alt_account.addresses.iter().map(|p| p.to_string()).collect(),
            }
        ],
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Jupiter candidate with ALTs and native fee must pass validation: {:?}", res);
    let preview = res.unwrap();
    assert!(preview.platform_fee_recipient_verified);
    assert_eq!(preview.venue, "Jupiter (Raydium CLMM)");
}

#[test]
fn test_adversarial_jupiter_alt_privilege_escalation_system_program_writable_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    // Adversarial ALT includes PUMP_PROGRAM as a loaded address
    let alt_key = Pubkey::new_unique();
    let alt_account = solana_sdk::address_lookup_table::AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![protocol_pump::addresses::PUMP_PROGRAM],
    };

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
            // Privilege escalation: Allowed program marked writable via ALT!
            AccountMeta::new(protocol_pump::addresses::PUMP_PROGRAM, false),
        ],
        data: create_jupiter_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[alt_account.clone()], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts_with_alts(&msg, &[alt_account.clone()]);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            transaction_validation::allowlist::COMPUTE_BUDGET_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
            transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
            spl_token::id().to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: 20,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 10_000_000 + fee_lamports,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 100,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_exploit_alt".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: vec![
            shared_types::swap::AddressLookupTableEntry {
                key: alt_account.key.to_string(),
                addresses: alt_account.addresses.iter().map(|p| p.to_string()).collect(),
            }
        ],
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::UnauthorizedWritableAccount(k)) if k == protocol_pump::addresses::PUMP_PROGRAM),
        "ALT writable program account must be rejected as privilege escalation: {:?}",
        res
    );
}

#[test]
fn test_adversarial_jupiter_alt_privilege_escalation_target_mint_writable_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    // Adversarial ALT includes target_mint (bonk_mint) as a loaded address
    let alt_key = Pubkey::new_unique();
    let alt_account = solana_sdk::address_lookup_table::AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![bonk_mint],
    };

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
            // Privilege escalation: Target mint marked writable via ALT!
            AccountMeta::new(bonk_mint, false),
        ],
        data: create_jupiter_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[alt_account.clone()], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts_with_alts(&msg, &[alt_account.clone()]);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            transaction_validation::allowlist::COMPUTE_BUDGET_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
            transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
            spl_token::id().to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: 20,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 10_000_000 + fee_lamports,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 100,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_exploit_mint".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: vec![
            shared_types::swap::AddressLookupTableEntry {
                key: alt_account.key.to_string(),
                addresses: alt_account.addresses.iter().map(|p| p.to_string()).collect(),
            }
        ],
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::UnauthorizedWritableAccount(k)) if k == bonk_mint),
        "ALT writable target mint must be rejected as privilege escalation: {:?}",
        res
    );
}

#[test]
fn test_adversarial_jupiter_unprovided_alt_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let alt_key = Pubkey::new_unique();
    let alt_account = solana_sdk::address_lookup_table::AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![Pubkey::new_unique()],
    };

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, 20_000));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, 9_980_000));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
            AccountMeta::new_readonly(alt_account.addresses[0], false),
        ],
        data: create_jupiter_route_ix_data(9_980_000, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[alt_account.clone()], Hash::default()).unwrap();
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_missing_alt".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary: SwapCandidateSummary {
            instructions: Vec::new(),
            program_ids: vec![transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string()],
            writable_accounts: vec![signer.to_string(), wsol_ata.to_string(), bonk_ata.to_string(), CONFIGURED_PLATFORM_TREASURY.to_string()],
            signer_accounts: vec![signer.to_string()],
            compute_unit_limit: 200_000,
            priority_fee_micro_lamports: 50_000,
            platform_fee_bps: 20,
            platform_fee_atomic: 20_000,
            platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 0,
            total_sol_required_atomic: 10_000_000 + 20_000,
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        },
        address_lookup_tables: Vec::new(), // Intentionally missing ALT!
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::UnknownLookupTable(_))),
        "Missing ALT must result in UnknownLookupTable rejection: {:?}",
        res
    );
}

#[test]
fn test_adversarial_jupiter_instruction_altered_amount_in_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let tampered_swap_lamports = 10_000_000u64; // Attacker tries to swap 10,000,000 without deducting 20,000 fee

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, 9_980_000));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
        ],
        data: create_jupiter_route_ix_data(tampered_swap_lamports, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_tampered_in".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary: SwapCandidateSummary {
            instructions: Vec::new(),
            program_ids: vec![
                solana_sdk::compute_budget::id().to_string(),
                solana_sdk::system_program::ID.to_string(),
                transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
                spl_token::id().to_string(),
            ],
            writable_accounts,
            signer_accounts: vec![signer.to_string()],
            compute_unit_limit: 200_000,
            priority_fee_micro_lamports: 50_000,
            platform_fee_bps: 20,
            platform_fee_atomic: fee_lamports,
            platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 0,
            total_sol_required_atomic: 10_000_000 + fee_lamports,
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        },
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::InvalidIntent(_))),
        "Altered in_amount not accounting for fee must be rejected: {:?}",
        res
    );
}

#[test]
fn test_adversarial_jupiter_instruction_lower_minimum_output_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    // Quoted out is set to 30,000,000,000 which with 100 bps slippage gives min out = 29,700,000,000 < candidate minimum 31,500,000,000!
    let tampered_quoted_out = 30_000_000_000u64;

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
        ],
        data: create_jupiter_route_ix_data(swap_lamports, tampered_quoted_out, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_lower_out".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary: SwapCandidateSummary {
            instructions: Vec::new(),
            program_ids: vec![
                solana_sdk::compute_budget::id().to_string(),
                solana_sdk::system_program::ID.to_string(),
                transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
                spl_token::id().to_string(),
            ],
            writable_accounts,
            signer_accounts: vec![signer.to_string()],
            compute_unit_limit: 200_000,
            priority_fee_micro_lamports: 50_000,
            platform_fee_bps: 20,
            platform_fee_atomic: fee_lamports,
            platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 0,
            total_sol_required_atomic: 10_000_000 + fee_lamports,
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        },
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::OutputTooLow { .. })),
        "Jupiter instruction minimum output below candidate minimum must be rejected: {:?}",
        res
    );
}

#[test]
fn test_adversarial_jupiter_instruction_wrong_destination_ata_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let wrong_dest_ata = Pubkey::new_unique(); // Diverted destination!

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(wrong_dest_ata, false), // Wrong destination!
        ],
        data: create_jupiter_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_wrong_dest".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary: SwapCandidateSummary {
            instructions: Vec::new(),
            program_ids: vec![
                solana_sdk::compute_budget::id().to_string(),
                solana_sdk::system_program::ID.to_string(),
                transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
                spl_token::id().to_string(),
            ],
            writable_accounts,
            signer_accounts: vec![signer.to_string()],
            compute_unit_limit: 200_000,
            priority_fee_micro_lamports: 50_000,
            platform_fee_bps: 20,
            platform_fee_atomic: fee_lamports,
            platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 0,
            total_sol_required_atomic: 10_000_000 + fee_lamports,
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        },
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::DangerousInstruction(_))),
        "Jupiter instruction wrong destination ATA must be rejected: {:?}",
        res
    );
}

fn create_jupiter_shared_accounts_route_ix_data(in_amount: u64, quoted_out_amount: u64, slippage_bps: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(48);
    data.extend_from_slice(&transaction_validation::validator::JUPITER_SHARED_ACCOUNTS_ROUTE_DISCRIMINATOR);
    // id: u8
    data.push(0u8);
    // Borsh Vec<RoutePlanStep>: length 1 (4 bytes u32) + 16 bytes step dummy data
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&[0u8; 16]);
    // Trailing parameters: in_amount (8), quoted_out_amount (8), slippage_bps (2), platform_fee_bps (1)
    data.extend_from_slice(&in_amount.to_le_bytes());
    data.extend_from_slice(&quoted_out_amount.to_le_bytes());
    data.extend_from_slice(&slippage_bps.to_le_bytes());
    data.push(0u8);
    data
}

#[test]
fn test_valid_jupiter_shared_accounts_route_candidate_passes_validation() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    let program_authority = pubkey!("CapuXNQoDviLvU1PxFiizLgPNQCxrsag1uMeyk6zLVps");
    let program_source_ata = Pubkey::new_unique();
    let program_dest_ata = Pubkey::new_unique();

    let alt_key = Pubkey::new_unique();
    let pool_account = Pubkey::new_unique();
    let alt_account = solana_sdk::address_lookup_table::AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![pool_account],
    };

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),           // 0: token_program
            AccountMeta::new_readonly(program_authority, false),          // 1: program_authority
            AccountMeta::new(signer, true),                               // 2: user_transfer_authority
            AccountMeta::new(wsol_ata, false),                            // 3: user_source
            AccountMeta::new(program_source_ata, false),                  // 4: program_source_ata
            AccountMeta::new(program_dest_ata, false),                    // 5: program_dest_ata
            AccountMeta::new(bonk_ata, false),                            // 6: user_dest
            AccountMeta::new_readonly(wsol_mint, false),                  // 7: source_mint
            AccountMeta::new_readonly(bonk_mint, false),                  // 8: destination_mint
            AccountMeta::new_readonly(pool_account, false),               // 9: pool account from ALT
        ],
        data: create_jupiter_shared_accounts_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });
    instructions.push(spl_token::instruction::close_account(
        &spl_token::id(),
        &wsol_ata,
        &signer,
        &signer,
        &[],
    ).unwrap());

    let msg = Message::try_compile(&signer, &instructions, &[alt_account.clone()], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts_with_alts(&msg, &[alt_account.clone()]);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            transaction_validation::allowlist::COMPUTE_BUDGET_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
            transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
            spl_token::id().to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: 20,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 10_000_000 + fee_lamports,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 100,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_shared_accounts_test".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: vec![
            shared_types::swap::AddressLookupTableEntry {
                key: alt_account.key.to_string(),
                addresses: alt_account.addresses.iter().map(|p| p.to_string()).collect(),
            }
        ],
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(res.is_ok(), "Jupiter candidate with sharedAccountsRoute must pass validation: {:?}", res);
    let preview = res.unwrap();
    assert!(preview.platform_fee_recipient_verified);
    assert_eq!(preview.venue, "Jupiter (Raydium CLMM)");
}

#[test]
fn test_adversarial_jupiter_shared_accounts_route_tampered_authority_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    let program_authority = pubkey!("CapuXNQoDviLvU1PxFiizLgPNQCxrsag1uMeyk6zLVps");
    let attacker_authority = Pubkey::new_unique();
    let program_source_ata = Pubkey::new_unique();
    let program_dest_ata = Pubkey::new_unique();

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),           // 0: token_program
            AccountMeta::new_readonly(program_authority, false),          // 1: program_authority
            AccountMeta::new(attacker_authority, false),                  // 2: TAMPERED user_transfer_authority!
            AccountMeta::new(wsol_ata, false),                            // 3: user_source
            AccountMeta::new(program_source_ata, false),                  // 4: program_source_ata
            AccountMeta::new(program_dest_ata, false),                    // 5: program_dest_ata
            AccountMeta::new(bonk_ata, false),                            // 6: user_dest
        ],
        data: create_jupiter_shared_accounts_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_tampered_authority".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary: SwapCandidateSummary {
            instructions: Vec::new(),
            program_ids: vec![
                solana_sdk::compute_budget::id().to_string(),
                solana_sdk::system_program::ID.to_string(),
                transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
                spl_token::id().to_string(),
            ],
            writable_accounts,
            signer_accounts: vec![signer.to_string()],
            compute_unit_limit: 200_000,
            priority_fee_micro_lamports: 50_000,
            platform_fee_bps: 20,
            platform_fee_atomic: fee_lamports,
            platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 0,
            total_sol_required_atomic: 10_000_000 + fee_lamports,
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        },
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::SignerMismatch { .. })),
        "Jupiter sharedAccountsRoute tampered authority must be rejected: {:?}",
        res
    );
}

#[test]
fn test_adversarial_jupiter_shared_accounts_route_tampered_dest_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let tampered_dest_ata = Pubkey::new_unique();

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    let program_authority = pubkey!("CapuXNQoDviLvU1PxFiizLgPNQCxrsag1uMeyk6zLVps");
    let program_source_ata = Pubkey::new_unique();
    let program_dest_ata = Pubkey::new_unique();

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),           // 0: token_program
            AccountMeta::new_readonly(program_authority, false),          // 1: program_authority
            AccountMeta::new(signer, true),                               // 2: user_transfer_authority
            AccountMeta::new(wsol_ata, false),                            // 3: user_source
            AccountMeta::new(program_source_ata, false),                  // 4: program_source_ata
            AccountMeta::new(program_dest_ata, false),                    // 5: program_dest_ata
            AccountMeta::new(tampered_dest_ata, false),                   // 6: TAMPERED user_dest!
        ],
        data: create_jupiter_shared_accounts_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts(&msg);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_tampered_dest".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary: SwapCandidateSummary {
            instructions: Vec::new(),
            program_ids: vec![
                solana_sdk::compute_budget::id().to_string(),
                solana_sdk::system_program::ID.to_string(),
                transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
                spl_token::id().to_string(),
            ],
            writable_accounts,
            signer_accounts: vec![signer.to_string()],
            compute_unit_limit: 200_000,
            priority_fee_micro_lamports: 50_000,
            platform_fee_bps: 20,
            platform_fee_atomic: fee_lamports,
            platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 0,
            total_sol_required_atomic: 10_000_000 + fee_lamports,
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        },
        address_lookup_tables: Vec::new(),
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    assert!(
        matches!(res, Err(ValidationError::DangerousInstruction(_))),
        "Jupiter sharedAccountsRoute tampered dest ATA must be rejected: {:?}",
        res
    );
}

#[test]
fn test_adversarial_duplicate_jupiter_trade_instruction_rejected() {
    let signer = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
    let bonk_mint = pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263");
    let wsol_mint = pubkey!("So11111111111111111111111111111111111111112");
    let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &wsol_mint,
        &spl_token::id(),
    );
    let bonk_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &signer,
        &bonk_mint,
        &spl_token::id(),
    );

    let fee_lamports = 20_000u64;
    let swap_lamports = 9_980_000u64;

    let alt_key = Pubkey::new_unique();
    let pool_state_account = Pubkey::new_unique();
    let amm_authority = Pubkey::new_unique();
    let alt_account = solana_sdk::address_lookup_table::AddressLookupTableAccount {
        key: alt_key,
        addresses: vec![pool_state_account, amm_authority],
    };

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(system_instruction::transfer(&signer, &CONFIGURED_PLATFORM_TREASURY, fee_lamports));
    instructions.push(system_instruction::transfer(&signer, &wsol_ata, swap_lamports));
    // First Jupiter swap instruction
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
            AccountMeta::new(pool_state_account, false),
            AccountMeta::new_readonly(amm_authority, false),
        ],
        data: create_jupiter_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });
    // Adversarial DUPLICATE Jupiter swap instruction
    instructions.push(Instruction {
        program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(signer, true),
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(bonk_ata, false),
            AccountMeta::new(pool_state_account, false),
            AccountMeta::new_readonly(amm_authority, false),
        ],
        data: create_jupiter_route_ix_data(swap_lamports, 32_000_000_000, 100),
    });
    instructions.push(spl_token::instruction::close_account(
        &spl_token::id(),
        &wsol_ata,
        &signer,
        &signer,
        &[],
    ).unwrap());

    let msg = Message::try_compile(&signer, &instructions, &[alt_account.clone()], Hash::default()).unwrap();
    let writable_accounts = extract_msg_writable_accounts_with_alts(&msg, &[alt_account.clone()]);

    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let summary = SwapCandidateSummary {
        instructions: Vec::new(),
        program_ids: vec![
            transaction_validation::allowlist::COMPUTE_BUDGET_PROGRAM.to_string(),
            solana_sdk::system_program::ID.to_string(),
            transaction_validation::allowlist::JUPITER_V6_PROGRAM.to_string(),
            spl_token::id().to_string(),
        ],
        writable_accounts,
        signer_accounts: vec![signer.to_string()],
        compute_unit_limit: 200_000,
        priority_fee_micro_lamports: 50_000,
        platform_fee_bps: 20,
        platform_fee_atomic: fee_lamports,
        platform_fee_recipient: CONFIGURED_PLATFORM_TREASURY.to_string(),
        venue_fee_atomic: None,
        estimated_rent_lamports: 0,
        total_sol_required_atomic: 10_000_000,
        tx_byte_size: bytes.len(),
        quote_freshness_ms: 100,
    };

    let candidate = PreparedSwapCandidate {
        candidate_id: "candidate_jupiter_duplicate_trade_test".to_string(),
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        input_mint: wsol_mint.to_string(),
        output_mint: bonk_mint.to_string(),
        input_amount_atomic: 10_000_000,
        estimated_output_atomic: 32_000_000_000,
        minimum_output_atomic: 31_500_000_000,
        slippage_bps: 100,
        recent_blockhash: Hash::default().to_string(),
        last_valid_block_height: 310_000_150,
        state_slot: 310_000_000,
        created_at: "2026-10-28T22:00:00Z".to_string(),
        expires_at: "2026-10-28T22:02:00Z".to_string(),
        unsigned_tx_base64: b64,
        summary,
        address_lookup_tables: vec![
            shared_types::swap::AddressLookupTableEntry {
                key: alt_account.key.to_string(),
                addresses: alt_account.addresses.iter().map(|p| p.to_string()).collect(),
            }
        ],
    };

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    let res = validate_swap_candidate(&candidate, &signer, &fee_config);
    match res {
        Err(ValidationError::DangerousInstruction(msg)) => {
            assert!(msg.contains("Duplicate trade instruction detected"), "Expected duplicate trade instruction error, got: {}", msg);
        }
        other => panic!("Expected DangerousInstruction for duplicate trade, got: {:?}", other),
    }
}






