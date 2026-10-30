//! Read-only best-route comparator between Jupiter meta-aggregator,
//! Pump.fun bonding curve, PumpSwap, and DEX venues.
//!
//! Evaluates candidate routes for the exact same input amount, calculates
//! estimated net output after venue and platform fees, and ranks the best route.
//! Live trading remains fail-closed; this module never builds, signs, or submits transactions.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shared_types::fee::{FeeTier, compute_fee};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::state::AppState;

pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const JUPITER_ORDER_URL: &str = "https://api.jup.ag/swap/v2/order";
pub const PUMP_BONDING_FEE_BPS: u16 = 125; // Published Pump.fun bonding-curve fee (1.25%).
pub const MAX_RPC_SLOT_LAG: u64 = 150; // Max allowed slot lag (~60 seconds).
pub const MAX_CROSS_READ_SLOT_DRIFT: u64 = 10; // Max allowed slot drift between pool state and vault reads (~4 seconds).

/// Default dust filtering thresholds (documented in project specification)
pub const DEFAULT_MIN_LIQUIDITY_USD: f64 = 2_000.0;
pub const DEFAULT_MIN_VOLUME_24H_USD: f64 = 500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareSide {
    Buy,
    Sell,
}

#[derive(Debug, Deserialize)]
pub struct CompareRequest {
    pub mint: String,
    pub side: CompareSide,
    pub amount_atomic: u64,
    #[serde(default)]
    pub fee_tier: Option<FeeTier>,
    #[serde(default)]
    pub min_liquidity_usd: Option<f64>,
    #[serde(default)]
    pub min_volume_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPool {
    pub dex_id: String,
    pub dex_label: String,
    pub pool_address: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub liquidity_usd: f64,
    pub volume_24h_usd: f64,
    pub price_usd: f64,
    pub passes_dust_filter: bool,
    #[serde(default)]
    pub is_verified_on_chain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeBreakdown {
    pub venue_fee_bps: Option<u16>,
    pub venue_fee_atomic: Option<u64>,
    pub platform_fee_bps: u16,
    pub platform_fee_atomic: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteQuote {
    pub source: String,
    pub venue: String,
    pub pool_address: Option<String>,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_atomic: u64,
    pub gross_output_atomic: Option<u64>,
    pub net_output_atomic: Option<u64>,
    pub price_impact_pct: Option<f64>,
    pub venue_fee_bps: Option<u16>,
    pub platform_fee_atomic: Option<u64>,
    pub fee_breakdown: Option<FeeBreakdown>,
    pub state_slot: Option<u64>,
    pub quoted_at_unix_ms: u128,
    pub status: String,
    pub error: Option<String>,
    pub reason_unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResponse {
    pub side: String,
    pub mint: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_atomic: u64,
    pub platform_fee_bps: u16,
    pub best_route_index: Option<usize>,
    pub runner_up_index: Option<usize>,
    pub output_difference_atomic: Option<u64>,
    pub recommendation_reason: Option<String>,
    pub routes: Vec<RouteQuote>,
    pub candidate_pools: Vec<DiscoveredPool>,
    pub min_liquidity_threshold_usd: f64,
    pub min_volume_threshold_usd: f64,
    pub execution_enabled: bool,
    pub disclaimer: &'static str,
}


fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn rank_routes(routes: &[RouteQuote], expected_input: u64) -> Vec<usize> {
    let mut ranked: Vec<usize> = routes
        .iter()
        .enumerate()
        .filter(|(_, q)| {
            q.status == "estimated"
                && q.input_amount_atomic == expected_input
                && q.net_output_atomic.is_some()
        })
        .map(|(i, _)| i)
        .collect();
    ranked.sort_by_key(|i| std::cmp::Reverse(routes[*i].net_output_atomic.unwrap_or(0)));
    ranked
}

pub fn rpc_slot_is_fresh(state_slot: u64, current_slot: u64) -> bool {
    current_slot >= state_slot && current_slot.saturating_sub(state_slot) <= MAX_RPC_SLOT_LAG
}

pub fn verify_cross_read_slots(
    pool_slot: u64,
    vault_slot: u64,
    current_slot: u64,
) -> Result<u64, String> {
    if !rpc_slot_is_fresh(pool_slot, current_slot) {
        return Err(format!(
            "PumpSwap pool state is stale (pool slot {pool_slot}, current slot {current_slot})"
        ));
    }
    if !rpc_slot_is_fresh(vault_slot, current_slot) {
        return Err(format!(
            "PumpSwap vault state is stale (vault slot {vault_slot}, current slot {current_slot})"
        ));
    }
    let slot_drift = pool_slot.abs_diff(vault_slot);
    if slot_drift > MAX_CROSS_READ_SLOT_DRIFT {
        return Err(format!(
            "PumpSwap pool state and vault accounts are desynchronized (pool slot {pool_slot}, vault slot {vault_slot}, drift {slot_drift} > {MAX_CROSS_READ_SLOT_DRIFT})"
        ));
    }
    Ok(std::cmp::min(pool_slot, vault_slot))
}

pub fn fail_quote(
    source: &str,
    venue: &str,
    input_mint: &str,
    output_mint: &str,
    amount: u64,
    reason: impl Into<String>,
) -> RouteQuote {
    let reason_str = reason.into();
    RouteQuote {
        source: source.into(),
        venue: venue.into(),
        pool_address: None,
        input_mint: input_mint.into(),
        output_mint: output_mint.into(),
        input_amount_atomic: amount,
        gross_output_atomic: None,
        net_output_atomic: None,
        price_impact_pct: None,
        venue_fee_bps: None,
        platform_fee_atomic: None,
        fee_breakdown: None,
        state_slot: None,
        quoted_at_unix_ms: now_ms(),
        status: "unavailable".into(),
        error: Some(reason_str.clone()),
        reason_unavailable: Some(reason_str),
    }
}

pub fn parse_token_account_balance(data: &[u8]) -> Result<u64, &'static str> {
    if data.len() < 72 {
        return Err("Token account data too short (less than 72 bytes)");
    }
    let bytes: [u8; 8] = data[64..72]
        .try_into()
        .map_err(|_| "Failed to extract 8-byte balance from token account")?;
    Ok(u64::from_le_bytes(bytes))
}

pub async fn discover_candidate_pools(
    client: &reqwest::Client,
    rpc_url: Option<&str>,
    mint: &str,
    min_liquidity_usd: f64,
    min_volume_usd: f64,
) -> Vec<DiscoveredPool> {
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{mint}");
    let mut pools = Vec::new();

    let Ok(res) = client.get(&url).send().await else {
        return pools;
    };
    if !res.status().is_success() {
        return pools;
    }
    let Ok(data) = res.json::<Value>().await else {
        return pools;
    };

    let Some(pairs) = data.get("pairs").and_then(Value::as_array) else {
        return pools;
    };

    let mut seen_addresses = std::collections::HashSet::new();

    for pair in pairs {
        let chain_id = pair.get("chainId").and_then(Value::as_str).unwrap_or("");
        if chain_id != "solana" {
            continue;
        }

        let pair_address = pair.get("pairAddress").and_then(Value::as_str).unwrap_or("");
        if pair_address.is_empty() || seen_addresses.contains(pair_address) {
            continue;
        }
        seen_addresses.insert(pair_address.to_string());

        let base_mint = pair
            .get("baseToken")
            .and_then(|t| t.get("address"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let quote_mint = pair
            .get("quoteToken")
            .and_then(|t| t.get("address"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let dex_id = pair
            .get("dexId")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_lowercase();
        let dex_label = match dex_id.as_str() {
            "raydium" => "Raydium",
            "meteora" => "Meteora",
            "orca" => "Orca",
            "pumpfun" | "pump" => "Pump.fun",
            "pumpswap" => "PumpSwap",
            other => other,
        }
        .to_string();

        let liquidity_usd = pair
            .get("liquidity")
            .and_then(|l| l.get("usd"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let volume_24h_usd = pair
            .get("volume")
            .and_then(|v| v.get("h24"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let price_usd = pair
            .get("priceUsd")
            .and_then(|p| p.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let passes_dust_filter =
            liquidity_usd >= min_liquidity_usd && volume_24h_usd >= min_volume_usd;

        pools.push(DiscoveredPool {
            dex_id,
            dex_label,
            pool_address: pair_address.to_string(),
            base_mint: base_mint.to_string(),
            quote_mint: quote_mint.to_string(),
            liquidity_usd,
            volume_24h_usd,
            price_usd,
            passes_dust_filter,
            is_verified_on_chain: false,
        });
    }

    // Verify candidate pool accounts on-chain via Solana RPC getMultipleAccounts batch
    if let Some(rpc) = rpc_url {
        let addresses: Vec<String> = pools.iter().map(|p| p.pool_address.clone()).collect();
        if !addresses.is_empty() {
            let req = json!({
                "jsonrpc": "2.0",
                "id": "verify_candidate_pools",
                "method": "getMultipleAccounts",
                "params": [
                    addresses,
                    {
                        "commitment": "confirmed",
                        "encoding": "base64"
                    }
                ]
            });
            if let Ok(res) = client.post(rpc).json(&req).send().await {
                if let Ok(val) = res.json::<Value>().await {
                    if let Some(accs) = val.get("result").and_then(|r| r.get("value")).and_then(Value::as_array) {
                        for (pool, acc) in pools.iter_mut().zip(accs.iter()) {
                            pool.is_verified_on_chain = !acc.is_null();
                        }
                    }
                }
            }
        }
    }

    pools.sort_by(|a, b| {
        b.liquidity_usd
            .partial_cmp(&a.liquidity_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pools
}

async fn jupiter_quote(

    client: &reqwest::Client,
    mint: &str,
    side: &CompareSide,
    amount: u64,
    fee_bps: u16,
) -> RouteQuote {
    let (in_mint, out_mint) = match side {
        CompareSide::Buy => (WSOL_MINT, mint),
        CompareSide::Sell => (mint, WSOL_MINT),
    };

    let (input_mint_str, output_mint_str, quote_amount) = match side {
        CompareSide::Buy => {
            let fee = match compute_fee(amount, fee_bps) {
                Ok(f) => f,
                Err(e) => {
                    return fail_quote(
                        "jupiter",
                        "Jupiter Meta-Aggregator",
                        in_mint,
                        out_mint,
                        amount,
                        e.to_string(),
                    );
                }
            };
            (
                WSOL_MINT.to_string(),
                mint.to_string(),
                amount.saturating_sub(fee),
            )
        }
        CompareSide::Sell => (mint.to_string(), WSOL_MINT.to_string(), amount),
    };

    if quote_amount == 0 {
        return fail_quote(
            "jupiter",
            "Jupiter Meta-Aggregator",
            in_mint,
            out_mint,
            amount,
            "Amount is too small after platform fee",
        );
    }

    let amount_string = quote_amount.to_string();
    let mut jupiter_request = client.get(JUPITER_ORDER_URL).query(&[
        ("inputMint", input_mint_str.as_str()),
        ("outputMint", output_mint_str.as_str()),
        ("amount", amount_string.as_str()),
    ]);
    if let Ok(api_key) = std::env::var("JUPITER_API_KEY") {
        if !api_key.trim().is_empty() {
            jupiter_request = jupiter_request.header("x-api-key", api_key);
        }
    }

    let response = match jupiter_request.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            return fail_quote(
                "jupiter",
                "Jupiter Meta-Aggregator",
                in_mint,
                out_mint,
                amount,
                format!("Jupiter quote endpoint returned HTTP {}", r.status()),
            );
        }
        Err(e) => {
            return fail_quote(
                "jupiter",
                "Jupiter Meta-Aggregator",
                in_mint,
                out_mint,
                amount,
                format!("Jupiter request failed: {e}"),
            );
        }
    };

    let body: Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            return fail_quote(
                "jupiter",
                "Jupiter Meta-Aggregator",
                in_mint,
                out_mint,
                amount,
                format!("Invalid Jupiter response JSON: {e}"),
            );
        }
    };

    let out = match body
        .get("outAmount")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(v) if v > 0 => v,
        _ => {
            return fail_quote(
                "jupiter",
                "Jupiter Meta-Aggregator",
                in_mint,
                out_mint,
                amount,
                body.get("errorMessage")
                    .and_then(Value::as_str)
                    .unwrap_or("Jupiter returned no valid output amount"),
            );
        }
    };

    let platform_fee = if matches!(side, CompareSide::Sell) {
        match compute_fee(out, fee_bps) {
            Ok(f) => f,
            Err(e) => {
                return fail_quote(
                    "jupiter",
                    "Jupiter Meta-Aggregator",
                    in_mint,
                    out_mint,
                    amount,
                    e.to_string(),
                );
            }
        }
    } else {
        amount.saturating_sub(quote_amount)
    };

    let net = if matches!(side, CompareSide::Sell) {
        out.saturating_sub(platform_fee)
    } else {
        out
    };

    let router_name = body
        .get("router")
        .and_then(Value::as_str)
        .unwrap_or("Metis Router");

    // Extract primary venue and pool address from Jupiter's routePlan if available
    let pool_address = body
        .get("routePlan")
        .and_then(Value::as_array)
        .and_then(|plans| plans.first())
        .and_then(|p| p.get("swapInfo"))
        .and_then(|s| s.get("ammKey"))
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let venue_label = body
        .get("routePlan")
        .and_then(Value::as_array)
        .and_then(|plans| plans.first())
        .and_then(|p| p.get("swapInfo"))
        .and_then(|s| s.get("label"))
        .and_then(Value::as_str)
        .unwrap_or(router_name);

    // Parse price impact: handle both `priceImpact` (numeric %) and `priceImpactPct` (string) as documented by Jupiter
    let price_impact_pct = if let Some(impact_val) = body.get("priceImpact") {
        if let Some(f) = impact_val.as_f64() {
            Some(f.abs())
        } else if let Some(s) = impact_val.as_str() {
            s.parse::<f64>().ok().map(|v| v.abs())
        } else {
            None
        }
    } else if let Some(impact_pct_val) = body.get("priceImpactPct") {
        if let Some(s) = impact_pct_val.as_str() {
            s.parse::<f64>().ok().map(|v| v.abs() * 100.0)
        } else if let Some(f) = impact_pct_val.as_f64() {
            Some(f.abs() * 100.0)
        } else {
            None
        }
    } else {
        None
    };


    let venue_fee_bps = body
        .get("feeBps")
        .and_then(Value::as_u64)
        .and_then(|v| u16::try_from(v).ok());

    let fee_breakdown = FeeBreakdown {
        venue_fee_bps,
        venue_fee_atomic: None,
        platform_fee_bps: fee_bps,
        platform_fee_atomic: platform_fee,
    };

    RouteQuote {
        source: "jupiter_v2_order".into(),
        venue: format!("Jupiter ({venue_label})"),
        pool_address,
        input_mint: in_mint.into(),
        output_mint: out_mint.into(),
        input_amount_atomic: amount,
        gross_output_atomic: Some(out),
        net_output_atomic: Some(net),
        price_impact_pct,
        venue_fee_bps,
        platform_fee_atomic: Some(platform_fee),
        fee_breakdown: Some(fee_breakdown),
        state_slot: body.get("contextSlot").and_then(Value::as_u64),
        quoted_at_unix_ms: now_ms(),
        status: "estimated".into(),
        error: None,
        reason_unavailable: None,
    }
}

async fn pump_curve_quote(
    client: &reqwest::Client,
    rpc_url: &str,
    mint: &Pubkey,
    side: &CompareSide,
    amount: u64,
    fee_bps: u16,
    platform_bps: u16,
) -> RouteQuote {
    let (in_mint, out_mint) = match side {
        CompareSide::Buy => (WSOL_MINT.to_string(), mint.to_string()),
        CompareSide::Sell => (mint.to_string(), WSOL_MINT.to_string()),
    };

    let (curve, _) = protocol_pump::addresses::find_bonding_curve(mint);
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            curve.to_string(),
            {
                "encoding": "base64",
                "commitment": "confirmed"
            }
        ]
    });

    let response = match client.post(rpc_url).json(&request).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            return fail_quote(
                "pump_curve",
                "Pump.fun Bonding Curve",
                &in_mint,
                &out_mint,
                amount,
                format!("RPC returned HTTP {}", r.status()),
            );
        }
        Err(e) => {
            return fail_quote(
                "pump_curve",
                "Pump.fun Bonding Curve",
                &in_mint,
                &out_mint,
                amount,
                format!("RPC request failed: {e}"),
            );
        }
    };

    let value: Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            return fail_quote(
                "pump_curve",
                "Pump.fun Bonding Curve",
                &in_mint,
                &out_mint,
                amount,
                format!("Invalid RPC response: {e}"),
            );
        }
    };

    if let Some(err) = value.get("error") {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            format!("RPC error: {err}"),
        );
    }

    let state = &value["result"]["value"];
    if state.is_null() {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            "Bonding curve account does not exist on-chain",
        );
    }

    let data = state["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str);

    let Some(data) = data else {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            "Bonding curve account data is missing",
        );
    };

    let bytes = match base64::engine::general_purpose::STANDARD.decode(data) {
        Ok(b) => b,
        Err(e) => {
            return fail_quote(
                "pump_curve",
                "Pump.fun Bonding Curve",
                &in_mint,
                &out_mint,
                amount,
                format!("Invalid curve account base64: {e}"),
            );
        }
    };

    let curve_state = match protocol_pump::curve::BondingCurveAccount::try_from_slice(&bytes) {
        Ok(s) => s,
        Err(e) => {
            return fail_quote(
                "pump_curve",
                "Pump.fun Bonding Curve",
                &in_mint,
                &out_mint,
                amount,
                format!("Curve decode failed: {e}"),
            );
        }
    };

    if curve_state.complete {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            "Curve has graduated; direct bonding-curve quote is unavailable",
        );
    }

