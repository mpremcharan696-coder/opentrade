//! Server-side transaction assembly and TradeIntent construction.

use base64::Engine;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_instruction;
use solana_sdk::transaction::VersionedTransaction;
use thiserror::Error;

use shared_types::fee::{ComputedFee, FeeBasis, FeeConfig, FeeTier, compute_fee};
use shared_types::intent::{TradeIntent, TradeSide};
use shared_types::platform::{Platform, PumpInstructionVersion, TradeRoute};
use shared_types::token::TokenInfo;

use protocol_okx_router::addresses::WSOL_MINT;
use protocol_okx_router::instructions::{
    OkxSwapV3Accounts, build_okx_swap_v3_instruction, create_single_hop_swap_args,
};
use protocol_okx_router::types::Dex;
use protocol_pump::curve::{
    apply_slippage_min, calculate_buy_tokens_out, calculate_sell_quote_out,
};
use protocol_pump::instructions::{build_buy_v2_instructions, build_sell_v2_instructions};
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;

#[derive(Error, Debug)]
pub enum BuilderError {
    #[error("Fee calculation error: {0}")]
    Fee(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Curve calculation error: {0}")]
    Curve(String),
}

/// Request parameters sent by desktop to request an unsigned trade transaction.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CreateIntentRequest {
    pub wallet_pubkey: String,
    pub mint: String,
    pub amount_atomic: u64,
    pub slippage_bps: Option<u16>,
    pub fee_tier: Option<FeeTier>,
    pub priority_fee_micro_lamports: Option<u64>,
    pub platform: Option<Platform>,
    pub dex_id: Option<u8>,
}

/// Generate current and expiry ISO-8601 UTC timestamps (Howard Hinnant algorithm).
pub fn current_iso8601_timestamps() -> (String, String) {
    let now = std::time::SystemTime::now();
    let format_time = |t: std::time::SystemTime| -> String {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let total_secs = dur.as_secs();
        let days = (total_secs / 86400) as i64;
        let rem_secs = (total_secs % 86400) as i64;
        let hours = rem_secs / 3600;
        let minutes = (rem_secs % 3600) / 60;
        let seconds = rem_secs % 60;

        let z = days + 719468;
        let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
        let doe = (z - era * 146097) as u32;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = (yoe as i64) + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            y, m, d, hours, minutes, seconds
        )
    };

    let created_at = format_time(now);
    let expires_at = format_time(now + std::time::Duration::from_secs(120));
    (created_at, expires_at)
}

/// Build an unsigned Buy transaction and wrap it in a versioned TradeIntent.
pub fn build_buy_intent(
    req: &CreateIntentRequest,
    fee_config: &FeeConfig,
    token_info: &TokenInfo,
    recent_blockhash: Hash,
    current_slot: u64,
) -> Result<TradeIntent, BuilderError> {
    let wallet = req
        .wallet_pubkey
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let mint = req
        .mint
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;

    let slippage_bps = req.slippage_bps.unwrap_or(100); // 1.0% default
    let fee_tier = req.fee_tier.unwrap_or(fee_config.tier);

    // 1. Calculate platform fee
    let fee_amount = compute_fee(req.amount_atomic, fee_tier.rate_bps())
        .map_err(|e| BuilderError::Fee(e.to_string()))?;

    // 2. Compute expected output tokens from verified curve reserves
    let v_token = token_info.virtual_token_reserves.ok_or_else(|| {
        BuilderError::Curve(
            "Missing virtual_token_reserves: real on-chain market state is required".to_string(),
        )
    })?;
    let v_quote = token_info.virtual_quote_reserves.ok_or_else(|| {
        BuilderError::Curve(
            "Missing virtual_quote_reserves: real on-chain market state is required".to_string(),
        )
    })?;
    let expected_tokens = calculate_buy_tokens_out(req.amount_atomic, v_token, v_quote)
        .map_err(|e| BuilderError::Curve(e.to_string()))?;
    let min_tokens_out = apply_slippage_min(expected_tokens, slippage_bps);

    // 3. Assemble instructions
    let mut instructions: Vec<Instruction> = Vec::new();

    // Compute budget: 200k units, priority fee from request (default 50k micro-lamports)
    let priority_fee = req.priority_fee_micro_lamports.unwrap_or(50_000);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee,
    ));

    // Platform fee transfer
    instructions.push(system_instruction::transfer(
        &wallet,
        &fee_config.fee_wallet,
        fee_amount,
    ));

    // Pump.fun buy_v2 instructions (includes base ATA idempotent creation)
    let buy_ixs = build_buy_v2_instructions(token_info, &wallet, min_tokens_out, req.amount_atomic);
    instructions.extend(buy_ixs);

    // 4. Build VersionedTransaction
    let message = Message::try_compile(&wallet, &instructions, &[], recent_blockhash)
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let versioned_msg = VersionedMessage::V0(message);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: versioned_msg,
    };

    let serialized_bytes =
        bincode::serialize(&tx).map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let unsigned_tx_base64 = base64::engine::general_purpose::STANDARD.encode(&serialized_bytes);

    let intent_id = format!("intent_{}_{}", mint, rand::random::<u32>());
    let (created_at, expires_at) = current_iso8601_timestamps();

    Ok(TradeIntent {
        api_version: 1,
        intent_id,
        created_at,
        expires_at,
        last_valid_block_height: current_slot + 150,
        wallet_pubkey: wallet,
        side: TradeSide::Buy,
        mint,
        route: TradeRoute::PumpCurve {
            version: PumpInstructionVersion::V2,
        },
        input_amount_atomic: req.amount_atomic,
        minimum_output_atomic: min_tokens_out,
        slippage_bps,
        fee: ComputedFee {
            rate_bps: fee_tier.rate_bps(),
            basis: FeeBasis::QuoteAmount,
            recipient: fee_config.fee_wallet,
            amount_atomic: fee_amount,
        },
        state_slot: current_slot,
        unsigned_tx_base64,
    })
}

