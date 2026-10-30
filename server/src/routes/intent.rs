use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use serde_json::json;
use shared_types::platform::Platform;
use shared_types::token::TokenInfo;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;

use crate::builder::{
    CreateIntentRequest, build_buy_intent, build_okx_router_buy_intent,
    build_okx_router_sell_intent, build_sell_intent,
};
use crate::state::AppState;

/// Fetch latest blockhash and slot from the Solana RPC.
///
/// Fail closed: Returns an explicit error if RPC is unreachable or fails.
/// Never substitutes a fake blockhash or default slot.
async fn fetch_blockhash_and_slot(rpc_url: &str) -> Result<(Hash, u64), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(5000))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getLatestBlockhash",
        "params": [{"commitment": "confirmed"}]
    });

    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC blockhash request failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("RPC returned HTTP error status: {}", res.status()));
    }

    let json = res
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse RPC response: {}", e))?;

    if let Some(err) = json.get("error") {
        return Err(format!("RPC error response: {}", err));
    }

    let bh_str = json["result"]["value"]["blockhash"]
        .as_str()
        .ok_or_else(|| "Missing blockhash in RPC response".to_string())?;

    let hash = bh_str
        .parse::<Hash>()
        .map_err(|e| format!("Failed to parse blockhash: {}", e))?;

    let slot = json["result"]["context"]["slot"]
        .as_u64()
        .ok_or_else(|| "Missing slot in RPC response".to_string())?;

    Ok((hash, slot))
}

/// Fetch real on-chain bonding curve account state for a Pump.fun mint via RPC.
///
/// Fail closed: Returns an explicit error if the bonding curve account does not exist,
/// has already graduated/completed, or fails to decode. Never creates an intent from
/// placeholder reserves.
async fn fetch_pump_curve_state(
    rpc_url: &str,
    mint: &Pubkey,
) -> Result<(Pubkey, protocol_pump::curve::BondingCurveAccount), String> {
    let (curve_pda, _) = protocol_pump::addresses::find_bonding_curve(mint);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(5000))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            curve_pda.to_string(),
            {
                "encoding": "base64",
                "commitment": "confirmed"
            }
        ]
    });

    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC account fetch failed for curve {}: {}", curve_pda, e))?;

    if !res.status().is_success() {
        return Err(format!("RPC returned HTTP error status: {}", res.status()));
    }

    let json = res
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse RPC response: {}", e))?;

    if let Some(err) = json.get("error") {
        return Err(format!("RPC error response: {}", err));
    }

    let value = &json["result"]["value"];
    if value.is_null() {
        return Err(format!(
            "Bonding curve account ({}) does not exist on-chain for mint {}. Token may not be on Pump.fun or curve was not created.",
            curve_pda, mint
        ));
    }

    let data_arr = value["data"]
        .as_array()
        .ok_or_else(|| "Invalid account data field format in RPC response".to_string())?;

    let b64_str = data_arr
        .get(0)
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing base64 data string in account data array".to_string())?;

    let data_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_str)
        .map_err(|e| format!("Failed to decode base64 bonding curve data: {}", e))?;

    let curve_account = protocol_pump::curve::BondingCurveAccount::try_from_slice(&data_bytes)
        .map_err(|e| format!("Failed to decode bonding curve account: {:?}", e))?;

    if curve_account.complete {
        return Err(format!(
            "Bonding curve for mint {} has completed (migrated). Trades must use AMM/PumpSwap routes.",
            mint
        ));
    }

    Ok((curve_pda, curve_account))
}

pub async fn create_buy_intent_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateIntentRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let mint = payload.mint.parse::<Pubkey>().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid mint pubkey: {}", e) })),
        )
    })?;

    // Fail closed: Never substitute fake blockhash or slot if RPC fails
    let (recent_blockhash, current_slot) = fetch_blockhash_and_slot(&state.inner.rpc_url)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("Cannot create trade intent: {}", e) })),
            )
        })?;

    let is_okx = matches!(
        payload.platform,
        Some(Platform::OkxRouter | Platform::Raydium | Platform::Meteora | Platform::Orca)
    );

    let intent = if is_okx {
        build_okx_router_buy_intent(
            &payload,
            &state.inner.fee_config,
            recent_blockhash,
            current_slot,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
    } else {
        // Fail closed: Must fetch verified real on-chain bonding curve reserves; never use placeholder reserves
        let (bonding_curve_pda, curve_state) = fetch_pump_curve_state(&state.inner.rpc_url, &mint)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": format!("Cannot create trade intent: {}", e) })),
                )
            })?;

        let token_info = TokenInfo {
            name: "Pump Token".to_string(),
            symbol: "PUMP".to_string(),
            uri: "https://pump.fun".to_string(),
            mint,
            platform: Platform::PumpFun,
            bonding_curve: Some(bonding_curve_pda),
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
            virtual_token_reserves: Some(curve_state.virtual_token_reserves),
            virtual_quote_reserves: Some(curve_state.virtual_quote_reserves),
            real_token_reserves: Some(curve_state.real_token_reserves),
        };
        build_buy_intent(
            &payload,
            &state.inner.fee_config,
            &token_info,
            recent_blockhash,
            current_slot,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
    };

    Ok(Json(intent))
}

pub async fn create_sell_intent_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateIntentRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let mint = payload.mint.parse::<Pubkey>().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid mint pubkey: {}", e) })),
        )
    })?;

    // Fail closed: Never substitute fake blockhash or slot if RPC fails
    let (recent_blockhash, current_slot) = fetch_blockhash_and_slot(&state.inner.rpc_url)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("Cannot create trade intent: {}", e) })),
            )
        })?;

    let is_okx = matches!(
        payload.platform,
        Some(Platform::OkxRouter | Platform::Raydium | Platform::Meteora | Platform::Orca)
    );

    let intent = if is_okx {
        build_okx_router_sell_intent(
            &payload,
            &state.inner.fee_config,
            recent_blockhash,
            current_slot,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
    } else {
        // Fail closed: Must fetch verified real on-chain bonding curve reserves; never use placeholder reserves
        let (bonding_curve_pda, curve_state) = fetch_pump_curve_state(&state.inner.rpc_url, &mint)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": format!("Cannot create trade intent: {}", e) })),
                )
            })?;

        let token_info = TokenInfo {
            name: "Pump Token".to_string(),
            symbol: "PUMP".to_string(),
            uri: "https://pump.fun".to_string(),
            mint,
            platform: Platform::PumpFun,
            bonding_curve: Some(bonding_curve_pda),
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
            virtual_token_reserves: Some(curve_state.virtual_token_reserves),
            virtual_quote_reserves: Some(curve_state.virtual_quote_reserves),
            real_token_reserves: Some(curve_state.real_token_reserves),
        };
        build_sell_intent(
            &payload,
            &state.inner.fee_config,
            &token_info,
            recent_blockhash,
            current_slot,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
    };

    Ok(Json(intent))
}
