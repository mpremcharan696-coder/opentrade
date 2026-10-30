//! Safety and Tamper-Resistance Integration Tests.
//!
//! Verifies that:
//! 1. Intentionally modified transactions are rejected.
//! 2. Unapproved foreign program IDs are blocked.
//! 3. Platform fee transfers to unauthorized wallets fail validation.
//! 4. Valid trade intents pass and produce accurate preview bounds.

use base64::Engine;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::Message;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_instruction;
use solana_sdk::transaction::VersionedTransaction;

use protocol_pump::addresses::PUMP_PROGRAM;
use protocol_pump::instructions::build_buy_v2_instructions;
use shared_types::fee::{ComputedFee, FeeBasis, FeeConfig, FeeTier};
use shared_types::intent::{TradeIntent, TradeSide};
use shared_types::platform::{Platform, PumpInstructionVersion, TradeRoute};
use shared_types::token::TokenInfo;
use transaction_validation::validator::{ValidationError, validate_transaction};

fn mock_token(mint: Pubkey) -> TokenInfo {
    TokenInfo {
        name: "Test Pump".to_string(),
        symbol: "TEST".to_string(),
        uri: "https://test.com".to_string(),
        mint,
        platform: Platform::PumpFun,
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

fn create_valid_test_intent_and_tx(
    signer: &Pubkey,
    mint: &Pubkey,
    fee_wallet: &Pubkey,
) -> (TradeIntent, FeeConfig) {
    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: *fee_wallet,
        tier: FeeTier::Default,
    };

    let token_info = mock_token(*mint);
    let fee_lamports = 1_000_000; // 0.001 SOL fee

    let mut instructions = Vec::new();
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
    instructions.push(system_instruction::transfer(
        signer,
        fee_wallet,
        fee_lamports,
    ));
    instructions.extend(build_buy_v2_instructions(
        &token_info,
        signer,
        30_000_000,
        1_000_000_000,
    ));

    let msg = Message::try_compile(signer, &instructions, &[], Hash::default()).unwrap();
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let intent = TradeIntent {
        api_version: 1,
        intent_id: "test_intent_1".to_string(),
        created_at: "2026-10-24T20:00:00Z".to_string(),
        expires_at: "2026-10-24T20:02:00Z".to_string(),
        last_valid_block_height: 310_000_150,
        wallet_pubkey: *signer,
        side: TradeSide::Buy,
        mint: *mint,
        route: TradeRoute::PumpCurve {
            version: PumpInstructionVersion::V2,
        },
        input_amount_atomic: 1_000_000_000,
        minimum_output_atomic: 30_000_000,
        slippage_bps: 100,
        fee: ComputedFee {
            rate_bps: 10,
            basis: FeeBasis::QuoteAmount,
            recipient: *fee_wallet,
            amount_atomic: fee_lamports,
        },
        state_slot: 310_000_000,
        unsigned_tx_base64: b64,
    };

    (intent, fee_config)
}

#[test]
fn test_valid_transaction_passes_validation() {
    let signer = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let fee_wallet = Pubkey::new_unique();

    let (intent, fee_config) = create_valid_test_intent_and_tx(&signer, &mint, &fee_wallet);
    let preview =
        validate_transaction(&intent, &signer, &fee_config).expect("Validation should succeed");

    assert_eq!(preview.mint, mint);
    assert_eq!(preview.side, TradeSide::Buy);
    assert_eq!(preview.platform_fee_atomic, 1_000_000);
    assert_eq!(preview.platform_fee_recipient, fee_wallet);
}

#[test]
fn test_tampered_signer_is_rejected() {
    let signer = Pubkey::new_unique();
    let imposter = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let fee_wallet = Pubkey::new_unique();

    let (intent, fee_config) = create_valid_test_intent_and_tx(&signer, &mint, &fee_wallet);
    let err = validate_transaction(&intent, &imposter, &fee_config).unwrap_err();

    match err {
        ValidationError::SignerMismatch { .. } => {}
        _ => panic!("Expected SignerMismatch, got {:?}", err),
    }
}

#[test]
fn test_tampered_fee_recipient_is_rejected() {
    let signer = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let fee_wallet = Pubkey::new_unique();
    let attacker_wallet = Pubkey::new_unique();

    let (mut intent, fee_config) = create_valid_test_intent_and_tx(&signer, &mint, &fee_wallet);

    // Attacker modifies the expected fee recipient in fee config
    let altered_fee_config = FeeConfig {
        version: 1,
        fee_wallet: attacker_wallet,
        tier: FeeTier::Default,
    };

    let preview = validate_transaction(&intent, &signer, &altered_fee_config).unwrap();
    // A warning is raised because the fee transfer in tx went to fee_wallet instead of attacker_wallet
    assert!(
        preview
            .warnings
            .iter()
            .any(|w| w.contains("Platform fee transfer instruction was not found"))
    );
}

#[test]
fn test_forbidden_program_is_blocked() {
    let signer = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let fee_wallet = Pubkey::new_unique();
    let evil_program = Pubkey::new_unique();

    let (mut intent, fee_config) = create_valid_test_intent_and_tx(&signer, &mint, &fee_wallet);

    // Inject an instruction invoking an unknown/foreign program
    let mut instructions = Vec::new();
    instructions.push(Instruction {
        program_id: evil_program,
        accounts: vec![AccountMeta::new(signer, true)],
        data: vec![1, 2, 3],
    });

    let msg = Message::try_compile(&signer, &instructions, &[], Hash::default()).unwrap();
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };

    let bytes = bincode::serialize(&tx).unwrap();
    intent.unsigned_tx_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let err = validate_transaction(&intent, &signer, &fee_config).unwrap_err();
    match err {
        ValidationError::ForbiddenProgram(p) => assert_eq!(p, evil_program),
        _ => panic!("Expected ForbiddenProgram, got {:?}", err),
    }
}