/// Build an unsigned Sell transaction and wrap it in a versioned TradeIntent.
pub fn build_sell_intent(
    req: &CreateIntentRequest,
    fee_config: &FeeConfig,
    token_info: &TokenInfo,
    recent_blockhash: Hash,
    current_slot: u64,
) -> Result<TradeIntent, BuilderError> {
    let wallet = req
        .wallet_pubkey
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let mint = req
        .mint
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;

    let slippage_bps = req.slippage_bps.unwrap_or(100);
    let fee_tier = req.fee_tier.unwrap_or(fee_config.tier);

    // Compute expected quote output from verified curve reserves
    let v_token = token_info.virtual_token_reserves.ok_or_else(|| {
        BuilderError::Curve(
            "Missing virtual_token_reserves: real on-chain market state is required".to_string(),
        )
    })?;
    let v_quote = token_info.virtual_quote_reserves.ok_or_else(|| {
        BuilderError::Curve(
            "Missing virtual_quote_reserves: real on-chain market state is required".to_string(),
        )
    })?;
    let expected_quote = calculate_sell_quote_out(req.amount_atomic, v_token, v_quote)
        .map_err(|e| BuilderError::Curve(e.to_string()))?;
    let min_quote_out = apply_slippage_min(expected_quote, slippage_bps);

    let fee_amount = compute_fee(min_quote_out, fee_tier.rate_bps())
        .map_err(|e| BuilderError::Fee(e.to_string()))?;

    let mut instructions: Vec<Instruction> = Vec::new();
    // Compute budget: 200k units, priority fee from request (default 50k micro-lamports)
    let priority_fee = req.priority_fee_micro_lamports.unwrap_or(50_000);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee,
    ));

    // Platform fee transfer
    instructions.push(system_instruction::transfer(
        &wallet,
        &fee_config.fee_wallet,
        fee_amount,
    ));

    // Pump.fun sell_v2 instructions
    let sell_ixs =
        build_sell_v2_instructions(token_info, &wallet, req.amount_atomic, min_quote_out);
    instructions.extend(sell_ixs);

    let message = Message::try_compile(&wallet, &instructions, &[], recent_blockhash)
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let versioned_msg = VersionedMessage::V0(message);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: versioned_msg,
    };

    let serialized_bytes =
        bincode::serialize(&tx).map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let unsigned_tx_base64 = base64::engine::general_purpose::STANDARD.encode(&serialized_bytes);

    let intent_id = format!("intent_{}_{}", mint, rand::random::<u32>());
    let (created_at, expires_at) = current_iso8601_timestamps();

    Ok(TradeIntent {
        api_version: 1,
        intent_id,
        created_at,
        expires_at,
        last_valid_block_height: current_slot + 150,
        wallet_pubkey: wallet,
        side: TradeSide::Sell,
        mint,
        route: TradeRoute::PumpCurve {
            version: PumpInstructionVersion::V2,
        },
        input_amount_atomic: req.amount_atomic,
        minimum_output_atomic: min_quote_out,
        slippage_bps,
        fee: ComputedFee {
            rate_bps: fee_tier.rate_bps(),
            basis: FeeBasis::QuoteAmount,
            recipient: fee_config.fee_wallet,
            amount_atomic: fee_amount,
        },
        state_slot: current_slot,
        unsigned_tx_base64,
    })
}

