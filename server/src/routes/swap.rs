//! Unsigned Swap Transaction Preparation Route.
//!
//! Enforces Phase 3 contract requirements:
//! - Validates mints, exact atomic amounts, and trade side.
//! - Enforces quote freshness (fails if quote > 60s or slot lag > 150).
//! - Fetches real on-chain state and fresh recent blockhash via Solana RPC.
//! - Uses official Rust instruction builders for Pump.fun v2, PumpSwap AMM, and Jupiter Swap V2 /build.
//! - Enforces strict platform fee transfer to native CONFIGURED_PLATFORM_TREASURY.
//! - Supports Pump.fun, PumpSwap, and Jupiter/Raydium native routes.
//! - Never returns fabricated blockhashes, signatures, or success states.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::Engine;
use serde_json::json;
use solana_sdk::address_lookup_table::AddressLookupTableAccount;
use solana_sdk::address_lookup_table::program::id as address_lookup_table_program_id;
use solana_sdk::address_lookup_table::state::AddressLookupTable;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_instruction;
use solana_sdk::transaction::VersionedTransaction;
use std::str::FromStr;

use shared_types::fee::{CONFIGURED_PLATFORM_TREASURY, compute_fee};
use shared_types::intent::TradeSide;
use shared_types::platform::Platform;
use shared_types::swap::{
    PrepareSwapRequest, PreparedSwapCandidate, SwapAccountMetaSummary, SwapCandidateSummary,
    SwapInstructionDetail,
};
use shared_types::token::TokenInfo;

use crate::builder::current_iso8601_timestamps;
use crate::state::AppState;

pub const WSOL_MINT_STR: &str = "So11111111111111111111111111111111111111112";
pub const MAX_QUOTE_AGE_MS: u128 = 60_000; // 60 seconds max quote age
pub const MAX_SLOT_LAG: u64 = 150; // ~60 seconds max slot lag

/// Fetch latest blockhash and slot from Solana RPC.
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

/// Fetch real bonding curve state for Pump.fun, validating program owner and slot freshness.
async fn fetch_pump_curve_account(
    rpc_url: &str,
    mint: &Pubkey,
    current_slot: u64,
) -> Result<(Pubkey, protocol_pump::curve::BondingCurveAccount, u64), String> {
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
            { "encoding": "base64", "commitment": "confirmed" }
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
            "Bonding curve account ({}) does not exist on-chain for mint {}.",
            curve_pda, mint
        ));
    }

    // Verify program owner is Pump.fun program
    let owner_str = value["owner"].as_str().unwrap_or_default();
    if owner_str != protocol_pump::addresses::PUMP_PROGRAM.to_string() {
        return Err(format!(
            "Bonding curve account ({}) has unexpected owner: expected {}, got {}",
            curve_pda,
            protocol_pump::addresses::PUMP_PROGRAM,
            owner_str
        ));
    }

    let curve_slot = json["result"]["context"]["slot"].as_u64().unwrap_or(0);
    let max_future_drift = crate::routes::compare::MAX_CROSS_READ_SLOT_DRIFT;
    if curve_slot > current_slot && curve_slot - current_slot > max_future_drift {
        return Err(format!(
            "Pump.fun bonding curve state slot is in the future beyond allowed drift: curve slot {}, current slot {}",
            curve_slot, current_slot
        ));
    }
    let effective_current_slot = std::cmp::max(current_slot, curve_slot);
    if !crate::routes::compare::rpc_slot_is_fresh(curve_slot, effective_current_slot) {
        return Err(format!(
            "Pump.fun bonding curve state is stale (curve slot {}, current slot {})",
            curve_slot, current_slot
        ));
    }

    let data_arr = value["data"]
        .as_array()
        .ok_or_else(|| "Invalid account data format in RPC response".to_string())?;

    let b64_str = data_arr
        .get(0)
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing base64 data string in account data array".to_string())?;

    let data_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_str)
        .map_err(|e| format!("Failed to decode base64 bonding curve data: {}", e))?;

    let curve_account = protocol_pump::curve::BondingCurveAccount::try_from_slice(&data_bytes)
        .map_err(|e| format!("Failed to decode bonding curve account: {:?}", e))?;

    Ok((curve_pda, curve_account, curve_slot))
}

/// Fetch real PumpSwap pool state and vault balances from RPC, fully verifying owners and token pair mints.
async fn fetch_pumpswap_pool_and_vaults(
    rpc_url: &str,
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
    preferred_pool: Option<&Pubkey>,
    current_slot: u64,
) -> Result<
    (
        protocol_pumpswap::pool::PumpSwapPoolState,
        u64, // base reserve
        u64, // quote reserve
        Pubkey, // base token program (spl_token or token_2022)
        Pubkey, // quote token program (spl_token or token_2022)
        u64, // derived active fee bps
        u64, // synchronized state slot
    ),
    String,
