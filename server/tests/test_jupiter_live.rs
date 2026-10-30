//! Live mainnet simulation test for Jupiter swap candidate preparation, native validation, and RPC simulation.

use base64::Engine;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use trading_server::state::AppState;
use trading_server::routes::swap::prepare_swap_candidate_handler;
use shared_types::intent::TradeSide;
use shared_types::swap::PrepareSwapRequest;
use shared_types::fee::{FeeConfig, FeeTier, CONFIGURED_PLATFORM_TREASURY};
use transaction_validation::validator::{validate_swap_candidate, validate_and_simulate_swap_candidate};

const TEST_WALLET: Pubkey = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
const HOLDER_WALLET: Pubkey = pubkey!("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM");
const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const HELIUS_RPC: &str = "https://mainnet.helius-rpc.com/?api-key=eda6e938-7c34-4332-bf0e-3722786de677";

#[tokio::test]
async fn test_jupiter_live_prepare_and_validation() {
    let state = AppState::new(CONFIGURED_PLATFORM_TREASURY, HELIUS_RPC.to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let req = PrepareSwapRequest {
        wallet_pubkey: TEST_WALLET.to_string(),
        side: TradeSide::Buy,
        input_mint: WSOL_MINT.to_string(),
        output_mint: BONK_MINT.to_string(),
        input_amount_atomic: 10_000_000, // 0.01 SOL -> fee = 20,000 lamports
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        slippage_bps: 100, // 1%
        expected_minimum_output_atomic: 1_000_000, // Safe bound
        quoted_at_unix_ms: now,
        state_slot: None,
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
    };

    use axum::response::IntoResponse;

    let res = prepare_swap_candidate_handler(
        axum::extract::State(state),
        axum::Json(req),
    ).await;

    assert!(res.is_ok(), "Failed to prepare Jupiter candidate: {:?}", res.err());
    let response = res.unwrap().into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let candidate: shared_types::swap::PreparedSwapCandidate = serde_json::from_slice(&bytes)
        .expect("Failed to deserialize PreparedSwapCandidate");

    assert_eq!(candidate.selected_route_source, "jupiter_v2_order");
    assert!(candidate.venue.contains("Raydium"), "Jupiter candidate venue must prove Raydium routing: {}", candidate.venue);
    assert_eq!(candidate.summary.platform_fee_atomic, 20_000); // 0.20% on 10,000,000
    assert_eq!(candidate.summary.platform_fee_recipient, CONFIGURED_PLATFORM_TREASURY.to_string());

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    // Native client-side validation
    let validation_summary = validate_swap_candidate(&candidate, &TEST_WALLET, &fee_config);
    assert!(validation_summary.is_ok(), "Native validation failed: {:?}", validation_summary.err());
    let preview = validation_summary.unwrap();
    assert!(preview.platform_fee_recipient_verified);
    assert!(preview.venue.contains("Raydium"), "Preview venue must prove Raydium routing: {}", preview.venue);

    let tx_bytes = base64::engine::general_purpose::STANDARD.decode(&candidate.unsigned_tx_base64).unwrap();
    let tx: solana_sdk::transaction::VersionedTransaction = bincode::deserialize(&tx_bytes).unwrap();
    for (i, k) in tx.message.static_account_keys().iter().enumerate() {
        println!("Static Account [{}]: {}", i, k);
    }
    for (i, ix) in tx.message.instructions().iter().enumerate() {
        let prog_id = tx.message.static_account_keys()[ix.program_id_index as usize];
        println!("Instruction #{}: Program {}", i, prog_id);
        println!("  data len: {}, first 8 bytes: {:02x?}", ix.data.len(), &ix.data[..std::cmp::min(8, ix.data.len())]);
        for (a_idx, &acc_idx) in ix.accounts.iter().enumerate() {
            println!("    Account meta [{}]: index {}", a_idx, acc_idx);
        }
    }
    // Full validation & read-only mainnet RPC simulation
    let sim_result = validate_and_simulate_swap_candidate(&candidate, &TEST_WALLET, &fee_config, HELIUS_RPC).await;
    println!("Simulation is_valid: {}, is_simulated: {}", sim_result.is_valid, sim_result.is_simulated);
    println!("Simulation units: {:?}", sim_result.simulation_units_consumed);
    if !sim_result.validation_errors.is_empty() {
        println!("Validation/Simulation errors: {:?}", sim_result.validation_errors);
    }
    assert!(sim_result.is_valid, "Candidate must be valid and pass on-chain simulation: {:?}", sim_result.validation_errors);
}

#[tokio::test]
async fn test_jupiter_live_sell_prepare_and_native_validation() {
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let state = AppState::new(CONFIGURED_PLATFORM_TREASURY, HELIUS_RPC.to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let req = PrepareSwapRequest {
        wallet_pubkey: HOLDER_WALLET.to_string(),
        side: TradeSide::Sell,
        input_mint: BONK_MINT.to_string(),
        output_mint: WSOL_MINT.to_string(),
        input_amount_atomic: 100_000_000, // 100 BONK
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        slippage_bps: 100, // 1%
        expected_minimum_output_atomic: 1_000, // Safe bound
        quoted_at_unix_ms: now,
        state_slot: None,
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
    };

    use axum::response::IntoResponse;

    let res = prepare_swap_candidate_handler(
        axum::extract::State(state),
        axum::Json(req),
    ).await;

    assert!(res.is_ok(), "Failed to prepare Jupiter sell candidate: {:?}", res.err());
    let response = res.unwrap().into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let candidate: shared_types::swap::PreparedSwapCandidate = serde_json::from_slice(&bytes)
        .expect("Failed to deserialize PreparedSwapCandidate");

    assert_eq!(candidate.selected_route_source, "jupiter_v2_order");
    assert!(candidate.venue.contains("Raydium"), "Jupiter sell candidate venue must prove Raydium routing: {}", candidate.venue);

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: FeeTier::Default,
    };

    // Native client-side validation for Jupiter sell
    let validation_summary = validate_swap_candidate(&candidate, &HOLDER_WALLET, &fee_config);
    assert!(validation_summary.is_ok(), "Native validation of Jupiter sell failed: {:?}", validation_summary.err());
    let preview = validation_summary.unwrap();
    assert!(preview.platform_fee_recipient_verified);
    assert_eq!(preview.side, "Sell");
    assert!(preview.venue.contains("Raydium"), "Preview venue must prove Raydium routing: {}", preview.venue);

    let sim_result = validate_and_simulate_swap_candidate(&candidate, &HOLDER_WALLET, &fee_config, HELIUS_RPC).await;
    println!("Sell Simulation is_valid: {}, is_simulated: {}", sim_result.is_valid, sim_result.is_simulated);
    println!("Sell Simulation errors: {:?}", sim_result.validation_errors);
    println!("Sell Simulation logs: {:?}", sim_result.simulation_logs);

    // Assert that the on-chain simulation actually succeeded
    assert!(sim_result.is_valid, "Sell candidate must pass native validation and on-chain simulation: {:?}", sim_result.validation_errors);
    assert!(sim_result.is_simulated, "On-chain simulation must execute");
    assert!(sim_result.simulation_units_consumed.is_some(), "Simulation must consume compute units");
    assert!(
        sim_result.simulation_logs.iter().any(|l| l.contains("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")),
        "Simulation logs must confirm invocation of Jupiter V6 on-chain: {:?}",
        sim_result.simulation_logs
    );
}