/// Build an unsigned Buy transaction via OKX DEX Router V1 and wrap it in a versioned TradeIntent.
pub fn build_okx_router_buy_intent(
    req: &CreateIntentRequest,
    fee_config: &FeeConfig,
    recent_blockhash: Hash,
    current_slot: u64,
) -> Result<TradeIntent, BuilderError> {
    let wallet = req
        .wallet_pubkey
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let mint = req
        .mint
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;

    let slippage_bps = req.slippage_bps.unwrap_or(100);
    let fee_tier = req.fee_tier.unwrap_or(fee_config.tier);
    let fee_amount = compute_fee(req.amount_atomic, fee_tier.rate_bps())
        .map_err(|e| BuilderError::Fee(e.to_string()))?;

    // Determine target DEX (RaydiumSwap by default if unspecified)
    let dex = match req.dex_id {
        Some(4) => Dex::RaydiumSwap,
        Some(13) => Dex::MeteoraDlmm,
        Some(2) => Dex::Whirlpool,
        Some(23) => Dex::PumpfunBuy,
        Some(_) => Dex::RaydiumSwap,
        None => Dex::RaydiumSwap,
    };

    let user_wsol_ata = get_associated_token_address(&wallet, &WSOL_MINT);
    let user_token_ata = get_associated_token_address(&wallet, &mint);

    let accounts = OkxSwapV3Accounts {
        payer: wallet,
        source_token_account: user_wsol_ata,
        destination_token_account: user_token_ata,
        source_mint: WSOL_MINT,
        destination_mint: mint,
        platform_fee_wallet: fee_config.fee_wallet,
        pool_remaining_accounts: vec![],
    };

    let min_return = req.amount_atomic.saturating_mul(99) / 100;
    let args = create_single_hop_swap_args(req.amount_atomic, min_return, dex);
    let order_id = rand::random::<u64>();

    let mut instructions: Vec<Instruction> = Vec::new();
    let priority_fee = req.priority_fee_micro_lamports.unwrap_or(50_000);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(300_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee,
    ));

    // Ensure ATAs exist
    instructions.push(create_associated_token_account_idempotent(
        &wallet,
        &wallet,
        &WSOL_MINT,
        &spl_token::id(),
    ));
    instructions.push(create_associated_token_account_idempotent(
        &wallet,
        &wallet,
        &mint,
        &spl_token::id(),
    ));

    // Fund WSOL ATA and sync native
    instructions.push(solana_sdk::system_instruction::transfer(
        &wallet,
        &user_wsol_ata,
        req.amount_atomic,
    ));
    instructions.push(
        spl_token::instruction::sync_native(&spl_token::id(), &user_wsol_ata)
            .map_err(|e| BuilderError::Serialization(e.to_string()))?,
    );

    // OKX Router swap_v3 instruction with on-chain platform fee routing
    let swap_ix =
        build_okx_swap_v3_instruction(&accounts, args, fee_tier.rate_bps(), order_id, None)
            .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    instructions.push(swap_ix);

    // Unwind any remaining WSOL
    instructions.push(
        spl_token::instruction::close_account(
            &spl_token::id(),
            &user_wsol_ata,
            &wallet,
            &wallet,
            &[],
        )
        .map_err(|e| BuilderError::Serialization(e.to_string()))?,
    );

    let message = Message::try_compile(&wallet, &instructions, &[], recent_blockhash)
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let versioned_msg = VersionedMessage::V0(message);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: versioned_msg,
    };

    let serialized_bytes =
        bincode::serialize(&tx).map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let unsigned_tx_base64 = base64::engine::general_purpose::STANDARD.encode(&serialized_bytes);

    let intent_id = format!("intent_okx_{}_{}", mint, rand::random::<u32>());
    let (created_at, expires_at) = current_iso8601_timestamps();

    Ok(TradeIntent {
        api_version: 1,
        intent_id,
        created_at,
        expires_at,
        last_valid_block_height: current_slot + 150,
        wallet_pubkey: wallet,
        side: TradeSide::Buy,
        mint,
        route: TradeRoute::OkxRouter { dex_id: dex as u8 },
        input_amount_atomic: req.amount_atomic,
        minimum_output_atomic: min_return,
        slippage_bps,
        fee: ComputedFee {
            rate_bps: fee_tier.rate_bps(),
            basis: FeeBasis::QuoteAmount,
            recipient: fee_config.fee_wallet,
            amount_atomic: fee_amount,
        },
        state_slot: current_slot,
        unsigned_tx_base64,
    })
}