> {
    let pool_pda = if let Some(p) = preferred_pool {
        *p
    } else {
        let (pda, _) = protocol_pumpswap::addresses::find_canonical_pool(base_mint, quote_mint);
        pda
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(5000))
        .build()
        .map_err(|e| format!("HTTP client build error: {}", e))?;

    // 1. Fetch pool account info
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            pool_pda.to_string(),
            { "encoding": "base64", "commitment": "confirmed" }
        ]
    });

    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC pool fetch failed: {}", e))?;

    let json = res
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse RPC pool response: {}", e))?;

    if let Some(err) = json.get("error") {
        return Err(format!("RPC error response: {}", err));
    }

    let value = &json["result"]["value"];
    if value.is_null() {
        return Err(format!("Canonical PumpSwap pool ({}) does not exist on-chain.", pool_pda));
    }

    // Verify pool program owner
    let pool_owner = value["owner"].as_str().unwrap_or_default();
    if pool_owner != protocol_pumpswap::addresses::PUMP_AMM_PROGRAM.to_string() {
        return Err(format!(
            "PumpSwap pool account ({}) has unexpected owner: expected {}, got {}",
            pool_pda,
            protocol_pumpswap::addresses::PUMP_AMM_PROGRAM,
            pool_owner
        ));
    }

    let pool_slot = json["result"]["context"]["slot"].as_u64().unwrap_or(0);

    let data_b64 = value["data"][0]
        .as_str()
        .ok_or_else(|| "Missing base64 data for pool account".to_string())?;

    let pool_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| format!("Base64 decode error for pool: {}", e))?;

    let pool_state = protocol_pumpswap::pool::PumpSwapPoolState::try_from_slice(pool_pda, &pool_bytes)
        .map_err(|e| format!("Failed to decode PumpSwap pool state: {:?}", e))?;

    // Verify token pair mints inside pool state
    if pool_state.base_mint != *base_mint {
        return Err(format!(
            "PumpSwap pool base mint mismatch: expected {}, got {}",
            base_mint, pool_state.base_mint
        ));
    }
    if pool_state.quote_mint != *quote_mint {
        return Err(format!(
            "PumpSwap pool quote mint mismatch: expected {}, got {}",
            quote_mint, pool_state.quote_mint
        ));
    }

    // 2. Fetch vaults and fee config PDA via getMultipleAccounts
    let (fee_config_pda, _) = protocol_pumpswap::addresses::find_fee_config();
    let vault_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "getMultipleAccounts",
        "params": [
            [
                pool_state.base_account.to_string(),
                pool_state.quote_account.to_string(),
                fee_config_pda.to_string()
            ],
            { "encoding": "base64", "commitment": "confirmed" }
        ]
    });

    let vault_res = client
        .post(rpc_url)
        .json(&vault_body)
        .send()
        .await
        .map_err(|e| format!("RPC vault fetch failed: {}", e))?;

    let vault_json = vault_res
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse RPC vault response: {}", e))?;

    let vault_slot = vault_json["result"]["context"]["slot"].as_u64().unwrap_or(0);

    let accounts = vault_json["result"]["value"]
        .as_array()
        .ok_or_else(|| "Expected array of vault accounts".to_string())?;

    if accounts.len() < 2 || accounts[0].is_null() || accounts[1].is_null() {
        return Err("One or both pool vault accounts do not exist on-chain".to_string());
    }

    let parse_and_verify_vault = |acc: &serde_json::Value, expected_mint: &Pubkey, label: &str| -> Result<(u64, Pubkey), String> {
        let acc_owner = acc["owner"].as_str().unwrap_or_default();
        let token_program = if acc_owner == spl_token::id().to_string() {
            spl_token::id()
        } else if acc_owner == protocol_pump::addresses::TOKEN_2022_PROGRAM.to_string() {
            protocol_pump::addresses::TOKEN_2022_PROGRAM
        } else {
            return Err(format!("{} vault account has unexpected program owner: {}", label, acc_owner));
        };

        let b64 = acc["data"][0]
            .as_str()
            .ok_or_else(|| format!("Missing {} vault data", label))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("{} vault base64 decode error: {}", label, e))?;
        if bytes.len() < 72 {
            return Err(format!("{} vault data too short for SPL Token account", label));
        }

        let mint_bytes: [u8; 32] = bytes[0..32].try_into().map_err(|_| "Failed to parse vault mint")?;
        let vault_mint = Pubkey::new_from_array(mint_bytes);
        if vault_mint != *expected_mint {
            return Err(format!("{} vault mint mismatch: expected {}, got {}", label, expected_mint, vault_mint));
        }

        let owner_bytes: [u8; 32] = bytes[32..64].try_into().map_err(|_| "Failed to parse vault owner")?;
        let vault_owner = Pubkey::new_from_array(owner_bytes);
        if vault_owner != pool_pda {
            return Err(format!("{} vault owner mismatch: expected pool {}, got {}", label, pool_pda, vault_owner));
        }

        let amount_bytes: [u8; 8] = bytes[64..72]
            .try_into()
            .map_err(|_| "Failed to slice token amount".to_string())?;
        Ok((u64::from_le_bytes(amount_bytes), token_program))
    };

    let (base_reserve, base_token_program) = parse_and_verify_vault(&accounts[0], base_mint, "Base")?;
    let (quote_reserve, quote_token_program) = parse_and_verify_vault(&accounts[1], quote_mint, "Quote")?;

    // Derive active dynamic fee tier from fee config PDA and pool creator fee
    let mut parsed_fee_config: Option<protocol_pumpswap::pool::PumpSwapFeeConfig> = None;
    if accounts.len() >= 3 && !accounts[2].is_null() {
        let fee_acc = &accounts[2];
        let acc_owner = fee_acc["owner"].as_str().unwrap_or_default();
        if acc_owner == protocol_pumpswap::addresses::PUMP_FEE_PROGRAM.to_string() {
            if let Some(b64) = fee_acc["data"][0].as_str() {
                if let Ok(fee_bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                    if let Ok(cfg) = protocol_pumpswap::pool::PumpSwapFeeConfig::try_from_slice(&fee_bytes) {
                        parsed_fee_config = Some(cfg);
                    }
                }
            }
        }
    }

    let effective_quote_for_fee = pool_state
        .effective_quote_reserve(quote_reserve)
        .unwrap_or(quote_reserve);
    let fee_bps = pool_state.derive_dynamic_fee_bps(
        parsed_fee_config.as_ref(),
        base_reserve,
        effective_quote_for_fee,
    );

    // 3. Verify cross-read slots against current slot with cluster drift protection
    let max_future_drift = crate::routes::compare::MAX_CROSS_READ_SLOT_DRIFT;
    if pool_slot > current_slot && pool_slot - current_slot > max_future_drift {
        return Err(format!(
            "PumpSwap pool state slot is in the future beyond allowed drift: pool slot {}, current slot {}",
            pool_slot, current_slot
        ));
    }
    if vault_slot > current_slot && vault_slot - current_slot > max_future_drift {
        return Err(format!(
            "PumpSwap vault state slot is in the future beyond allowed drift: vault slot {}, current slot {}",
            vault_slot, current_slot
        ));
    }
    let effective_current_slot = std::cmp::max(current_slot, std::cmp::max(pool_slot, vault_slot));
    let synchronized_slot = crate::routes::compare::verify_cross_read_slots(pool_slot, vault_slot, effective_current_slot)
        .map_err(|e| format!("PumpSwap slot synchronization check failed: {}", e))?;

    Ok((
        pool_state,
        base_reserve,
        quote_reserve,
        base_token_program,
        quote_token_program,
        fee_bps,
        synchronized_slot,
    ))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterAccountMetaJson {
    pubkey: String,
    is_signer: bool,
    is_writable: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterInstructionJson {
    program_id: String,
    accounts: Vec<JupiterAccountMetaJson>,
    data: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct JupiterBuildResponse {
    out_amount: String,
    #[serde(default)]
    other_amount_threshold: Option<String>,
    #[serde(default)]
    route_plan: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    token_ledger_instruction: Option<JupiterInstructionJson>,
    #[serde(default)]
    compute_budget_instructions: Option<Vec<JupiterInstructionJson>>,
    #[serde(default)]
    setup_instructions: Option<Vec<JupiterInstructionJson>>,
    swap_instruction: JupiterInstructionJson,
    #[serde(default)]
    cleanup_instruction: Option<JupiterInstructionJson>,
    #[serde(default)]
    other_instructions: Option<Vec<JupiterInstructionJson>>,
    #[serde(default)]
    address_lookup_table_addresses: Option<Vec<String>>,
    #[serde(default)]
    addresses_by_lookup_table_address: Option<std::collections::HashMap<String, Vec<String>>>,
}

fn convert_jupiter_instruction(json_ix: &JupiterInstructionJson) -> Result<Instruction, String> {
    let program_id = Pubkey::from_str(&json_ix.program_id)
        .map_err(|e| format!("Invalid Jupiter instruction program_id {}: {}", json_ix.program_id, e))?;

    let accounts = json_ix
        .accounts
        .iter()
        .map(|meta| {
            let pubkey = Pubkey::from_str(&meta.pubkey)
                .map_err(|e| format!("Invalid Jupiter instruction account pubkey {}: {}", meta.pubkey, e))?;
            Ok(solana_sdk::instruction::AccountMeta {
                pubkey,
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let data = base64::engine::general_purpose::STANDARD
        .decode(&json_ix.data)
        .map_err(|e| format!("Failed to base64 decode Jupiter instruction data: {}", e))?;

    Ok(Instruction {
        program_id,
        accounts,
        data,
    })
}

async fn fetch_address_lookup_tables(
    rpc_url: &str,
    table_addresses: &[String],
) -> Result<Vec<AddressLookupTableAccount>, String> {
    if table_addresses.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(5000))
        .build()
        .map_err(|e| format!("Failed to build HTTP client for ALT fetch: {}", e))?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getMultipleAccounts",
        "params": [
            table_addresses,
            { "encoding": "base64", "commitment": "confirmed" }
        ]
    });

    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC ALT request failed: {}", e))?;

    let json = res
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse RPC ALT response: {}", e))?;

    let accounts = json["result"]["value"]
        .as_array()
        .ok_or_else(|| "Missing result.value in getMultipleAccounts for ALTs".to_string())?;

    let mut lookup_tables = Vec::new();
    for (idx, acc_val) in accounts.iter().enumerate() {
        if acc_val.is_null() {
            return Err(format!("ALT account {} not found on-chain", table_addresses[idx]));
        }

        // Verify that the account owner is the official Solana Address Lookup Table program
        let owner_str = acc_val["owner"]
            .as_str()
            .ok_or_else(|| format!("Missing owner for ALT {}", table_addresses[idx]))?;
        let expected_owner = address_lookup_table_program_id().to_string();
        if owner_str != expected_owner {
            return Err(format!(
                "ALT account {} has invalid owner: expected {}, got {}",
                table_addresses[idx], expected_owner, owner_str
            ));
        }

        let b64 = acc_val["data"][0]
            .as_str()
            .ok_or_else(|| format!("Missing base64 data for ALT {}", table_addresses[idx]))?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("Failed to decode base64 for ALT {}: {}", table_addresses[idx], e))?;

        // Deserialize and validate lookup table state using official SDK
        let lookup_table = AddressLookupTable::deserialize(&data)
            .map_err(|e| format!("Failed to deserialize ALT {}: {:?}", table_addresses[idx], e))?;

        // Enforce that the lookup table is active (not deactivated)
        if lookup_table.meta.deactivation_slot != u64::MAX {
            return Err(format!(
                "ALT {} is inactive or deactivated (deactivation_slot: {})",
                table_addresses[idx], lookup_table.meta.deactivation_slot
            ));
        }

        let addresses = lookup_table.addresses.to_vec();
        let key = Pubkey::from_str(&table_addresses[idx])
            .map_err(|e| format!("Invalid ALT pubkey {}: {}", table_addresses[idx], e))?;

        lookup_tables.push(AddressLookupTableAccount {
            key,
            addresses,
        });
    }

    Ok(lookup_tables)
}

/// Helper to generate human-readable labels and descriptions for instruction accounts.
fn analyze_instruction(
    ix: &Instruction,
    wallet: &Pubkey,
    mint: &Pubkey,
    treasury: &Pubkey,
) -> SwapInstructionDetail {
    let program_id_str = ix.program_id.to_string();
    let program_name = if ix.program_id == solana_sdk::system_program::ID {
        "System Program".to_string()
    } else if ix.program_id == solana_sdk::compute_budget::id() {
        "Compute Budget Program".to_string()
    } else if ix.program_id == spl_associated_token_account::id() {
        "Associated Token Account Program".to_string()
    } else if ix.program_id == spl_token::id() {
        "SPL Token Program".to_string()
    } else if ix.program_id == protocol_pump::addresses::PUMP_PROGRAM {
        "Pump.fun Bonding Curve Program".to_string()
    } else if ix.program_id == protocol_pumpswap::addresses::PUMP_AMM_PROGRAM {
        "PumpSwap AMM Program".to_string()
    } else if ix.program_id == protocol_okx_router::addresses::OKX_ROUTER_PROGRAM_ID
        || ix.program_id == protocol_okx_router::addresses::OKX_ROUTER_DEVNET_PROGRAM_ID
    {
        "OKX DEX Router Program".to_string()
    } else {
        format!("Program ({})", program_id_str)
    };

    let mut description = format!("Invoke {}", program_name);

    if ix.program_id == solana_sdk::compute_budget::id() && !ix.data.is_empty() {
        match ix.data[0] {
            2 if ix.data.len() >= 5 => {
                let units = u32::from_le_bytes(ix.data[1..5].try_into().unwrap_or_default());
                description = format!("Set Compute Unit Limit to {}", units);
            }
            3 if ix.data.len() >= 9 => {
                let price = u64::from_le_bytes(ix.data[1..9].try_into().unwrap_or_default());
                description = format!("Set Compute Unit Price to {} micro-lamports/CU", price);
            }
            _ => {}
        }
    } else if ix.program_id == solana_sdk::system_program::ID && ix.data.len() >= 12 {
        let type_idx = u32::from_le_bytes(ix.data[0..4].try_into().unwrap_or_default());
        if type_idx == 2 && ix.accounts.len() >= 2 {
            let to = ix.accounts[1].pubkey;
            let lamports = u64::from_le_bytes(ix.data[4..12].try_into().unwrap_or_default());
            if to == *treasury {
                description = format!(
                    "Transfer platform fee of {} lamports to native treasury",
                    lamports
                );
            } else {
                description = format!("System transfer of {} lamports to {}", lamports, to);
            }
        }
    } else if ix.program_id == spl_associated_token_account::id() {
        description = "Create Associated Token Account (Idempotent)".to_string();
    } else if ix.program_id == protocol_pump::addresses::PUMP_PROGRAM {
        description = "Execute Swap on Pump.fun Bonding Curve".to_string();
    } else if ix.program_id == protocol_pumpswap::addresses::PUMP_AMM_PROGRAM {
        description = "Execute Swap on PumpSwap AMM".to_string();
    } else if ix.program_id == transaction_validation::allowlist::JUPITER_V6_PROGRAM {
        description = "Execute Swap via Jupiter Meta-Aggregator".to_string();
    } else if (ix.program_id == spl_token::id() || ix.program_id == protocol_pump::addresses::TOKEN_2022_PROGRAM)
        && !ix.data.is_empty()
        && ix.data[0] == 9
    {
        description = "Close Wrapped SOL Account (Unwrap WSOL proceeds to native SOL)".to_string();
    }

    let accounts = ix
        .accounts
        .iter()
        .map(|meta| {
            let label = if meta.pubkey == *wallet {
                Some("User Wallet (Signer & Fee Payer)".to_string())
            } else if meta.pubkey == *treasury {
                Some("Configured Platform Treasury".to_string())
            } else if meta.pubkey == *mint {
                Some("Target Token Mint".to_string())
            } else if meta.pubkey == ix.program_id {
                Some("Program Account".to_string())
            } else {
                None
            };

            SwapAccountMetaSummary {
                pubkey: meta.pubkey.to_string(),
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
                label,
            }
        })
        .collect();

    SwapInstructionDetail {
        program_id: program_id_str,
        program_name,
        accounts,
        data_len: ix.data.len(),
        description,
    }
}

/// Handler for `POST /api/v1/swap/prepare`.
pub async fn prepare_swap_candidate_handler(
    State(state): State<AppState>,
    Json(req): Json<PrepareSwapRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // 1. Quote freshness check: Fail closed if quote is expired (> 60s)
    let quote_age_ms = now.saturating_sub(req.quoted_at_unix_ms);
    if quote_age_ms > MAX_QUOTE_AGE_MS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "Quote is expired: age is {:.1}s (max allowed 60s). Please refresh the route quote.",
                    quote_age_ms as f64 / 1000.0
                )
            })),
        ));
    }

    // 2. Parse and validate wallet and mints
    let wallet = req.wallet_pubkey.parse::<Pubkey>().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid wallet pubkey: {}", e) })),
        )
    })?;

    let input_mint = req.input_mint.parse::<Pubkey>().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid input_mint pubkey: {}", e) })),
        )
    })?;

    let output_mint = req.output_mint.parse::<Pubkey>().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid output_mint pubkey: {}", e) })),
        )
    })?;

    let wsol = Pubkey::from_str(WSOL_MINT_STR).unwrap();

    let (trade_token_mint, is_buy) = match req.side {
        TradeSide::Buy => {
            if input_mint != wsol {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!("For buy side, input_mint must be WSOL ({}), got {}", WSOL_MINT_STR, input_mint)
                    })),
                ));
            }
            (output_mint, true)
        }
        TradeSide::Sell => {
            if output_mint != wsol {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!("For sell side, output_mint must be WSOL ({}), got {}", WSOL_MINT_STR, output_mint)
                    })),
                ));
            }
            (input_mint, false)
        }
    };

    if req.input_amount_atomic == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "input_amount_atomic must be greater than 0" })),
        ));
    }

    if req.expected_minimum_output_atomic == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "expected_minimum_output_atomic must be greater than 0 to bind the prepared candidate to a selected route quote" })),
        ));
    }

    // 3. Fetch fresh recent blockhash and slot from RPC (Fail-closed)
    let (recent_blockhash, current_slot) = fetch_blockhash_and_slot(&state.inner.rpc_url)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("Failed to fetch live blockhash from RPC: {}", e) })),
            )
        })?;

    // Check slot drift if state_slot was provided in the quote
    if let Some(quote_slot) = req.state_slot {
        let slot_lag = current_slot.saturating_sub(quote_slot);
        if slot_lag > MAX_SLOT_LAG {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!(
                        "Quote state is stale: slot lag is {} slots (max allowed {} slots).",
                        slot_lag, MAX_SLOT_LAG
                    )
                })),
            ));
        }
    }

    let fee_tier = req.fee_tier.unwrap_or(state.inner.fee_config.tier);
    let slippage_bps = req.slippage_bps;
    let priority_fee = req.priority_fee_micro_lamports.unwrap_or(50_000);
    let cu_limit = 200_000u32;

    // Platform fee calculation (enforcing native constant treasury)
    let configured_treasury = CONFIGURED_PLATFORM_TREASURY;

    let route_source = req.selected_route_source.trim().to_lowercase();
    let (instructions, estimated_output_atomic, minimum_output_atomic, platform_fee_atomic, venue_fee_atomic, rent_lamports, alts, candidate_venue, candidate_pool_address) =
        match route_source.as_str() {
            "pump" | "pump_bonding_curve" | "pump_fun_curve" | "pump_curve" | "pump_fun" | "pumpfun" => {
                let (curve_pda, curve_state, _curve_slot) =
                    fetch_pump_curve_account(&state.inner.rpc_url, &trade_token_mint, current_slot)
                        .await
                        .map_err(|e| {
                            (
                                StatusCode::BAD_GATEWAY,
                                Json(json!({ "error": format!("Failed to fetch pump curve state: {}", e) })),
                            )
                        })?;

                if curve_state.complete {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": format!(
                                "Bonding curve for mint {} has completed (graduated). Route candidate cannot be prepared on bonding curve; please select PumpSwap AMM.",
                                trade_token_mint
                            )
                        })),
                    ));
                }

                let token_info = TokenInfo {
                    name: "Pump Token".to_string(),
                    symbol: "PUMP".to_string(),
                    uri: "https://pump.fun".to_string(),
                    mint: trade_token_mint,
                    platform: Platform::PumpFun,
                    bonding_curve: Some(curve_pda),
                    quote_mint: wsol,
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

                let mut ixs = Vec::new();
                ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(cu_limit));
                ixs.push(ComputeBudgetInstruction::set_compute_unit_price(priority_fee));

                let (est_out, min_out, fee_lamports) = if is_buy {
                    let fee = compute_fee(req.input_amount_atomic, fee_tier.rate_bps())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let est = protocol_pump::curve::calculate_buy_tokens_out(
                        req.input_amount_atomic,
                        curve_state.virtual_token_reserves,
                        curve_state.virtual_quote_reserves,
                    )
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let calculated_min = protocol_pump::curve::apply_slippage_min(est, slippage_bps);

                    if calculated_min < req.expected_minimum_output_atomic {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Adverse price movement: fresh on-chain minimum output ({} atomic) is lower than selected route quote minimum ({} atomic). Market conditions changed; please refresh route quote.",
                                    calculated_min, req.expected_minimum_output_atomic
                                )
                            })),
                        ));
                    }
                    let min = calculated_min;

                    ixs.push(system_instruction::transfer(&wallet, &configured_treasury, fee));

                    let buy_ixs = protocol_pump::instructions::build_buy_v2_instructions(
                        &token_info,
                        &wallet,
                        min,
                        req.input_amount_atomic,
                    );
                    ixs.extend(buy_ixs);

                    (est, min, fee)
                } else {
                    let est_gross = protocol_pump::curve::calculate_sell_quote_out(
                        req.input_amount_atomic,
                        curve_state.virtual_token_reserves,
                        curve_state.virtual_quote_reserves,
                    )
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let calculated_min_gross = protocol_pump::curve::apply_slippage_min(est_gross, slippage_bps);
                    let rate_bps = fee_tier.rate_bps();

                    let fresh_fee = compute_fee(calculated_min_gross, rate_bps)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
                    let est_fee = compute_fee(est_gross, rate_bps)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let net_est = est_gross.saturating_sub(est_fee);
                    let calculated_net_min = calculated_min_gross.saturating_sub(fresh_fee);

                    if calculated_net_min < req.expected_minimum_output_atomic {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Adverse price movement: fresh on-chain net minimum output ({} atomic) is lower than selected route quote minimum ({} atomic). Market conditions changed; please refresh route quote.",
                                    calculated_net_min, req.expected_minimum_output_atomic
                                )
                            })),
                        ));
                    }
                    let net_min = req.expected_minimum_output_atomic;

                    // Derive the gross minimum bound from the bound net minimum and calculate the fee from that encoded gross bound,
                    // ensuring that even after favorable price movement, the server and native validator agree on the exact allowed fee.
                    let gross_min_bound = if rate_bps >= 10_000 {
                        net_min
                    } else {
                        let divisor = 10_000u128 - rate_bps as u128;
                        let gross_u128 = (net_min as u128 * 10_000u128 + (divisor - 1)) / divisor;
                        u64::try_from(gross_u128).map_err(|_| {
                            (
                                StatusCode::BAD_REQUEST,
                                Json(json!({
                                    "error": "Overflow calculating gross minimum output bound from net minimum and platform fee"
                                })),
                            )
                        })?
                    };

                    let fee = compute_fee(gross_min_bound, rate_bps)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    // 1. Swap sell instruction executed first so user wallet receives SOL proceeds
                    let sell_ixs = protocol_pump::instructions::build_sell_v2_instructions(
                        &token_info,
                        &wallet,
                        req.input_amount_atomic,
                        gross_min_bound,
                    );
                    ixs.extend(sell_ixs);

                    // 2. Fee deduction transfer executed second, deducting platform fee from trade proceeds
                    ixs.push(system_instruction::transfer(&wallet, &configured_treasury, fee));

                    (net_est, net_min, fee)
                };

                let rent = if is_buy { 2_039_280u64 } else { 0u64 };
                let alts: Vec<AddressLookupTableAccount> = Vec::new();
                (ixs, est_out, min_out, fee_lamports, None, rent, alts, req.venue.clone(), req.pool_address.clone())
            }
            "pumpswap" | "pumpswap_canonical" | "pumpswap_amm" => {
                let preferred_pool_pk = req.pool_address.as_ref().and_then(|p| p.parse::<Pubkey>().ok());
                let (
                    pool_state,
                    base_reserve,
                    quote_reserve,
                    base_token_program,
                    quote_token_program,
                    pumpswap_fee_bps,
                    _,
                ) = fetch_pumpswap_pool_and_vaults(&state.inner.rpc_url, &trade_token_mint, &wsol, preferred_pool_pk.as_ref(), current_slot)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::BAD_GATEWAY,
                            Json(json!({ "error": format!("Failed to fetch PumpSwap pool state: {}", e) })),
                        )
                    })?;

                let mut ixs = Vec::new();
                ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(cu_limit));
                ixs.push(ComputeBudgetInstruction::set_compute_unit_price(priority_fee));

                let effective_quote_reserve = pool_state
                    .effective_quote_reserve(quote_reserve)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{:?}", e) }))))?;

                let (est_out, min_out, fee_lamports, venue_fee) = if is_buy {
                    let fee = compute_fee(req.input_amount_atomic, fee_tier.rate_bps())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
                    let swap_in = req.input_amount_atomic.saturating_sub(fee);
                    if swap_in == 0 {
                        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Input amount too small after platform fee" }))));
                    }

                    let est = protocol_pumpswap::pool::calculate_amm_buy(
                        swap_in,
                        base_reserve,
                        effective_quote_reserve,
                        pumpswap_fee_bps,
                    )
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{:?}", e) }))))?;

                    let calculated_min = protocol_pump::curve::apply_slippage_min(est, slippage_bps);

                    if calculated_min < req.expected_minimum_output_atomic {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Adverse price movement: fresh on-chain minimum output ({} atomic) is lower than selected route quote minimum ({} atomic). Market conditions changed; please refresh route quote.",
                                    calculated_min, req.expected_minimum_output_atomic
                                )
                            })),
                        ));
                    }
                    let min = calculated_min;

                    let venue_f = (swap_in * pumpswap_fee_bps) / 10_000;

                    ixs.push(system_instruction::transfer(&wallet, &configured_treasury, fee));

                    let buy_ixs = protocol_pumpswap::instructions::build_pumpswap_buy_instructions(
                        &pool_state,
                        &wallet,
                        min,
                        swap_in,
                        &base_token_program,
                        &quote_token_program,
                    );
                    ixs.extend(buy_ixs);

                    (est, min, fee, Some(venue_f))
                } else {
                    let est_gross = protocol_pumpswap::pool::calculate_amm_sell(
                        req.input_amount_atomic,
                        base_reserve,
                        effective_quote_reserve,
                        pumpswap_fee_bps,
                    )
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{:?}", e) }))))?;

                    let calculated_min_gross = protocol_pump::curve::apply_slippage_min(est_gross, slippage_bps);
                    let rate_bps = fee_tier.rate_bps();

                    let fresh_fee = compute_fee(calculated_min_gross, rate_bps)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
                    let est_fee = compute_fee(est_gross, rate_bps)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let net_est = est_gross.saturating_sub(est_fee);
                    let calculated_net_min = calculated_min_gross.saturating_sub(fresh_fee);

                    if calculated_net_min < req.expected_minimum_output_atomic {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Adverse price movement: fresh on-chain net minimum output ({} atomic) is lower than selected route quote minimum ({} atomic). Market conditions changed; please refresh route quote.",
                                    calculated_net_min, req.expected_minimum_output_atomic
                                )
                            })),
                        ));
                    }
                    let net_min = req.expected_minimum_output_atomic;

                    // Derive the gross minimum bound from the bound net minimum and calculate the fee from that encoded gross bound,
                    // ensuring that even after favorable price movement, the server and native validator agree on the exact allowed fee.
                    let gross_min_bound = if rate_bps >= 10_000 {
                        net_min
                    } else {
                        let divisor = 10_000u128 - rate_bps as u128;
                        let gross_u128 = (net_min as u128 * 10_000u128 + (divisor - 1)) / divisor;
                        u64::try_from(gross_u128).map_err(|_| {
                            (
                                StatusCode::BAD_REQUEST,
                                Json(json!({
                                    "error": "Overflow calculating gross minimum output bound from net minimum and platform fee"
                                })),
                            )
                        })?
                    };

                    let fee = compute_fee(gross_min_bound, rate_bps)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let venue_f = (est_gross * pumpswap_fee_bps) / 10_000;

                    // 1. Swap sell instruction executed first so user wallet receives SOL proceeds
                    let sell_ixs = protocol_pumpswap::instructions::build_pumpswap_sell_instructions(
                        &pool_state,
                        &wallet,
                        req.input_amount_atomic,
                        gross_min_bound,
                        &base_token_program,
                        &quote_token_program,
                    );
                    ixs.extend(sell_ixs);

                    // 2. Fee deduction transfer executed second, deducting platform fee from trade proceeds
                    ixs.push(system_instruction::transfer(&wallet, &configured_treasury, fee));

                    (net_est, net_min, fee, Some(venue_f))
                };

                let rent = if is_buy { 2_039_280u64 } else { 0u64 };
                let alts: Vec<AddressLookupTableAccount> = Vec::new();
                (ixs, est_out, min_out, fee_lamports, venue_fee, rent, alts, req.venue.clone(), req.pool_address.clone())
            }
            "okx_router" | "okx" => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "OKX DEX Router candidate preparation is not supported. OKX Router requires dynamic off-chain route resolution and pool-specific remaining accounts (the builder supplies an empty pool-account list and zero commission), so it cannot currently establish a working swap or collect platform fees. Live swap candidate preparation is restricted to natively verified Pump.fun bonding curves and PumpSwap canonical AMMs."
                    })),
                ));
            }
            "jupiter" | "jupiter_v2_order" | "jupiter_v6" | "jupiter_swap" | "raydium" | "raydium_clmm" | "raydium_cpmm" | "raydium_amm" => {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_millis(8000))
                    .build()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                let is_raydium_requested = req.venue.to_lowercase().contains("raydium")
                    || req.selected_route_source.to_lowercase().contains("raydium");
                let dex_filter = if is_raydium_requested {
                    "&dexes=Raydium,Raydium%20CLMM,Raydium%20CP"
                } else {
                    ""
                };

                let (est_out, min_out, fee_lamports, build_data) = if is_buy {
                    let fee = compute_fee(req.input_amount_atomic, fee_tier.rate_bps())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
                    let swap_in = req.input_amount_atomic.saturating_sub(fee);
                    if swap_in == 0 {
                        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Input amount too small after platform fee" }))));
                    }

                    // Call Jupiter Swap V2 /build endpoint
                    let build_url = format!(
                        "https://api.jup.ag/swap/v2/build?inputMint={}&outputMint={}&amount={}&taker={}&slippageBps={}&transactionVersion=0&maxAccounts=30{}",
                        WSOL_MINT_STR, trade_token_mint, swap_in, wallet, slippage_bps, dex_filter
                    );
                    let mut b_req = client.get(&build_url);
                    if let Ok(k) = std::env::var("JUPITER_API_KEY") {
                        if !k.trim().is_empty() { b_req = b_req.header("x-api-key", k); }
                    }
                    let b_res = b_req.send().await
                        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Jupiter build request failed: {}", e) }))))?;
                    if !b_res.status().is_success() {
                        let status = b_res.status();
                        let err_text = b_res.text().await.unwrap_or_default();
                        return Err((StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Jupiter build failed (HTTP {}): {}", status, err_text) }))));
                    }
                    let b_data = b_res.json::<JupiterBuildResponse>().await
                        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Failed to parse Jupiter build response: {}", e) }))))?;

                    let est = b_data.out_amount.parse::<u64>()
                        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Invalid outAmount from Jupiter: {}", e) }))))?;
                    let on_chain_min = if slippage_bps >= 10_000 {
                        0u64
                    } else {
                        ((est as u128 * (10_000u128 - slippage_bps as u128)) / 10_000u128) as u64
                    };
                    let min = on_chain_min;

                    if min < req.expected_minimum_output_atomic {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Adverse price movement: fresh on-chain minimum output ({} atomic) is lower than selected route quote minimum ({} atomic). Market conditions changed; please refresh route quote.",
                                    min, req.expected_minimum_output_atomic
                                )
                            })),
                        ));
                    }

                    (est, min, fee, b_data)
                } else {
                    // Call Jupiter Swap V2 /build endpoint for sell
                    let build_url = format!(
                        "https://api.jup.ag/swap/v2/build?inputMint={}&outputMint={}&amount={}&taker={}&slippageBps={}&transactionVersion=0&maxAccounts=30{}",
                        trade_token_mint, WSOL_MINT_STR, req.input_amount_atomic, wallet, slippage_bps, dex_filter
                    );
                    let mut b_req = client.get(&build_url);
                    if let Ok(k) = std::env::var("JUPITER_API_KEY") {
                        if !k.trim().is_empty() { b_req = b_req.header("x-api-key", k); }
                    }
                    let b_res = b_req.send().await
                        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Jupiter build request failed: {}", e) }))))?;
                    if !b_res.status().is_success() {
                        let status = b_res.status();
                        let err_text = b_res.text().await.unwrap_or_default();
                        return Err((StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Jupiter build failed (HTTP {}): {}", status, err_text) }))));
                    }
                    let b_data = b_res.json::<JupiterBuildResponse>().await
                        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Failed to parse Jupiter build response: {}", e) }))))?;

                    let est_gross = b_data.out_amount.parse::<u64>()
                        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Invalid outAmount from Jupiter: {}", e) }))))?;
                    let on_chain_min_gross = if slippage_bps >= 10_000 {
                        0u64
                    } else {
                        ((est_gross as u128 * (10_000u128 - slippage_bps as u128)) / 10_000u128) as u64
                    };
                    let min_gross = on_chain_min_gross;

                    let fee = compute_fee(min_gross, fee_tier.rate_bps())
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

                    let net_min = min_gross.saturating_sub(fee);
                    if net_min < req.expected_minimum_output_atomic {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Adverse price movement: fresh on-chain net minimum output ({} atomic) is lower than selected route quote minimum ({} atomic). Market conditions changed; please refresh route quote.",
                                    net_min, req.expected_minimum_output_atomic
                                )
                            })),
                        ));
                    }
                    let net_est = est_gross.saturating_sub(compute_fee(est_gross, fee_tier.rate_bps()).unwrap_or(0));

                    (net_est, net_min, fee, b_data)
                };

                let mut ixs = Vec::new();

                // Compute budget
                if let Some(cb_ixs) = &build_data.compute_budget_instructions {
                    for cb in cb_ixs {
                        ixs.push(convert_jupiter_instruction(cb).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?);
                    }
                } else {
                    ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(cu_limit));
                    ixs.push(ComputeBudgetInstruction::set_compute_unit_price(priority_fee));
                }

                if is_buy {
                    // Prepend platform fee transfer before swap instructions
                    ixs.push(system_instruction::transfer(&wallet, &configured_treasury, fee_lamports));
                }

                // Setup instructions
                if let Some(setups) = &build_data.setup_instructions {
                    for s in setups {
                        ixs.push(convert_jupiter_instruction(s).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?);
                    }
                }

                // Swap instruction
                ixs.push(convert_jupiter_instruction(&build_data.swap_instruction).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?);

                // Cleanup instruction
                if let Some(cleanup) = &build_data.cleanup_instruction {
                    ixs.push(convert_jupiter_instruction(cleanup).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?);
                }

                // Other instructions
                if let Some(other_ixs) = &build_data.other_instructions {
                    for o in other_ixs {
                        ixs.push(convert_jupiter_instruction(o).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?);
                    }
                }

                if !is_buy {
                    // Append platform fee transfer after trade execution and WSOL unwrapping
                    ixs.push(system_instruction::transfer(&wallet, &configured_treasury, fee_lamports));
                }

                // Fetch address lookup tables: support both addresses_by_lookup_table_address and address_lookup_table_addresses
                let mut alt_addresses: Vec<String> = build_data.address_lookup_table_addresses.clone().unwrap_or_default();
                if alt_addresses.is_empty() {
                    if let Some(map) = &build_data.addresses_by_lookup_table_address {
                        alt_addresses = map.keys().cloned().collect();
                    }
                }
                let alts = fetch_address_lookup_tables(&state.inner.rpc_url, &alt_addresses).await
                    .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": format!("Failed to fetch ALTs: {}", e) }))))?;

                // Extract DEX labels and pool address from Jupiter routePlan
                let mut dex_labels = Vec::new();
                let mut extracted_pool_address: Option<String> = None;

                if let Some(route_plan) = &build_data.route_plan {
                    for step in route_plan {
                        if let Some(swap_info) = step.get("swapInfo") {
                            if let Some(label) = swap_info.get("label").and_then(|l| l.as_str()) {
                                if !dex_labels.contains(&label.to_string()) {
                                    dex_labels.push(label.to_string());
                                }
                            }
                            if extracted_pool_address.is_none() {
                                if let Some(amm_key) = swap_info.get("ammKey").and_then(|k| k.as_str()) {
                                    extracted_pool_address = Some(amm_key.to_string());
                                }
                            }
                        }
                    }
                }

                // Raydium verification: fail closed if Raydium was requested but routePlan does not confirm Raydium pools
                if is_raydium_requested {
                    if dex_labels.is_empty() || !dex_labels.iter().any(|l| l.to_lowercase().contains("raydium")) {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({
                                "error": format!(
                                    "Requested Raydium route, but Jupiter quote routePlan did not confirm Raydium pool execution: labels={:?}",
                                    dex_labels
                                )
                            })),
                        ));
                    }
                }

                let actual_venue = if !dex_labels.is_empty() {
                    format!("Jupiter ({})", dex_labels.join(", "))
                } else {
                    "Jupiter".to_string()
                };

                let final_pool_address = extracted_pool_address.or_else(|| req.pool_address.clone());

                (ixs, est_out, min_out, fee_lamports, None, 0u64, alts, actual_venue, final_pool_address)
            }
            other => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!(
                            "Route source '{}' is not supported for transaction preparation in Phase 3. Only Pump.fun, PumpSwap, and Jupiter native routes are available for candidate preparation.",
                            other
                        )
                    })),
                ));
            }
        };

    // Compile message into VersionedTransaction (v0)
    let message = Message::try_compile(&wallet, &instructions, &alts, recent_blockhash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    let versioned_msg = VersionedMessage::V0(message);
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: versioned_msg,
    };

    let tx_bytes = bincode::serialize(&tx)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    let unsigned_tx_base64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    let (created_at, expires_at) = current_iso8601_timestamps();
    let candidate_id = format!("candidate_{}_{}", trade_token_mint, rand::random::<u32>());

    let summary = extract_tx_summary(
        &tx,
        &wallet,
        &trade_token_mint,
        &configured_treasury,
        fee_tier.rate_bps(),
        platform_fee_atomic,
        venue_fee_atomic,
        rent_lamports,
        req.input_amount_atomic,
        is_buy,
        now.saturating_sub(req.quoted_at_unix_ms),
        &alts,
    );

    Ok(Json(PreparedSwapCandidate {
        candidate_id,
        selected_route_source: req.selected_route_source,
        venue: candidate_venue,
        pool_address: candidate_pool_address,
        input_mint: req.input_mint,
        output_mint: req.output_mint,
        input_amount_atomic: req.input_amount_atomic,
        estimated_output_atomic,
        minimum_output_atomic,
        slippage_bps,
        recent_blockhash: recent_blockhash.to_string(),
        last_valid_block_height: current_slot + 150,
        state_slot: current_slot,
        created_at,
        expires_at,
        unsigned_tx_base64,
        summary,
        address_lookup_tables: alts
            .iter()
            .map(|a| shared_types::swap::AddressLookupTableEntry {
                key: a.key.to_string(),
                addresses: a.addresses.iter().map(|p| p.to_string()).collect(),
            })
            .collect(),
    }))
}