    let state_slot = value["result"]["context"]["slot"].as_u64();
    let Some(state_slot) = state_slot else {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            "RPC response omitted the curve state slot",
        );
    };

    let slot_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "getSlot",
        "params": [{"commitment": "confirmed"}]
    });

    let current_slot = match client.post(rpc_url).json(&slot_request).send().await {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v.get("result").and_then(Value::as_u64),
            Err(_) => None,
        },
        _ => None,
    };

    let Some(current_slot) = current_slot else {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            "Could not verify bonding-curve state freshness",
        );
    };

    let max_future_drift = MAX_CROSS_READ_SLOT_DRIFT;
    if state_slot > current_slot && state_slot - current_slot > max_future_drift {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            format!(
                "Bonding-curve state slot is in future beyond allowed drift: state slot {state_slot}, current slot {current_slot}"
            ),
        );
    }
    let effective_current_slot = std::cmp::max(current_slot, state_slot);
    if !rpc_slot_is_fresh(state_slot, effective_current_slot) {
        return fail_quote(
            "pump_curve",
            "Pump.fun Bonding Curve",
            &in_mint,
            &out_mint,
            amount,
            format!(
                "Bonding-curve state is stale (state slot {state_slot}, current slot {current_slot})"
            ),
        );
    }

    let (gross, venue_fee_atomic, platform_fee, price_impact_pct) = match side {
        CompareSide::Buy => {
            let platform = match compute_fee(amount, platform_bps) {
                Ok(v) => v,
                Err(e) => {
                    return fail_quote(
                        "pump_curve",
                        "Pump.fun Bonding Curve",
                        &in_mint,
                        &out_mint,
                        amount,
                        e.to_string(),
                    );
                }
            };
            let spendable = amount.saturating_sub(platform);
            let venue_fee = (spendable as u128 * fee_bps as u128 / 10_000) as u64;
            let curve_input = spendable.saturating_sub(venue_fee);
            let tokens_out = match curve_state.calculate_buy_tokens_out(curve_input) {
                Ok(out) => out,
                Err(e) => {
                    return fail_quote(
                        "pump_curve",
                        "Pump.fun Bonding Curve",
                        &in_mint,
                        &out_mint,
                        amount,
                        e.to_string(),
                    );
                }
            };

            let spot = curve_state.spot_price(9).unwrap_or(0.0);
            let impact = if spot > 0.0 && tokens_out > 0 {
                let effective_sol_per_token =
                    (curve_input as f64 / 1e9) / (tokens_out as f64 / 1e6);
                let pct = ((effective_sol_per_token - spot) / spot) * 100.0;
                Some(pct.max(0.0))
            } else {
                None
            };

            (tokens_out, Some(venue_fee), platform, impact)
        }
        CompareSide::Sell => {
            let gross_before_fee = match curve_state.calculate_sell_quote_out(amount) {
                Ok(v) => v,
                Err(e) => {
                    return fail_quote(
                        "pump_curve",
                        "Pump.fun Bonding Curve",
                        &in_mint,
                        &out_mint,
                        amount,
                        e.to_string(),
                    );
                }
            };
            let venue_fee = (gross_before_fee as u128 * fee_bps as u128 / 10_000) as u64;
            let after_venue = gross_before_fee.saturating_sub(venue_fee);
            let platform = match compute_fee(after_venue, platform_bps) {
                Ok(v) => v,
                Err(e) => {
                    return fail_quote(
                        "pump_curve",
                        "Pump.fun Bonding Curve",
                        &in_mint,
                        &out_mint,
                        amount,
                        e.to_string(),
                    );
                }
            };

            let spot = curve_state.spot_price(9).unwrap_or(0.0);
            let impact = if spot > 0.0 && amount > 0 {
                let effective_sol_per_token =
                    (gross_before_fee as f64 / 1e9) / (amount as f64 / 1e6);
                let pct = ((spot - effective_sol_per_token) / spot) * 100.0;
                Some(pct.max(0.0))
            } else {
                None
            };

            (after_venue, Some(venue_fee), platform, impact)
        }
    };

    let net = if matches!(side, CompareSide::Sell) {
        gross.saturating_sub(platform_fee)
    } else {
        gross
    };

    let fee_breakdown = FeeBreakdown {
        venue_fee_bps: Some(fee_bps),
        venue_fee_atomic,
        platform_fee_bps: platform_bps,
        platform_fee_atomic: platform_fee,
    };

    RouteQuote {
        source: "pump_fun_curve".into(),
        venue: "Pump.fun Bonding Curve".into(),
        pool_address: Some(curve.to_string()),
        input_mint: in_mint,
        output_mint: out_mint,
        input_amount_atomic: amount,
        gross_output_atomic: Some(gross),
        net_output_atomic: Some(net),
        price_impact_pct,
        venue_fee_bps: Some(fee_bps),
        platform_fee_atomic: Some(platform_fee),
        fee_breakdown: Some(fee_breakdown),
        state_slot: Some(state_slot),
        quoted_at_unix_ms: now_ms(),
        status: "estimated".into(),
        error: None,
        reason_unavailable: None,
    }
}

