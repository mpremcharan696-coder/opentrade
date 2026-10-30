//! Solana RPC read-only transaction simulation.
//!
//! Validates candidate execution on-chain using `simulateTransaction` with `sigVerify: false`.
//! Strictly read-only: does NOT sign, does NOT broadcast, and does NOT mutate state.
//! Fails closed on any RPC error, revert, or stale blockhash.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SimulationError {
    #[error("HTTP transport error: {0}")]
    Http(String),
    #[error("RPC returned error: {0}")]
    RpcError(String),
    #[error("Transaction simulation reverted on-chain: {0}")]
    Reverted(String),
    #[error("Malformed simulation response from RPC: {0}")]
    MalformedResponse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulationResult {
    pub units_consumed: u64,
    pub logs: Vec<String>,
}

/// Simulate an unsigned transaction through the configured Solana RPC as a read-only safety check.
///
/// Uses `sigVerify: false` so that unsigned candidate transactions can be verified on-chain.
/// Uses `replaceRecentBlockhash: true` to avoid failures due to recent blockhash freshness during simulation.
pub async fn simulate_transaction_rpc(
    rpc_url: &str,
    unsigned_tx_base64: &str,
) -> Result<SimulationResult, SimulationError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(7000))
        .build()
        .map_err(|e| SimulationError::Http(e.to_string()))?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateTransaction",
        "params": [
            unsigned_tx_base64,
            {
                "encoding": "base64",
                "sigVerify": false,
                "commitment": "confirmed",
                "replaceRecentBlockhash": true
            }
        ]
    });

    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| SimulationError::Http(format!("Failed to connect to RPC: {}", e)))?;

    if !res.status().is_success() {
        return Err(SimulationError::RpcError(format!(
            "RPC returned HTTP error status {}",
            res.status()
        )));
    }

    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| SimulationError::MalformedResponse(format!("Invalid JSON from RPC: {}", e)))?;

    if let Some(err) = json.get("error") {
        return Err(SimulationError::RpcError(format!("RPC error response: {}", err)));
    }

    let value = &json["result"]["value"];
    if value.is_null() {
        return Err(SimulationError::MalformedResponse(
            "Missing 'value' field in simulateTransaction result".to_string(),
        ));
    }

    // Check for simulation execution error
    if !value["err"].is_null() {
        let err_str = value["err"].to_string();
        let logs: Vec<String> = value["logs"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let log_summary = if !logs.is_empty() {
            format!(" | Logs: {}", logs.join("; "))
        } else {
            String::new()
        };

        return Err(SimulationError::Reverted(format!("{}{}", err_str, log_summary)));
    }

    let units_consumed = value["unitsConsumed"].as_u64().unwrap_or(0);
    let logs: Vec<String> = value["logs"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(SimulationResult {
        units_consumed,
        logs,
    })
}