/// Helper to extract structured summary from a compiled VersionedTransaction.
pub fn extract_tx_summary(
    tx: &VersionedTransaction,
    wallet: &Pubkey,
    mint: &Pubkey,
    treasury: &Pubkey,
    fee_bps: u16,
    fee_atomic: u64,
    venue_fee: Option<u64>,
    rent_lamports: u64,
    input_amount_atomic: u64,
    is_buy: bool,
    quote_freshness_ms: u128,
    alts: &[AddressLookupTableAccount],
) -> SwapCandidateSummary {
    let msg = &tx.message;
    let static_keys = msg.static_account_keys();

    // Resolve loaded ALT addresses if any
    let (loaded_writable, loaded_readonly) = match msg {
        VersionedMessage::V0(v0_msg) => {
            let mut w = Vec::new();
            let mut r = Vec::new();
            for lookup in &v0_msg.address_table_lookups {
                if let Some(table) = alts.iter().find(|a| a.key == lookup.account_key) {
                    for &idx in &lookup.writable_indexes {
                        if let Some(&pk) = table.addresses.get(idx as usize) {
                            w.push(pk);
                        }
                    }
                    for &idx in &lookup.readonly_indexes {
                        if let Some(&pk) = table.addresses.get(idx as usize) {
                            r.push(pk);
                        }
                    }
                }
            }
            (w, r)
        }
        VersionedMessage::Legacy(_) => (Vec::new(), Vec::new()),
    };

    let mut all_keys = Vec::with_capacity(static_keys.len() + loaded_writable.len() + loaded_readonly.len());
    all_keys.extend_from_slice(static_keys);
    all_keys.extend_from_slice(&loaded_writable);
    all_keys.extend_from_slice(&loaded_readonly);

    let mut instructions = Vec::new();
    let mut program_ids = Vec::new();
    let mut writable_accounts = Vec::new();
    let mut signer_accounts = Vec::new();
    let mut cu_limit = 200_000u32;
    let mut priority_fee = 0u64;

    for (idx, key) in static_keys.iter().enumerate() {
        if msg.is_signer(idx) && !signer_accounts.contains(&key.to_string()) {
            signer_accounts.push(key.to_string());
        }
        if msg.is_maybe_writable(idx, None) && !writable_accounts.contains(&key.to_string()) {
            writable_accounts.push(key.to_string());
        }
    }

    for key in &loaded_writable {
        let key_str = key.to_string();
        if !writable_accounts.contains(&key_str) {
            writable_accounts.push(key_str);
        }
    }

    let compiled_ixs = match msg {
        VersionedMessage::Legacy(m) => &m.instructions,
        VersionedMessage::V0(m) => &m.instructions,
    };

    for c_ix in compiled_ixs {
        let prog_key = all_keys.get(c_ix.program_id_index as usize).copied().unwrap_or_default();
        if !program_ids.contains(&prog_key.to_string()) {
            program_ids.push(prog_key.to_string());
        }

        let decompiled_accounts = c_ix
            .accounts
            .iter()
            .map(|acc_idx| {
                let pk = all_keys.get(*acc_idx as usize).copied().unwrap_or_default();
                let is_signer = if (*acc_idx as usize) < static_keys.len() {
                    msg.is_signer(*acc_idx as usize)
                } else {
                    false
                };
                let is_writable = if (*acc_idx as usize) < static_keys.len() {
                    msg.is_maybe_writable(*acc_idx as usize, None)
                } else {
                    let loaded_idx = (*acc_idx as usize) - static_keys.len();
                    loaded_idx < loaded_writable.len()
                };
                solana_sdk::instruction::AccountMeta {
                    pubkey: pk,
                    is_signer,
                    is_writable,
                }
            })
            .collect();

        let ix = Instruction {
            program_id: prog_key,
            accounts: decompiled_accounts,
            data: c_ix.data.clone(),
        };

        if prog_key == solana_sdk::compute_budget::id() && !ix.data.is_empty() {
            if ix.data[0] == 2 && ix.data.len() >= 5 {
                cu_limit = u32::from_le_bytes(ix.data[1..5].try_into().unwrap_or_default());
            } else if ix.data[0] == 3 && ix.data.len() >= 9 {
                priority_fee = u64::from_le_bytes(ix.data[1..9].try_into().unwrap_or_default());
            }
        }

        instructions.push(analyze_instruction(&ix, wallet, mint, treasury));
    }

    let mut total_sol = 5_000; // base network fee
    if priority_fee > 0 {
        total_sol += (priority_fee * cu_limit as u64) / 1_000_000;
    }
    if is_buy {
        total_sol += input_amount_atomic;
    }
    total_sol += fee_atomic;
    total_sol += rent_lamports;

    let tx_byte_size = bincode::serialize(tx).map(|b| b.len()).unwrap_or(0);

    SwapCandidateSummary {
        instructions,
        program_ids,
        writable_accounts,
        signer_accounts,
        compute_unit_limit: cu_limit,
        priority_fee_micro_lamports: priority_fee,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: fee_atomic,
        platform_fee_recipient: treasury.to_string(),
        venue_fee_atomic: venue_fee,
        estimated_rent_lamports: rent_lamports,
        total_sol_required_atomic: total_sol,
        tx_byte_size,
        quote_freshness_ms,
    }
}