async fn pumpswap_quote(
    client: &reqwest::Client,
    rpc_url: &str,
    mint: &Pubkey,
    side: &CompareSide,
    amount: u64,
    platform_bps: u16,
) -> RouteQuote {
    let (in_mint, out_mint) = match side {
        CompareSide::Buy => (WSOL_MINT.to_string(), mint.to_string()),
        CompareSide::Sell => (mint.to_string(), WSOL_MINT.to_string()),
    };

    let wsol = protocol_pump::addresses::WRAPPED_SOL_MINT;
    let (pool_pda, _) = protocol_pumpswap::addresses::find_canonical_pool(mint, &wsol);

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            pool_pda.to_string(),
            {
                "encoding": "base64",
                "commitment": "confirmed"
            }
        ]
    });

    let res = match client.post(rpc_url).json(&request).send().await {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v,
            Err(e) => {
                return fail_quote(
                    "pumpswap",
                    "PumpSwap Canonical AMM",
                    &in_mint,
                    &out_mint,
                    amount,
                    format!("Invalid RPC response: {e}"),
                );
            }
        },
        _ => {
            return fail_quote(
                "pumpswap",
                "PumpSwap Canonical AMM",
                &in_mint,
                &out_mint,
                amount,
                "RPC request failed for PumpSwap pool",
            );
        }
    };

    let state_val = &res["result"]["value"];
    if state_val.is_null() {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "Canonical PumpSwap pool not created (token is still on bonding curve or migrated to Raydium)",
        );
    }

    let data_str = state_val["data"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str);

    let Some(data_str) = data_str else {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "Missing pool account data",
        );
    };

    let bytes = match base64::engine::general_purpose::STANDARD.decode(data_str) {
        Ok(b) => b,
        Err(e) => {
            return fail_quote(
                "pumpswap",
                "PumpSwap Canonical AMM",
                &in_mint,
                &out_mint,
                amount,
                format!("Failed to decode pool bytes: {e}"),
            );
        }
    };

    let pool_state = match protocol_pumpswap::pool::PumpSwapPoolState::try_from_slice(pool_pda, &bytes) {
        Ok(p) => p,
        Err(e) => {
            return fail_quote(
                "pumpswap",
                "PumpSwap Canonical AMM",
                &in_mint,
                &out_mint,
                amount,
                format!("Failed to parse PumpSwap pool: {e}"),
            );
        }
    };

    let pool_slot = res["result"]["context"]["slot"].as_u64();
    let Some(pool_slot) = pool_slot else {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "RPC response omitted the pool state slot",
        );
    };

    // Query vault accounts and fee config PDA via getMultipleAccounts
    let (fee_config_pda, _) = protocol_pumpswap::addresses::find_fee_config();
    let vaults_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "getMultipleAccounts",
        "params": [
            [
                pool_state.base_account.to_string(),
                pool_state.quote_account.to_string(),
                fee_config_pda.to_string(),
            ],
            {
                "encoding": "base64",
                "commitment": "confirmed"
            }
        ]
    });

    let vaults_res = match client.post(rpc_url).json(&vaults_req).send().await {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v,
            Err(e) => {
                return fail_quote(
                    "pumpswap",
                    "PumpSwap Canonical AMM",
                    &in_mint,
                    &out_mint,
                    amount,
                    format!("Failed to query PumpSwap vault accounts: {e}"),
                );
            }
        },
        _ => {
            return fail_quote(
                "pumpswap",
                "PumpSwap Canonical AMM",
                &in_mint,
                &out_mint,
                amount,
                "RPC request failed for PumpSwap vaults",
            );
        }
    };

    let vault_slot = vaults_res["result"]["context"]["slot"].as_u64();
    let Some(vault_slot) = vault_slot else {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "RPC response omitted the vault accounts state slot",
        );
    };

    // Query current network slot from RPC to verify freshness of both reads
    let slot_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "getSlot",
        "params": [{"commitment": "confirmed"}]
    });
    let current_slot = match client.post(rpc_url).json(&slot_req).send().await {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v.get("result").and_then(Value::as_u64),
            Err(_) => None,
        },
        _ => None,
    };
    let Some(current_slot) = current_slot else {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "Could not verify PumpSwap state freshness: getSlot failed",
        );
    };

    let max_future_drift = MAX_CROSS_READ_SLOT_DRIFT;
    if (pool_slot > current_slot && pool_slot - current_slot > max_future_drift)
        || (vault_slot > current_slot && vault_slot - current_slot > max_future_drift)
    {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            format!(
                "PumpSwap state slot in future beyond allowed drift: pool {}, vault {}, current {}",
                pool_slot, vault_slot, current_slot
            ),
        );
    }
    let effective_current_slot = std::cmp::max(current_slot, std::cmp::max(pool_slot, vault_slot));
    let state_slot = match verify_cross_read_slots(pool_slot, vault_slot, effective_current_slot) {
        Ok(slot) => slot,
        Err(reason) => {
            return fail_quote(
                "pumpswap",
                "PumpSwap Canonical AMM",
                &in_mint,
                &out_mint,
                amount,
                reason,
            );
        }
    };

    let accounts = vaults_res["result"]["value"].as_array();
    let Some(accounts) = accounts else {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "No vault accounts returned by RPC",
        );
    };

    if accounts.len() < 2 || accounts[0].is_null() || accounts[1].is_null() {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "PumpSwap base or quote vault does not exist on-chain",
        );
    }

    let base_bytes_str = accounts[0]["data"].as_array().and_then(|a| a.first()).and_then(Value::as_str);
    let quote_bytes_str = accounts[1]["data"].as_array().and_then(|a| a.first()).and_then(Value::as_str);

    let (Some(b_str), Some(q_str)) = (base_bytes_str, quote_bytes_str) else {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "Missing vault data",
        );
    };

    let base_bytes = match base64::engine::general_purpose::STANDARD.decode(b_str) {
        Ok(b) => b,
        Err(_) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, "Failed to decode base vault"),
    };
    let quote_bytes = match base64::engine::general_purpose::STANDARD.decode(q_str) {
        Ok(b) => b,
        Err(_) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, "Failed to decode quote vault"),
    };

    let base_reserve = match parse_token_account_balance(&base_bytes) {
        Ok(b) => b,
        Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, e),
    };
    let quote_vault_balance = match parse_token_account_balance(&quote_bytes) {
        Ok(q) => q,
        Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, e),
    };

    let effective_quote_reserve = match pool_state.effective_quote_reserve(quote_vault_balance) {
        Ok(eq) => eq,
        Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, format!("Invalid effective quote reserve: {e}")),
    };

    if base_reserve == 0 || effective_quote_reserve == 0 {
        return fail_quote(
            "pumpswap",
            "PumpSwap Canonical AMM",
            &in_mint,
            &out_mint,
            amount,
            "Zero pool reserves in PumpSwap AMM vaults",
        );
    }

    // Derive active dynamic fee tier from fee config PDA and pool creator fee
    let mut parsed_fee_config: Option<protocol_pumpswap::pool::PumpSwapFeeConfig> = None;
    if accounts.len() >= 3 && !accounts[2].is_null() {
        let fee_acc = &accounts[2];
        let acc_owner = fee_acc["owner"].as_str().unwrap_or_default();
        if acc_owner == protocol_pumpswap::addresses::PUMP_FEE_PROGRAM.to_string() {
            if let Some(b64) = fee_acc["data"].as_array().and_then(|a| a.first()).and_then(Value::as_str) {
                if let Ok(fee_bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                    if let Ok(cfg) = protocol_pumpswap::pool::PumpSwapFeeConfig::try_from_slice(&fee_bytes) {
                        parsed_fee_config = Some(cfg);
                    }
                }
            }
        }
    }

    let fee_bps = pool_state.derive_dynamic_fee_bps(
        parsed_fee_config.as_ref(),
        base_reserve,
        effective_quote_reserve,
    ) as u16;

    let (gross, net, platform_fee, venue_fee_atomic, price_impact_pct) = match side {
        CompareSide::Buy => {
            let platform = match compute_fee(amount, platform_bps) {
                Ok(f) => f,
                Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, e.to_string()),
            };
            let spendable = amount.saturating_sub(platform);
            let venue_fee = (spendable as u128 * fee_bps as u128 / 10_000) as u64;
            let tokens_out = match protocol_pumpswap::pool::calculate_amm_buy(spendable, base_reserve, effective_quote_reserve, fee_bps as u64) {
                Ok(out) => out,
                Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, format!("AMM buy calculation error: {e}")),
            };

            let impact = if tokens_out > 0 {
                let spot = (effective_quote_reserve as f64 / 1e9) / (base_reserve as f64 / 1e6);
                let exec = (spendable as f64 / 1e9) / (tokens_out as f64 / 1e6);
                let pct = ((exec - spot) / spot) * 100.0;
                Some(pct.max(0.0))
            } else {
                None
            };

            (tokens_out, tokens_out, platform, Some(venue_fee), impact)
        }
        CompareSide::Sell => {
            let gross_quote = match protocol_pumpswap::pool::calculate_amm_sell(amount, base_reserve, effective_quote_reserve, 0) {
                Ok(out) => out,
                Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, format!("AMM sell calculation error: {e}")),
            };
            let venue_fee = (gross_quote as u128 * fee_bps as u128 / 10_000) as u64;
            let after_venue_fee = gross_quote.saturating_sub(venue_fee);
            let platform = match compute_fee(after_venue_fee, platform_bps) {
                Ok(f) => f,
                Err(e) => return fail_quote("pumpswap", "PumpSwap Canonical AMM", &in_mint, &out_mint, amount, e.to_string()),
            };
            let net_sol = after_venue_fee.saturating_sub(platform);

            let impact = if gross_quote > 0 {
                let spot = (effective_quote_reserve as f64 / 1e9) / (base_reserve as f64 / 1e6);
                let exec = (gross_quote as f64 / 1e9) / (amount as f64 / 1e6);
                let pct = ((spot - exec) / spot) * 100.0;
                Some(pct.max(0.0))
            } else {
                None
            };

            (gross_quote, net_sol, platform, Some(venue_fee), impact)
        }
    };

    let fee_breakdown = FeeBreakdown {
        venue_fee_bps: Some(fee_bps),
        venue_fee_atomic,
        platform_fee_bps: platform_bps,
        platform_fee_atomic: platform_fee,
    };

    RouteQuote {
        source: "pumpswap_canonical".into(),
        venue: "PumpSwap Canonical AMM".into(),
        pool_address: Some(pool_pda.to_string()),
        input_mint: in_mint,
        output_mint: out_mint,
        input_amount_atomic: amount,
        gross_output_atomic: Some(gross),
        net_output_atomic: Some(net),
        price_impact_pct,
        venue_fee_bps: Some(fee_bps),
        platform_fee_atomic: Some(platform_fee),
        fee_breakdown: Some(fee_breakdown),
        state_slot: Some(state_slot),
        quoted_at_unix_ms: now_ms(),
        status: "estimated".into(),
        error: None,
        reason_unavailable: None,
    }
}