/// Build an unsigned Sell transaction via OKX DEX Router V1 and wrap it in a versioned TradeIntent.
pub fn build_okx_router_sell_intent(
    req: &CreateIntentRequest,
    fee_config: &FeeConfig,
    recent_blockhash: Hash,
    current_slot: u64,
) -> Result<TradeIntent, BuilderError> {
    let wallet = req
        .wallet_pubkey
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let mint = req
        .mint
        .parse::<Pubkey>()
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;

    let slippage_bps = req.slippage_bps.unwrap_or(100);
    let fee_tier = req.fee_tier.unwrap_or(fee_config.tier);

    let dex = match req.dex_id {
        Some(4) => Dex::RaydiumSwap,
        Some(13) => Dex::MeteoraDlmm,
        Some(2) => Dex::Whirlpool,
        Some(24) => Dex::PumpfunSell,
        Some(_) => Dex::RaydiumSwap,
        None => Dex::RaydiumSwap,
    };

    let user_wsol_ata = get_associated_token_address(&wallet, &WSOL_MINT);
    let user_token_ata = get_associated_token_address(&wallet, &mint);

    let accounts = OkxSwapV3Accounts {
        payer: wallet,
        source_token_account: user_token_ata,
        destination_token_account: user_wsol_ata,
        source_mint: mint,
        destination_mint: WSOL_MINT,
        platform_fee_wallet: fee_config.fee_wallet,
        pool_remaining_accounts: vec![],
    };

    let min_return = req.amount_atomic.saturating_mul(95) / 100;
    let fee_amount = compute_fee(min_return, fee_tier.rate_bps())
        .map_err(|e| BuilderError::Fee(e.to_string()))?;
    let args = create_single_hop_swap_args(req.amount_atomic, min_return, dex);
    let order_id = rand::random::<u64>();

    let mut instructions: Vec<Instruction> = Vec::new();
    let priority_fee = req.priority_fee_micro_lamports.unwrap_or(50_000);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(300_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee,
    ));

    // Ensure user WSOL ATA exists to receive swap output
    instructions.push(create_associated_token_account_idempotent(
        &wallet,
        &wallet,
        &WSOL_MINT,
        &spl_token::id(),
    ));

    // OKX Router swap_v3 instruction with on-chain platform fee routing
    let swap_ix =
        build_okx_swap_v3_instruction(&accounts, args, fee_tier.rate_bps(), order_id, None)
            .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    instructions.push(swap_ix);

    // Unwind WSOL to native SOL
    instructions.push(
        spl_token::instruction::close_account(
            &spl_token::id(),
            &user_wsol_ata,
            &wallet,
            &wallet,
            &[],
        )
        .map_err(|e| BuilderError::Serialization(e.to_string()))?,
    );

    let message = Message::try_compile(&wallet, &instructions, &[], recent_blockhash)
        .map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let versioned_msg = VersionedMessage::V0(message);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: versioned_msg,
    };

    let serialized_bytes =
        bincode::serialize(&tx).map_err(|e| BuilderError::Serialization(e.to_string()))?;
    let unsigned_tx_base64 = base64::engine::general_purpose::STANDARD.encode(&serialized_bytes);

    let intent_id = format!("intent_okx_{}_{}", mint, rand::random::<u32>());
    let (created_at, expires_at) = current_iso8601_timestamps();

    Ok(TradeIntent {
        api_version: 1,
        intent_id,
        created_at,
        expires_at,
        last_valid_block_height: current_slot + 150,
        wallet_pubkey: wallet,
        side: TradeSide::Sell,
        mint,
        route: TradeRoute::OkxRouter { dex_id: dex as u8 },
        input_amount_atomic: req.amount_atomic,
        minimum_output_atomic: min_return,
        slippage_bps,
        fee: ComputedFee {
            rate_bps: fee_tier.rate_bps(),
            basis: FeeBasis::QuoteAmount,
            recipient: fee_config.fee_wallet,
            amount_atomic: fee_amount,
        },
        state_slot: current_slot,
        unsigned_tx_base64,
    })
}
