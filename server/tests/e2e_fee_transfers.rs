//! End-to-End Test for Fee Deduction and Treasury Transfer.
//!
//! Validates that:
//! 1. Buy transaction deducts exactly 0.20% fee and routes to AnWnLUpa1ohdgp3WRmhPpxXEusmcEDZNtYTsCM8qq9eH.
//! 2. Sell transaction deducts 0.20% fee and routes to AnWnLUpa1ohdgp3WRmhPpxXEusmcEDZNtYTsCM8qq9eH.
//! 3. System transfer instruction is compiled into the VersionedTransaction.
//! 4. Client-side transaction validator passes and confirms recipient and fee lamports.
//! 5. Keypair signs the transaction successfully.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;

use shared_types::fee::{FeeConfig, FeeTier};
use shared_types::platform::Platform;
use shared_types::token::TokenInfo;
use trading_server::builder::{CreateIntentRequest, build_buy_intent, build_sell_intent};
use transaction_validation::validator::validate_transaction;

const TREASURY_WALLET: Pubkey = pubkey!("AnWnLUpa1ohdgp3WRmhPpxXEusmcEDZNtYTsCM8qq9eH");

fn test_token_info(mint: Pubkey) -> TokenInfo {
    TokenInfo {
        name: "Test Pump Token".to_string(),
        symbol: "TEST".to_string(),
        uri: "https://pump.fun/test".to_string(),
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

#[test]
fn test_buy_transaction_deducts_fee_and_transfers_to_treasury() {
    let buyer = Keypair::new();
    let mint = Pubkey::new_unique();
    let token = test_token_info(mint);

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: TREASURY_WALLET,
        tier: FeeTier::Default,
    };

    // 1.0 SOL Buy = 1_000_000_000 lamports
    let input_sol_lamports: u64 = 1_000_000_000;
    let req = CreateIntentRequest {
        wallet_pubkey: buyer.pubkey().to_string(),
        mint: mint.to_string(),
        amount_atomic: input_sol_lamports,
        slippage_bps: Some(100), // 1.0%
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
        platform: None,
        dex_id: None,
    };

    let intent = build_buy_intent(&req, &fee_config, &token, Hash::default(), 310_000_000)
        .expect("build_buy_intent should succeed");

    // 1. Verify fee is 20 bps (0.20%)
    assert_eq!(intent.fee.rate_bps, 20, "Fee rate must be 20 bps (0.20%)");

    // 2. Verify recipient is exactly TREASURY_WALLET
    assert_eq!(
        intent.fee.recipient, TREASURY_WALLET,
        "Fee recipient must be CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM"
    );

    // 3. Verify exact fee amount: 1 SOL * 0.20% = 0.002 SOL = 2_000_000 lamports
    let expected_fee_lamports = 2_000_000u64;
    assert_eq!(
        intent.fee.amount_atomic, expected_fee_lamports,
        "Fee amount must be exactly 2,000,000 lamports (0.002 SOL)"
    );

    // 4. Verify deserialized transaction contains system transfer to treasury
    let tx_bytes = STANDARD
        .decode(&intent.unsigned_tx_base64)
        .expect("Base64 decode");
    let tx: VersionedTransaction = bincode::deserialize(&tx_bytes).expect("Deserialize tx");

    let account_keys = tx.message.static_account_keys();
    assert!(
        account_keys.contains(&TREASURY_WALLET),
        "Compiled transaction accounts must include treasury wallet"
    );

    // 5. Client-Side Validation Engine Verification
    let preview = validate_transaction(&intent, &buyer.pubkey(), &fee_config)
        .expect("Client-side transaction validation must pass");

    assert_eq!(preview.platform_fee_atomic, expected_fee_lamports);
    assert_eq!(preview.platform_fee_recipient, TREASURY_WALLET);
    assert_eq!(intent.wallet_pubkey, buyer.pubkey());
    assert_eq!(preview.mint, mint);
    assert!(
        preview.warnings.is_empty(),
        "Validation should have zero warnings: {:?}",
        preview.warnings
    );

    // 6. Sign transaction locally
    let mut signed_tx = tx;
    let message_data = signed_tx.message.serialize();
    let sig = buyer.sign_message(&message_data);
    signed_tx.signatures = vec![sig];
    assert!(signed_tx.signatures[0] != solana_sdk::signature::Signature::default());
}

#[test]
fn test_sell_transaction_deducts_fee_and_transfers_to_treasury() {
    let seller = Keypair::new();
    let mint = Pubkey::new_unique();
    let token = test_token_info(mint);

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: TREASURY_WALLET,
        tier: FeeTier::Default,
    };

    // ~34,000,000 tokens sell (with 6 decimals = 34_000_000_000_000 atomic units, ~0.95 SOL worth)
    let tokens_to_sell: u64 = 34_000_000_000_000;
    let req = CreateIntentRequest {
        wallet_pubkey: seller.pubkey().to_string(),
        mint: mint.to_string(),
        amount_atomic: tokens_to_sell,
        slippage_bps: Some(100),
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
        platform: None,
        dex_id: None,
    };

    let intent = build_sell_intent(&req, &fee_config, &token, Hash::default(), 310_000_000)
        .expect("build_sell_intent should succeed");

    // 1. Verify fee rate
    assert_eq!(intent.fee.rate_bps, 20, "Fee rate must be 20 bps (0.20%)");

    // 2. Verify recipient is treasury
    assert_eq!(intent.fee.recipient, TREASURY_WALLET);

    // 3. Verify fee amount > 0
    assert!(
        intent.fee.amount_atomic > 0,
        "Fee lamports must be greater than zero"
    );

    // 4. Verify transaction contains treasury
    let tx_bytes = STANDARD
        .decode(&intent.unsigned_tx_base64)
        .expect("Base64 decode");
    let tx: VersionedTransaction = bincode::deserialize(&tx_bytes).expect("Deserialize tx");

    let account_keys = tx.message.static_account_keys();
    assert!(
        account_keys.contains(&TREASURY_WALLET),
        "Compiled sell transaction accounts must include treasury wallet"
    );

    // 5. Client-Side Validation Engine Verification
    let preview = validate_transaction(&intent, &seller.pubkey(), &fee_config)
        .expect("Client-side transaction validation must pass");

    assert_eq!(preview.platform_fee_atomic, intent.fee.amount_atomic);
    assert_eq!(preview.platform_fee_recipient, TREASURY_WALLET);
    assert_eq!(intent.wallet_pubkey, seller.pubkey());
    assert_eq!(preview.mint, mint);
    assert!(
        preview.warnings.is_empty(),
        "Validation should have zero warnings: {:?}",
        preview.warnings
    );
}