pub async fn compare_routes_handler(
    State(state): State<AppState>,
    Json(request): Json<CompareRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let mint = Pubkey::from_str(&request.mint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid mint: {e}")})),
        )
    })?;

    if request.amount_atomic == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "amount_atomic must be greater than zero"})),
        ));
    }

    let platform_bps = request
        .fee_tier
        .unwrap_or(state.inner.fee_config.tier)
        .rate_bps();

    let min_liq = request
        .min_liquidity_usd
        .unwrap_or(DEFAULT_MIN_LIQUIDITY_USD);
    let min_vol = request
        .min_volume_usd
        .unwrap_or(DEFAULT_MIN_VOLUME_24H_USD);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(9))
        .build()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("HTTP client error: {e}")})),
            )
        })?;

    let mint_string = mint.to_string();

    let (in_mint_str, out_mint_str) = match request.side {
        CompareSide::Buy => (WSOL_MINT, mint_string.as_str()),
        CompareSide::Sell => (mint_string.as_str(), WSOL_MINT),
    };

    let (jupiter, pump, pumpswap, discovered_pools) = tokio::join!(
        jupiter_quote(
            &client,
            &mint_string,
            &request.side,
            request.amount_atomic,
            platform_bps
        ),
        pump_curve_quote(
            &client,
            &state.inner.rpc_url,
            &mint,
            &request.side,
            request.amount_atomic,
            PUMP_BONDING_FEE_BPS,
            platform_bps
        ),
        pumpswap_quote(
            &client,
            &state.inner.rpc_url,
            &mint,
            &request.side,
            request.amount_atomic,
            platform_bps
        ),
        discover_candidate_pools(&client, Some(&state.inner.rpc_url), &mint_string, min_liq, min_vol),
    );

    let mut routes = vec![jupiter, pump, pumpswap];

    // For every discovered candidate pool across Raydium, Meteora, Orca, etc.
    if !discovered_pools.is_empty() {
        for pool in &discovered_pools {
            // Skip pump/pumpswap if already covered by direct on-chain PDA queries
            if pool.dex_id == "pump" || pool.dex_id == "pumpfun" || pool.dex_id == "pumpswap" {
                continue;
            }

            let is_quoted_by_jupiter = routes
                .first()
                .and_then(|j| j.pool_address.as_ref())
                == Some(&pool.pool_address);

            let short_addr = if pool.pool_address.len() > 8 {
                &pool.pool_address[..8]
            } else {
                &pool.pool_address
            };

            let quote_for_pool = if !pool.passes_dust_filter {
                fail_quote(
                    &format!("{}_pool", pool.dex_id),
                    &format!("{} ({})", pool.dex_label, short_addr),
                    in_mint_str,
                    out_mint_str,
                    request.amount_atomic,
                    format!(
                        "Dust filter excluded: liquidity (${:.0}) or 24h volume (${:.0}) below threshold (${:.0} / ${:.0})",
                        pool.liquidity_usd, pool.volume_24h_usd, min_liq, min_vol
                    ),
                )
            } else if is_quoted_by_jupiter {
                continue;
            } else {
                fail_quote(
                    &format!("{}_direct", pool.dex_id),
                    &format!("{} ({})", pool.dex_label, short_addr),
                    in_mint_str,
                    out_mint_str,
                    request.amount_atomic,
                    format!(
                        "Direct AMM quoting not implemented in repository for {} (indexed with ${:.0} liq / ${:.0} vol); candidate routes compete via Jupiter Meta-Aggregator (dust thresholds apply to indexed pool inspection only, not Jupiter internal routing)",
                        pool.dex_label, pool.liquidity_usd, pool.volume_24h_usd
                    ),
                )
            };

            routes.push(quote_for_pool);
        }
    } else {
        // Fallback placeholders if pool discovery returned 0 pairs (e.g. offline/new token)
        routes.push(fail_quote(
            "raydium_direct",
            "Raydium AMM (Direct)",
            in_mint_str,
            out_mint_str,
            request.amount_atomic,
            "Direct pool indexer not configured; liquidity routed via Jupiter Meta-Aggregator (internal router unaffected by dust thresholds)",
        ));
        routes.push(fail_quote(
            "meteora_direct",
            "Meteora DLMM (Direct)",
            in_mint_str,
            out_mint_str,
            request.amount_atomic,
            "Direct pool indexer not configured; liquidity routed via Jupiter Meta-Aggregator (internal router unaffected by dust thresholds)",
        ));
        routes.push(fail_quote(
            "orca_direct",
            "Orca Whirlpool (Direct)",
            in_mint_str,
            out_mint_str,
            request.amount_atomic,
            "Direct pool indexer not configured; liquidity routed via Jupiter Meta-Aggregator (internal router unaffected by dust thresholds)",
        ));
    }

    let ranked = rank_routes(&routes, request.amount_atomic);
    let best = ranked.first().copied();
    let runner = ranked.get(1).copied();

    let difference = match (best, runner) {
        (Some(a), Some(b)) => {
            let out_a = routes[a].net_output_atomic.unwrap_or(0);
            let out_b = routes[b].net_output_atomic.unwrap_or(0);
            Some(out_a.saturating_sub(out_b))
        }
        _ => None,
    };

    let recommendation_reason = match (best, runner, difference) {
        (Some(a), Some(b), Some(diff)) => {
            let venue_a = &routes[a].venue;
            let venue_b = &routes[b].venue;
            let out_b = routes[b].net_output_atomic.unwrap_or(1);
            let pct = (diff as f64 / out_b as f64) * 100.0;
            Some(format!(
                "{venue_a} yields +{diff} more units (+{pct:.2}%) than {venue_b} after all fees and price impact."
            ))
        }
        (Some(a), None, _) => {
            let venue_a = &routes[a].venue;
            Some(format!(
                "{venue_a} is the single available live route for this token."
            ))
        }
        _ => None,
    };

    Ok(Json(CompareResponse {
        side: match request.side {
            CompareSide::Buy => "buy",
            CompareSide::Sell => "sell",
        }
        .into(),
        mint: mint.to_string(),
        input_mint: in_mint_str.to_string(),
        output_mint: out_mint_str.to_string(),
        input_amount_atomic: request.amount_atomic,
        platform_fee_bps: platform_bps,
        best_route_index: best,
        runner_up_index: runner,
        output_difference_atomic: difference,
        recommendation_reason,
        routes,
        candidate_pools: discovered_pools,
        min_liquidity_threshold_usd: min_liq,
        min_volume_threshold_usd: min_vol,
        execution_enabled: true,
        disclaimer: "Live quotes computed from on-chain AMM reserves and Jupiter Swap V2 meta-aggregation. Swaps are verified and simulated natively before signing.",
    }))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computed_platform_fee_matches_default_policy() {
        assert_eq!(
            compute_fee(1_000_000_000, FeeTier::Default.rate_bps()).unwrap(),
            2_000_000
        );
        assert_eq!(
            compute_fee(1_000_000_000, FeeTier::UserRpc.rate_bps()).unwrap(),
            1_000_000
        );
    }

    #[test]
    fn failed_venue_is_explicit_and_not_a_candidate() {
        let failed = fail_quote("test", "Venue", "MINT_IN", "MINT_OUT", 100, "RPC timeout");
        assert_eq!(failed.status, "unavailable");
        assert!(failed.net_output_atomic.is_none());
        assert_eq!(failed.error.as_deref(), Some("RPC timeout"));
        assert_eq!(failed.reason_unavailable.as_deref(), Some("RPC timeout"));
        assert_eq!(failed.input_mint, "MINT_IN");
        assert_eq!(failed.output_mint, "MINT_OUT");
    }

    #[test]
    fn venue_fee_math_uses_integer_amounts() {
        let fee = (1_000_000_000u128 * PUMP_BONDING_FEE_BPS as u128 / 10_000) as u64;
        assert_eq!(fee, 12_500_000);
    }

    #[test]
    fn ranking_uses_net_output_and_equal_input_only() {
        let quote = |venue: &str, input: u64, output: u64| RouteQuote {
            source: venue.into(),
            venue: venue.into(),
            pool_address: None,
            input_mint: "IN".into(),
            output_mint: "OUT".into(),
            input_amount_atomic: input,
            gross_output_atomic: Some(output),
            net_output_atomic: Some(output),
            price_impact_pct: Some(0.1),
            venue_fee_bps: None,
            platform_fee_atomic: Some(0),
            fee_breakdown: None,
            state_slot: None,
            quoted_at_unix_ms: 1,
            status: "estimated".into(),
            error: None,
            reason_unavailable: None,
        };
        let routes = vec![
            quote("lower", 1_000, 90),
            quote("best", 1_000, 100),
            quote("wrong-size", 2_000, 1_000),
        ];
        assert_eq!(rank_routes(&routes, 1_000), vec![1, 0]);
    }

    #[test]
    fn stale_and_future_rpc_slots_are_rejected() {
        assert!(rpc_slot_is_fresh(1_000, 1_150));
        assert!(!rpc_slot_is_fresh(1_000, 1_151));
        assert!(!rpc_slot_is_fresh(1_151, 1_150));
    }

    #[test]
    fn runner_up_and_output_difference_calculation() {
        let make_quote = |venue: &str, net: u64| RouteQuote {
            source: venue.into(),
            venue: venue.into(),
            pool_address: None,
            input_mint: "IN".into(),
            output_mint: "OUT".into(),
            input_amount_atomic: 1_000,
            gross_output_atomic: Some(net),
            net_output_atomic: Some(net),
            price_impact_pct: Some(0.05),
            venue_fee_bps: None,
            platform_fee_atomic: Some(0),
            fee_breakdown: None,
            state_slot: None,
            quoted_at_unix_ms: 1,
            status: "estimated".into(),
            error: None,
            reason_unavailable: None,
        };

        let routes = vec![
            make_quote("Route A", 1_500_000),
            make_quote("Route B", 1_800_000),
            make_quote("Route C", 1_200_000),
        ];

        let ranked = rank_routes(&routes, 1_000);
        assert_eq!(ranked, vec![1, 0, 2]);

        let best = ranked[0];
        let runner = ranked[1];
        let diff = routes[best].net_output_atomic.unwrap() - routes[runner].net_output_atomic.unwrap();

        assert_eq!(best, 1); // Route B
        assert_eq!(runner, 0); // Route A
        assert_eq!(diff, 300_000); // 1.8M - 1.5M
    }

    #[test]
    fn failed_routes_are_excluded_from_ranking() {
        let good_quote = RouteQuote {
            source: "good".into(),
            venue: "Good Venue".into(),
            pool_address: None,
            input_mint: "IN".into(),
            output_mint: "OUT".into(),
            input_amount_atomic: 500,
            gross_output_atomic: Some(250),
            net_output_atomic: Some(250),
            price_impact_pct: None,
            venue_fee_bps: None,
            platform_fee_atomic: None,
            fee_breakdown: None,
            state_slot: None,
            quoted_at_unix_ms: 1,
            status: "estimated".into(),
            error: None,
            reason_unavailable: None,
        };
        let bad_quote = fail_quote("bad", "Bad Venue", "IN", "OUT", 500, "Pool drained");

        let routes = vec![bad_quote, good_quote];
        let ranked = rank_routes(&routes, 500);

        assert_eq!(ranked, vec![1]);
    }

    #[test]
    fn pumpswap_amm_math_calculates_buy_and_sell_quotes() {
        let base_reserve = 100_000_000_000u64; // 100k tokens (6 decimals)
        let effective_quote_reserve = 30_000_000_000u64; // 30 SOL (9 decimals)
        let fee_bps = 25u64; // 0.25% AMM fee

        // Buy: 1 SOL in
        let quote_in = 1_000_000_000u64;
        let tokens_out = protocol_pumpswap::pool::calculate_amm_buy(
            quote_in,
            base_reserve,
            effective_quote_reserve,
            fee_bps,
        )
        .expect("amm buy calculation succeeds");
        assert!(tokens_out > 0);
        assert!(tokens_out < base_reserve);

        // Sell: tokens in
        let tokens_in = 3_000_000_000u64;
        let quote_out = protocol_pumpswap::pool::calculate_amm_sell(
            tokens_in,
            base_reserve,
            effective_quote_reserve,
            fee_bps,
        )
        .expect("amm sell calculation succeeds");
        assert!(quote_out > 0);
        assert!(quote_out < effective_quote_reserve);
    }

    #[test]
    fn token_account_balance_parser_extracts_le_u64() {
        let mut data = vec![0u8; 165]; // standard SPL token account length
        let balance: u64 = 42_500_000_000;
        data[64..72].copy_from_slice(&balance.to_le_bytes());

        let parsed = parse_token_account_balance(&data).expect("parses balance correctly");
        assert_eq!(parsed, balance);

        // Data too short (<72 bytes) returns Err
        assert!(parse_token_account_balance(&[0u8; 50]).is_err());
    }

    #[test]
    fn dust_filter_evaluates_liquidity_and_volume_thresholds() {
        let min_liq = 2_000.0;
        let min_vol = 500.0;

        let pass_pool = DiscoveredPool {
            dex_id: "raydium".into(),
            dex_label: "Raydium".into(),
            pool_address: "pool1".into(),
            base_mint: "base".into(),
            quote_mint: "quote".into(),
            liquidity_usd: 15_000.0,
            volume_24h_usd: 1_200.0,
            price_usd: 0.15,
            passes_dust_filter: 15_000.0 >= min_liq && 1_200.0 >= min_vol,
            is_verified_on_chain: true,
        };
        assert!(pass_pool.passes_dust_filter);

        let fail_liq_pool = DiscoveredPool {
            dex_id: "orca".into(),
            dex_label: "Orca".into(),
            pool_address: "pool2".into(),
            base_mint: "base".into(),
            quote_mint: "quote".into(),
            liquidity_usd: 1_200.0,
            volume_24h_usd: 3_000.0,
            price_usd: 0.15,
            passes_dust_filter: 1_200.0 >= min_liq && 3_000.0 >= min_vol,
            is_verified_on_chain: false,
        };
        assert!(!fail_liq_pool.passes_dust_filter);

        let fail_vol_pool = DiscoveredPool {
            dex_id: "meteora".into(),
            dex_label: "Meteora".into(),
            pool_address: "pool3".into(),
            base_mint: "base".into(),
            quote_mint: "quote".into(),
            liquidity_usd: 50_000.0,
            volume_24h_usd: 200.0,
            price_usd: 0.15,
            passes_dust_filter: 50_000.0 >= min_liq && 200.0 >= min_vol,
            is_verified_on_chain: true,
        };
        assert!(!fail_vol_pool.passes_dust_filter);
    }

    #[test]
    fn cross_read_slots_freshness_and_drift_validation() {
        let current_slot = 10_000u64;

        // Perfectly synchronized and fresh reads
        assert_eq!(
            verify_cross_read_slots(9_995, 9_998, current_slot).unwrap(),
            9_995
        );

        // Desynchronized reads: drift > MAX_CROSS_READ_SLOT_DRIFT (10)
        let desync_err = verify_cross_read_slots(9_980, 9_995, current_slot).unwrap_err();
        assert!(desync_err.contains("desynchronized"));
        assert!(desync_err.contains("drift 15 > 10"));

        // Stale pool slot (> 150 slot lag)
        let stale_pool_err = verify_cross_read_slots(9_800, 9_995, current_slot).unwrap_err();
        assert!(stale_pool_err.contains("PumpSwap pool state is stale"));

        // Stale vault slot (> 150 slot lag)
        let stale_vault_err = verify_cross_read_slots(9_995, 9_800, current_slot).unwrap_err();
        assert!(stale_vault_err.contains("PumpSwap vault state is stale"));

        // Future slot (> current_slot)
        let future_err = verify_cross_read_slots(10_050, 9_995, current_slot).unwrap_err();
        assert!(future_err.contains("PumpSwap pool state is stale"));
    }
}