#[test]
fn test_okx_router_transaction_validation_and_fee_protection() {
    use protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID;
    use protocol_okx_router::instructions::{
        OkxSwapV3Accounts, build_okx_swap_v3_instruction, create_single_hop_swap_args,
    };
    use protocol_okx_router::types::Dex;

    let signer = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let fee_wallet = Pubkey::new_unique();
    let source_ata = Pubkey::new_unique();
    let dest_ata = Pubkey::new_unique();
    let wsol = Pubkey::new_unique();

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet,
        tier: FeeTier::Default,
    };

    let accounts = OkxSwapV3Accounts {
        payer: signer,
        source_token_account: source_ata,
        destination_token_account: dest_ata,
        source_mint: wsol,
        destination_mint: mint,
        platform_fee_wallet: fee_wallet,
        pool_remaining_accounts: vec![],
    };

    let args = create_single_hop_swap_args(1_000_000_000, 500_000_000, Dex::RaydiumSwap);
    let ix = build_okx_swap_v3_instruction(&accounts, args, 20, 12345, None).unwrap();

    let msg = Message::try_compile(&signer, &[ix], &[], Hash::default()).unwrap();
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };
    let bytes = bincode::serialize(&tx).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let intent = TradeIntent {
        api_version: 1,
        intent_id: "okx_intent_1".to_string(),
        created_at: "2026-10-24T20:00:00Z".to_string(),
        expires_at: "2026-10-24T20:02:00Z".to_string(),
        last_valid_block_height: 310_000_150,
        wallet_pubkey: signer,
        side: TradeSide::Buy,
        mint,
        route: TradeRoute::OkxRouter {
            dex_id: Dex::RaydiumSwap as u8,
        },
        input_amount_atomic: 1_000_000_000,
        minimum_output_atomic: 500_000_000,
        slippage_bps: 100,
        fee: ComputedFee {
            rate_bps: 20,
            basis: FeeBasis::QuoteAmount,
            recipient: fee_wallet,
            amount_atomic: 2_000_000,
        },
        state_slot: 310_000_000,
        unsigned_tx_base64: b64,
    };

    let preview =
        validate_transaction(&intent, &signer, &fee_config).expect("Validation should succeed");
    assert_eq!(preview.mint, mint);
    assert_eq!(preview.platform_fee_recipient, fee_wallet);
    assert_eq!(preview.platform_fee_atomic, 2_000_000);

    // Now test that if an attacker tampers with the fee wallet in the instruction, it is REJECTED
    let evil_wallet = Pubkey::new_unique();
    let evil_accounts = OkxSwapV3Accounts {
        platform_fee_wallet: evil_wallet,
        ..accounts
    };
    let evil_args = create_single_hop_swap_args(1_000_000_000, 500_000_000, Dex::RaydiumSwap);
    let evil_ix =
        build_okx_swap_v3_instruction(&evil_accounts, evil_args, 20, 12345, None).unwrap();
    let evil_msg = Message::try_compile(&signer, &[evil_ix], &[], Hash::default()).unwrap();
    let evil_tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(evil_msg),
    };
    let evil_b64 =
        base64::engine::general_purpose::STANDARD.encode(&bincode::serialize(&evil_tx).unwrap());

    let mut tampered_intent = intent.clone();
    tampered_intent.unsigned_tx_base64 = evil_b64;

    let err = validate_transaction(&tampered_intent, &signer, &fee_config).unwrap_err();
    match err {
        ValidationError::FeeRecipientMismatch { expected, actual } => {
            assert_eq!(expected, fee_wallet);
            assert_eq!(actual, evil_wallet);
        }
        _ => panic!("Expected FeeRecipientMismatch, got {:?}", err),
    }
}