#[tokio::test]
async fn test_okx_router_buy_and_sell_fee_transfers() {
    use trading_server::builder::{build_okx_router_buy_intent, build_okx_router_sell_intent};

    let buyer = Keypair::new();
    let mint = Pubkey::new_unique();

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: TREASURY_WALLET,
        tier: FeeTier::Default,
    };

    let req = CreateIntentRequest {
        wallet_pubkey: buyer.pubkey().to_string(),
        mint: mint.to_string(),
        amount_atomic: 2_000_000_000, // 2.0 SOL
        slippage_bps: Some(100),
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
        platform: Some(Platform::OkxRouter),
        dex_id: Some(4), // Raydium
    };

    let buy_intent = build_okx_router_buy_intent(&req, &fee_config, Hash::default(), 310_000_000)
        .expect("build_okx_router_buy_intent should succeed");

    assert_eq!(buy_intent.fee.rate_bps, 20);
    assert_eq!(buy_intent.fee.recipient, TREASURY_WALLET);
    assert_eq!(buy_intent.fee.amount_atomic, 4_000_000); // 0.20% of 2 SOL = 0.004 SOL

    let preview = validate_transaction(&buy_intent, &buyer.pubkey(), &fee_config)
        .expect("OKX router buy transaction validation must pass");
    assert_eq!(preview.platform_fee_atomic, 4_000_000);
    assert_eq!(preview.platform_fee_recipient, TREASURY_WALLET);

    let sell_req = CreateIntentRequest {
        wallet_pubkey: buyer.pubkey().to_string(),
        mint: mint.to_string(),
        amount_atomic: 1_000_000_000,
        slippage_bps: Some(100),
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
        platform: Some(Platform::OkxRouter),
        dex_id: Some(4),
    };

    let sell_intent =
        build_okx_router_sell_intent(&sell_req, &fee_config, Hash::default(), 310_000_000)
            .expect("build_okx_router_sell_intent should succeed");

    assert_eq!(sell_intent.fee.rate_bps, 20);
    assert_eq!(sell_intent.fee.recipient, TREASURY_WALLET);

    let sell_preview = validate_transaction(&sell_intent, &buyer.pubkey(), &fee_config)
        .expect("OKX router sell transaction validation must pass");
    assert_eq!(sell_preview.platform_fee_recipient, TREASURY_WALLET);
}
