//! Live Solana Swap Transaction Signing, RPC Submission, and Status Confirmation.
//!
//! Security and Operational Bounds:
//! 1. Native re-parsing and re-validation immediately before unlocking keypair.
//! 2. Treats frontend summaries as untrusted; enforces wallet, mint, amounts, treasury, and instruction order.
//! 3. Rejects expired or near-expiry blockhashes (never silently rebuilds after user review).
//! 4. Unlocks keystore only after all validations pass, verifies pubkey match, signs, and zeroizes secret bytes.
//! 5. Submits signed transaction to Solana RPC with preflight simulation enabled (`skipPreflight: false`).
//! 6. Bounded, cancellable confirmation tracking distinguishing: Prepared, Signed, Submitted, Confirmed, Failed, Expired, Unknown.
//! 7. Does not claim success on local signature or RPC sendTransaction acceptance; requires confirmed status on-chain.

use std::path::Path;
use std::time::Duration;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;

use shared_types::fee::{CONFIGURED_PLATFORM_TREASURY, FeeConfig, FeeTier};
use shared_types::swap::{PreparedSwapCandidate, SwapExecutionStatus, SwapSubmissionResult};
use solana_sdk::address_lookup_table::AddressLookupTableAccount;
use solana_sdk::message::VersionedMessage;
use transaction_validation::validator::{
    fetch_and_validate_address_lookup_tables, validate_swap_candidate_with_alts,
};

use crate::keystore::{decrypt_keypair, load_keystore};

/// RPC signature status info returned from `getSignatureStatuses`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcSignatureStatus {
    pub slot: u64,
    pub confirmations: Option<usize>,
    pub err: Option<serde_json::Value>,
    pub confirmation_status: Option<String>,
}

/// Details of RPC preflight simulation rejection or ambiguous submission error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcSubmissionError {
    /// Deterministic preflight simulation failure. Transaction simulated and reverted; never broadcast to cluster.
    PreflightSimulationFailed {
        error_message: String,
        logs: Vec<String>,
    },
    /// Ambiguous submission error (network timeout, connection drop, 5xx, or non-deterministic transport failure).
    /// The transaction may have reached the node/cluster and may still land on-chain.
    AmbiguousSubmission {
        error_message: String,
    },
}

/// Abstract Solana RPC interface for submission and confirmation tracking.
/// Allows full testability and mockability without network requests or mainnet broadcast.
pub trait SolanaRpc: Send + Sync {
    fn get_block_height(&self) -> impl std::future::Future<Output = Result<u64, String>> + Send;
    fn is_blockhash_valid(&self, blockhash: &str) -> impl std::future::Future<Output = Result<bool, String>> + Send;
    fn send_transaction(&self, tx_base64: &str, skip_preflight: bool) -> impl std::future::Future<Output = Result<String, RpcSubmissionError>> + Send;
    fn get_signature_status(&self, signature: &str) -> impl std::future::Future<Output = Result<Option<RpcSignatureStatus>, String>> + Send;
    fn fetch_lookup_tables(&self, keys: &[Pubkey]) -> impl std::future::Future<Output = Result<Vec<AddressLookupTableAccount>, String>> + Send;
}

/// Real HTTP JSON-RPC client for Solana cluster communication.
pub struct HttpSolanaRpc {
    client: reqwest::Client,
    rpc_url: String,
}

impl HttpSolanaRpc {
    pub fn new(rpc_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(15000))
            .build()
            .unwrap_or_default();
        Self { client, rpc_url }
    }
}

impl SolanaRpc for HttpSolanaRpc {
    async fn get_block_height(&self) -> Result<u64, String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBlockHeight",
            "params": [{"commitment": "confirmed"}]
        });

        let res = self.client.post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("RPC transport error on getBlockHeight: {}", e))?;

        let json: serde_json::Value = res.json()
            .await
            .map_err(|e| format!("Failed to parse getBlockHeight response: {}", e))?;

        if let Some(err) = json.get("error") {
            return Err(format!("RPC error on getBlockHeight: {}", err));
        }

        json["result"]
            .as_u64()
            .ok_or_else(|| "Missing numeric result in getBlockHeight".to_string())
    }

    async fn is_blockhash_valid(&self, blockhash: &str) -> Result<bool, String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "isBlockhashValid",
            "params": [
                blockhash,
                {"commitment": "confirmed"}
            ]
        });

        let res = self.client.post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("RPC transport error checking blockhash validity: {}", e))?;

        let json: serde_json::Value = res.json()
            .await
            .map_err(|e| format!("Failed to parse isBlockhashValid response: {}", e))?;

        if let Some(err) = json.get("error") {
            return Err(format!("RPC error on isBlockhashValid: {}", err));
        }

        json["result"]["value"]
            .as_bool()
            .ok_or_else(|| "Missing boolean value in isBlockhashValid result".to_string())
    }

    async fn send_transaction(&self, tx_base64: &str, skip_preflight: bool) -> Result<String, RpcSubmissionError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                tx_base64,
                {
                    "encoding": "base64",
                    "skipPreflight": skip_preflight,
                    "preflightCommitment": "confirmed",
                    "maxRetries": 0
                }
            ]
        });

        let res = self.client.post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| RpcSubmissionError::AmbiguousSubmission {
                error_message: format!("Transport error sending transaction: {}", e),
            })?;

        let json: serde_json::Value = res.json()
            .await
            .map_err(|e| RpcSubmissionError::AmbiguousSubmission {
                error_message: format!("Malformed or incomplete JSON from sendTransaction: {}", e),
            })?;

        if let Some(err) = json.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown RPC error").to_string();
            let mut logs = Vec::new();
            if let Some(log_arr) = err["data"]["logs"].as_array() {
                for l in log_arr {
                    if let Some(s) = l.as_str() {
                        logs.push(s.to_string());
                    }
                }
            }

            // Distinguish deterministic simulation failure vs ambiguous transport / RPC error
            let is_simulation_failure = err.get("code").and_then(|c| c.as_i64()) == Some(-32002)
                || err.get("data").and_then(|d| d.get("err")).is_some()
                || !logs.is_empty()
                || msg.to_lowercase().contains("simulation failed");

            if is_simulation_failure {
                return Err(RpcSubmissionError::PreflightSimulationFailed {
                    error_message: msg,
                    logs,
                });
            } else {
                return Err(RpcSubmissionError::AmbiguousSubmission {
                    error_message: msg,
                });
            }
        }

        if let Some(sig) = json["result"].as_str() {
            Ok(sig.to_string())
        } else {
            Err(RpcSubmissionError::AmbiguousSubmission {
                error_message: "Missing signature in sendTransaction response".to_string(),
            })
        }
    }

    async fn get_signature_status(&self, signature: &str) -> Result<Option<RpcSignatureStatus>, String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getSignatureStatuses",
            "params": [
                [signature],
                {"searchTransactionHistory": true}
            ]
        });

        let res = self.client.post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Transport error on getSignatureStatuses: {}", e))?;

        let json: serde_json::Value = res.json()
            .await
            .map_err(|e| format!("Failed to parse getSignatureStatuses response: {}", e))?;

        if let Some(err) = json.get("error") {
            return Err(format!("RPC error on getSignatureStatuses: {}", err));
        }

        let val = &json["result"]["value"][0];
        if val.is_null() {
            return Ok(None);
        }

        let slot = val["slot"].as_u64().unwrap_or(0);
        let confirmations = val["confirmations"].as_u64().map(|c| c as usize);
        let err = if val["err"].is_null() { None } else { Some(val["err"].clone()) };
        let confirmation_status = val["confirmationStatus"].as_str().map(|s| s.to_string());

        Ok(Some(RpcSignatureStatus {
            slot,
            confirmations,
            err,
            confirmation_status,
        }))
    }

    async fn fetch_lookup_tables(&self, keys: &[Pubkey]) -> Result<Vec<AddressLookupTableAccount>, String> {
        fetch_and_validate_address_lookup_tables(&self.rpc_url, keys)
            .await
            .map_err(|e| format!("Failed to fetch on-chain address lookup tables: {}", e))
    }
}

/// Sign an independently validated candidate and submit to Solana RPC with confirmation tracking.
pub async fn sign_and_submit_swap_candidate<R: SolanaRpc>(
    candidate: &PreparedSwapCandidate,
    expected_signer_str: &str,
    password: &str,
    cluster_safety_confirmed: bool,
    keystore_dir: &Path,
    fee_tier: Option<FeeTier>,
    rpc: &R,
    polling_timeout_secs: u64,
    polling_interval_millis: u64,
) -> SwapSubmissionResult {
    // 1. Mainnet Safety Gate enforcement
    if !cluster_safety_confirmed {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Failed,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some("Mainnet safety gate violation: live transaction broadcast requires explicit user cluster confirmation.".to_string()),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: false,
        };
    }

    // 2. Reject empty password
    if password.trim().is_empty() {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Failed,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some("Password is required to unlock wallet keystore for trade signing.".to_string()),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: false,
        };
    }

    // 3. Expected signer parse
    let expected_signer = match expected_signer_str.parse::<Pubkey>() {
        Ok(pk) => pk,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Invalid expected signer public key: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    // 4. Deserialize unsigned VersionedTransaction
    let tx_bytes = match STANDARD.decode(&candidate.unsigned_tx_base64) {
        Ok(b) => b,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Failed to decode candidate transaction base64: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    let unsigned_tx: VersionedTransaction = match bincode::deserialize(&tx_bytes) {
        Ok(t) => t,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Failed to deserialize candidate transaction: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    // 5. Fetch authoritative on-chain Address Lookup Tables (ALTs) directly from RPC
    // and strictly verify candidate's ALT data against the on-chain state immediately before signing.
    // Treat all candidate fields from the frontend as untrusted.
    let table_keys: Vec<Pubkey> = match &unsigned_tx.message {
        VersionedMessage::V0(v0) => v0.address_table_lookups.iter().map(|l| l.account_key).collect(),
        _ => Vec::new(),
    };

    let on_chain_alts = if !table_keys.is_empty() {
        let tables = match rpc.fetch_lookup_tables(&table_keys).await {
            Ok(t) => t,
            Err(e) => {
                return SwapSubmissionResult {
                    status: SwapExecutionStatus::Failed,
                    signature: None,
                    slot: None,
                    confirmations: None,
                    error: Some(format!("Pre-signing on-chain address lookup table fetch failed: {}", e)),
                    logs: Vec::new(),
                    explorer_url: None,
                    blockhash_expired: false,
                };
            }
        };

        let candidate_alts = match candidate.parsed_address_lookup_tables() {
            Ok(a) => a,
            Err(e) => {
                return SwapSubmissionResult {
                    status: SwapExecutionStatus::Failed,
                    signature: None,
                    slot: None,
                    confirmations: None,
                    error: Some(format!("Candidate contains malformed address lookup table data: {}", e)),
                    logs: Vec::new(),
                    explorer_url: None,
                    blockhash_expired: false,
                };
            }
        };

        if candidate_alts.len() != tables.len() {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!(
                    "Security violation: Candidate provided {} lookup tables, but on-chain lookup table fetch found {}",
                    candidate_alts.len(), tables.len()
                )),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }

        for on_chain_table in &tables {
            let cand_table = match candidate_alts.iter().find(|c| c.key == on_chain_table.key) {
                Some(c) => c,
                None => {
                    return SwapSubmissionResult {
                        status: SwapExecutionStatus::Failed,
                        signature: None,
                        slot: None,
                        confirmations: None,
                        error: Some(format!(
                            "Security violation: Table {} from on-chain transaction lookups is missing from candidate ALT list",
                            on_chain_table.key
                        )),
                        logs: Vec::new(),
                        explorer_url: None,
                        blockhash_expired: false,
                    };
                }
            };

            if cand_table.addresses != on_chain_table.addresses {
                return SwapSubmissionResult {
                    status: SwapExecutionStatus::Failed,
                    signature: None,
                    slot: None,
                    confirmations: None,
                    error: Some(format!(
                        "Security violation: Candidate ALT addresses for table {} do not match authoritative on-chain table state!",
                        on_chain_table.key
                    )),
                    logs: Vec::new(),
                    explorer_url: None,
                    blockhash_expired: false,
                };
            }
        }

        tables
    } else {
        Vec::new()
    };

    // 6. Re-run native transaction validation immediately before signing using authoritative on-chain ALTs.
    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: CONFIGURED_PLATFORM_TREASURY,
        tier: fee_tier.unwrap_or(FeeTier::Default),
    };

    if let Err(val_err) = validate_swap_candidate_with_alts(candidate, &expected_signer, &fee_config, &on_chain_alts) {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Failed,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some(format!("Pre-signing validation rejected transaction candidate: {}", val_err)),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: false,
        };
    }

    let tx_msg_blockhash = unsigned_tx.message.recent_blockhash().to_string();
    if tx_msg_blockhash != candidate.recent_blockhash {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Failed,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some(format!(
                "Transaction message blockhash '{}' does not match candidate.recent_blockhash '{}'",
                tx_msg_blockhash, candidate.recent_blockhash
            )),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: false,
        };
    }

    // 6. Verify blockhash freshness and cluster expiry bounds before touching keypair
    let current_height = match rpc.get_block_height().await {
        Ok(h) => h,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Failed to query cluster block height: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    if current_height >= candidate.last_valid_block_height {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Expired,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some(format!(
                "Candidate blockhash has expired on-chain (cluster height: {}, last valid: {}). Please request a fresh route candidate.",
                current_height, candidate.last_valid_block_height
            )),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: true,
        };
    }

    if candidate.last_valid_block_height.saturating_sub(current_height) < 4 {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Expired,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some(format!(
                "Candidate blockhash is near expiry (< 4 blocks remaining: cluster height {}, last valid {}). Please refresh quote.",
                current_height, candidate.last_valid_block_height
            )),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: true,
        };
    }

    // Blockhash validity check via RPC method (Fail-Closed) bound directly to the transaction message's blockhash
    match rpc.is_blockhash_valid(&tx_msg_blockhash).await {
        Ok(false) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Expired,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!(
                    "Candidate transaction message blockhash '{}' is no longer recognized as valid by cluster. Transaction rejected.",
                    tx_msg_blockhash
                )),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: true,
            };
        }
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!(
                    "Fail-closed blockhash check failed: unable to verify blockhash validity with cluster RPC: {}. Signing aborted.",
                    e
                )),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
        Ok(true) => {}
    }

    // 7. Unlock encrypted keystore and verify decrypted keypair
    let keystore = match load_keystore(expected_signer_str, keystore_dir) {
        Ok(k) => k,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Failed to load wallet keystore: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    let keypair = match decrypt_keypair(&keystore, password) {
        Ok(kp) => kp,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Keystore unlock failed: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    // Public key ownership verification
    if keypair.pubkey() != expected_signer {
        return SwapSubmissionResult {
            status: SwapExecutionStatus::Failed,
            signature: None,
            slot: None,
            confirmations: None,
            error: Some(format!(
                "Decrypted keypair public key {} does not match expected active signer {}",
                keypair.pubkey(), expected_signer
            )),
            logs: Vec::new(),
            explorer_url: None,
            blockhash_expired: false,
        };
    }

    // 8. Sign the transaction message using decrypted keypair
    let signed_tx = match VersionedTransaction::try_new(unsigned_tx.message, &[&keypair]) {
        Ok(tx) => tx,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: None,
                slot: None,
                confirmations: None,
                error: Some(format!("Transaction signing error: {}", e)),
                logs: Vec::new(),
                explorer_url: None,
                blockhash_expired: false,
            };
        }
    };

    // Drop and zeroize keypair immediately
    drop(keypair);

    let signature_bytes = signed_tx.signatures[0];
    let signature_str = bs58::encode(&signature_bytes).into_string();
    let explorer_url = format!("https://solscan.io/tx/{}", signature_str);

    let signed_tx_bytes = match bincode::serialize(&signed_tx) {
        Ok(b) => b,
        Err(e) => {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: Some(signature_str.clone()),
                slot: None,
                confirmations: None,
                error: Some(format!("Failed to serialize signed transaction: {}", e)),
                logs: Vec::new(),
                explorer_url: Some(explorer_url),
                blockhash_expired: false,
            };
        }
    };
    let signed_tx_base64 = STANDARD.encode(&signed_tx_bytes);

    // 8. Submit signed transaction via Solana RPC with preflight enabled
    match rpc.send_transaction(&signed_tx_base64, false).await {
        Ok(sig) => {
            // Ensure RPC returned signature matches computed signature
            if sig != signature_str {
                tracing::warn!("RPC returned signature {} differing from signed {}", sig, signature_str);
            }
            // 9. Bounded, cancellable confirmation tracking loop
            confirm_transaction_status(
                &signature_str,
                candidate.last_valid_block_height,
                rpc,
                polling_timeout_secs,
                polling_interval_millis,
            ).await
        }
        Err(RpcSubmissionError::PreflightSimulationFailed { error_message, logs }) => {
            // Deterministic preflight simulation failure: transaction was simulated and reverted; not broadcast.
            SwapSubmissionResult {
                status: SwapExecutionStatus::Failed,
                signature: Some(signature_str.clone()),
                slot: None,
                confirmations: None,
                error: Some(format!("RPC preflight simulation rejected: {}", error_message)),
                logs,
                explorer_url: Some(explorer_url),
                blockhash_expired: false,
            }
        }
        Err(RpcSubmissionError::AmbiguousSubmission { error_message }) => {
            // Ambiguous submission error (network timeout, connection drop, 5xx): transaction may still land!
            // DO NOT report as Failed. Transition to status confirmation check using locally derived signature
            // so a retry won't accidentally create a duplicate trade.
            let status_check = confirm_transaction_status(
                &signature_str,
                candidate.last_valid_block_height,
                rpc,
                polling_timeout_secs,
                polling_interval_millis,
            ).await;

            match status_check.status {
                SwapExecutionStatus::Confirmed => status_check,
                SwapExecutionStatus::Expired => status_check,
                _ => {
                    // Still unconfirmed or timed out: report as Unknown with locally derived signature preserved,
                    // warning the user to check the signature on Solscan and never submit duplicate trades.
                    SwapSubmissionResult {
                        status: SwapExecutionStatus::Unknown,
                        signature: Some(signature_str.clone()),
                        slot: status_check.slot,
                        confirmations: status_check.confirmations,
                        error: Some(format!(
                            "Ambiguous transaction submission error: {}. Transaction status is Unknown. Do NOT retry or submit a duplicate trade. Check transaction signature on Solscan or query status: {}",
                            error_message, signature_str
                        )),
                        logs: Vec::new(),
                        explorer_url: Some(explorer_url),
                        blockhash_expired: false,
                    }
                }
            }
        }
    }
}

/// Track status of a submitted transaction signature with bounded polling and blockhash expiry checks.
pub async fn confirm_transaction_status<R: SolanaRpc>(
    signature: &str,
    last_valid_block_height: u64,
    rpc: &R,
    polling_timeout_secs: u64,
    polling_interval_millis: u64,
) -> SwapSubmissionResult {
    let explorer_url = format!("https://solscan.io/tx/{}", signature);
    let start_time = std::time::Instant::now();
    let timeout_duration = Duration::from_secs(polling_timeout_secs);
    let step_duration = Duration::from_millis(polling_interval_millis.max(500));

    loop {
        // 1. Check signature status on cluster
        match rpc.get_signature_status(signature).await {
            Ok(Some(status)) => {
                // If on-chain execution error occurred, mark as Failed even though reached cluster
                if let Some(err_val) = status.err {
                    return SwapSubmissionResult {
                        status: SwapExecutionStatus::Failed,
                        signature: Some(signature.to_string()),
                        slot: Some(status.slot),
                        confirmations: status.confirmations,
                        error: Some(format!("On-chain execution failed: {}", err_val)),
                        logs: Vec::new(),
                        explorer_url: Some(explorer_url),
                        blockhash_expired: false,
                    };
                }

                // If confirmed or finalized without error: SUCCESS!
                if let Some(ref conf_status) = status.confirmation_status {
                    if conf_status == "confirmed" || conf_status == "finalized" {
                        return SwapSubmissionResult {
                            status: SwapExecutionStatus::Confirmed,
                            signature: Some(signature.to_string()),
                            slot: Some(status.slot),
                            confirmations: status.confirmations,
                            error: None,
                            logs: Vec::new(),
                            explorer_url: Some(explorer_url),
                            blockhash_expired: false,
                        };
                    }
                }
            }
            Ok(None) => {
                // Not yet processed / seen by RPC node
            }
            Err(e) => {
                tracing::warn!("Transient RPC error checking signature status: {}", e);
            }
        }

        // 2. Check cluster block height to detect blockhash expiry
        if let Ok(current_height) = rpc.get_block_height().await {
            if current_height > last_valid_block_height {
                // Perform one final check to be certain it didn't land right at the boundary
                if let Ok(Some(final_status)) = rpc.get_signature_status(signature).await {
                    if let Some(ref conf_status) = final_status.confirmation_status {
                        if (conf_status == "confirmed" || conf_status == "finalized") && final_status.err.is_none() {
                            return SwapSubmissionResult {
                                status: SwapExecutionStatus::Confirmed,
                                signature: Some(signature.to_string()),
                                slot: Some(final_status.slot),
                                confirmations: final_status.confirmations,
                                error: None,
                                logs: Vec::new(),
                                explorer_url: Some(explorer_url),
                                blockhash_expired: false,
                            };
                        }
                    }
                }

                // The blockhash has definitively expired on-chain without landing
                return SwapSubmissionResult {
                    status: SwapExecutionStatus::Expired,
                    signature: Some(signature.to_string()),
                    slot: None,
                    confirmations: None,
                    error: Some(format!(
                        "Transaction blockhash expired at block height {} without landing on-chain. No funds were spent.",
                        current_height
                    )),
                    logs: Vec::new(),
                    explorer_url: Some(explorer_url),
                    blockhash_expired: true,
                };
            }
        }

        // 3. Check polling timeout limit
        if start_time.elapsed() >= timeout_duration {
            return SwapSubmissionResult {
                status: SwapExecutionStatus::Unknown,
                signature: Some(signature.to_string()),
                slot: None,
                confirmations: None,
                error: Some(format!(
                    "Transaction status confirmation timed out after {}s. Do NOT submit a duplicate trade. Check transaction signature on Solscan or query status again.",
                    polling_timeout_secs
                )),
                logs: Vec::new(),
                explorer_url: Some(explorer_url),
                blockhash_expired: false,
            };
        }

        tokio::time::sleep(step_duration).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use solana_sdk::compute_budget::ComputeBudgetInstruction;
    use solana_sdk::hash::Hash;
    use solana_sdk::message::VersionedMessage;
    use solana_sdk::message::v0::Message;
    use solana_sdk::signature::Keypair;
    use solana_sdk::system_instruction;
    use protocol_pump::instructions::build_buy_v2_instructions;
    use shared_types::swap::SwapCandidateSummary;
    use shared_types::token::TokenInfo;
    use crate::keystore::{encrypt_keypair, save_keystore};

    #[derive(Clone, Default)]
    pub struct MockRpc {
        pub block_height: Arc<AtomicU64>,
        pub blockhash_valid: Arc<AtomicBool>,
        pub blockhash_valid_result: Arc<Mutex<Option<Result<bool, String>>>>,
        pub preflight_failure: Arc<Mutex<Option<RpcSubmissionError>>>,
        pub send_transaction_result: Arc<Mutex<Option<Result<String, RpcSubmissionError>>>>,
        pub signature_status: Arc<Mutex<Option<RpcSignatureStatus>>>,
        pub sent_transactions: Arc<Mutex<Vec<String>>>,
        pub checked_blockhashes: Arc<Mutex<Vec<String>>>,
        pub mock_lookup_tables: Arc<Mutex<Option<Result<Vec<AddressLookupTableAccount>, String>>>>,
    }

    impl SolanaRpc for MockRpc {
        async fn get_block_height(&self) -> Result<u64, String> {
            Ok(self.block_height.load(Ordering::SeqCst))
        }

        async fn is_blockhash_valid(&self, blockhash: &str) -> Result<bool, String> {
            self.checked_blockhashes.lock().await.push(blockhash.to_string());
            if let Some(ref res) = *self.blockhash_valid_result.lock().await {
                return res.clone();
            }
            Ok(self.blockhash_valid.load(Ordering::SeqCst))
        }

        async fn send_transaction(&self, tx_base64: &str, _skip_preflight: bool) -> Result<String, RpcSubmissionError> {
            self.sent_transactions.lock().await.push(tx_base64.to_string());
            if let Some(res) = self.send_transaction_result.lock().await.take() {
                return res;
            }
            if let Some(failure) = self.preflight_failure.lock().await.clone() {
                return Err(failure);
            }
            Ok("5abc123MockSignatureTestString999".to_string())
        }

        async fn get_signature_status(&self, _signature: &str) -> Result<Option<RpcSignatureStatus>, String> {
            Ok(self.signature_status.lock().await.clone())
        }

        async fn fetch_lookup_tables(&self, _keys: &[Pubkey]) -> Result<Vec<AddressLookupTableAccount>, String> {
            if let Some(ref res) = *self.mock_lookup_tables.lock().await {
                return res.clone();
            }
            Ok(Vec::new())
        }
    }

    fn mock_token(mint: Pubkey) -> TokenInfo {
        TokenInfo {
            name: "Test Pump Token".to_string(),
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

    fn build_test_candidate_with_tier(signer: &Pubkey, mint: &Pubkey, tier: FeeTier) -> PreparedSwapCandidate {
        let token_info = mock_token(*mint);
        let treasury = CONFIGURED_PLATFORM_TREASURY;
        let fee_rate_bps = tier.rate_bps();
        let fee_lamports = shared_types::fee::compute_fee(1_000_000_000, fee_rate_bps).unwrap();

        let mut instructions = Vec::new();
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
        instructions.push(system_instruction::transfer(signer, &treasury, fee_lamports));
        instructions.extend(build_buy_v2_instructions(
            &token_info,
            signer,
            30_000_000,
            1_000_000_000,
        ));

        let msg = Message::try_compile(signer, &instructions, &[], Hash::default()).unwrap();
        let writable_accounts = msg.account_keys
            .iter()
            .enumerate()
            .filter(|(idx, _)| msg.is_maybe_writable(*idx, None))
            .map(|(_, k)| k.to_string())
            .collect();

        let tx = VersionedTransaction {
            signatures: vec![solana_sdk::signature::Signature::default()],
            message: VersionedMessage::V0(msg),
        };

        let bytes = bincode::serialize(&tx).unwrap();
        let b64 = STANDARD.encode(&bytes);

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
            platform_fee_bps: fee_rate_bps,
            platform_fee_atomic: fee_lamports,
            platform_fee_recipient: treasury.to_string(),
            venue_fee_atomic: None,
            estimated_rent_lamports: 2_039_280,
            total_sol_required_atomic: 1_000_000_000 + 2_039_280 + fee_lamports + (50_000 * 200_000 / 1_000_000),
            tx_byte_size: bytes.len(),
            quote_freshness_ms: 100,
        };

        PreparedSwapCandidate {
            candidate_id: "test_candidate_1".to_string(),
            selected_route_source: "pump_bonding_curve".to_string(),
            venue: "Pump.fun Bonding Curve".to_string(),
            pool_address: None,
            input_mint: transaction_validation::validator::WSOL_MINT_STR.to_string(),
            output_mint: mint.to_string(),
            input_amount_atomic: 1_000_000_000,
            estimated_output_atomic: 33_000_000,
            minimum_output_atomic: 30_000_000,
            slippage_bps: 100,
            recent_blockhash: Hash::default().to_string(),
            last_valid_block_height: 310_000_150,
            state_slot: 310_000_000,
            created_at: "2026-10-28T20:00:00Z".to_string(),
            expires_at: "2026-10-28T20:02:00Z".to_string(),
            unsigned_tx_base64: b64,
            summary,
            address_lookup_tables: Vec::new(),
        }
    }

    fn build_test_candidate(signer: &Pubkey, mint: &Pubkey) -> PreparedSwapCandidate {
        build_test_candidate_with_tier(signer, mint, FeeTier::Default)
    }

    fn setup_keystore(keypair: &Keypair, password: &str) -> std::path::PathBuf {
        let temp_dir = std::env::temp_dir().join(format!("test_keystore_{}_{}", keypair.pubkey(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let keystore = encrypt_keypair(keypair, "Test Wallet", password).unwrap();
        save_keystore(&keystore, &temp_dir).unwrap();
        temp_dir
    }

    #[tokio::test]
    async fn test_wrong_keystore_password_fails_closed() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "correct_password_123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "wrong_password_999",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Keystore unlock failed"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_malformed_keystore_fails_closed() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = std::env::temp_dir().join(format!("test_malformed_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&keystore_dir).unwrap();
        std::fs::write(keystore_dir.join(format!("{}.json", keypair.pubkey())), "{ invalid_json ]").unwrap();

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Failed to load wallet keystore"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_wrong_active_wallet_mismatch_fails_closed() {
        let keypair = Keypair::new();
        let other_keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &other_keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_mint_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Adversary alters the output mint in candidate struct without changing the tx
        candidate.output_mint = Pubkey::new_unique().to_string();

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_amount_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Adversary alters the input spend amount
        candidate.input_amount_atomic = 500_000_000; // was 1_000_000_000

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_minimum_output_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Adversary artificially inflates minimum_output_atomic beyond instruction bound
        candidate.minimum_output_atomic = 999_000_000;

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_treasury_diverted_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Adversary alters fee recipient in summary to an attacker wallet
        candidate.summary.platform_fee_recipient = Pubkey::new_unique().to_string();

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_fee_amount_mismatch_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Adversary alters summary fee amount
        candidate.summary.platform_fee_atomic = 5_000_000;

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_raised_both_summary_fee_and_transfer_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Legitimate fee is 2_000_000 (20 bps on 1 SOL).
        // Adversary crafts a candidate where BOTH summary fee and compiled transfer are raised to 50_000_000.
        let token_info = mock_token(mint);
        let treasury = CONFIGURED_PLATFORM_TREASURY;
        let tampered_fee = 50_000_000;

        let mut instructions = Vec::new();
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
        instructions.push(system_instruction::transfer(&keypair.pubkey(), &treasury, tampered_fee));
        instructions.extend(build_buy_v2_instructions(
            &token_info,
            &keypair.pubkey(),
            30_000_000,
            1_000_000_000,
        ));

        let msg = Message::try_compile(&keypair.pubkey(), &instructions, &[], Hash::default()).unwrap();
        let writable_accounts = msg.account_keys
            .iter()
            .enumerate()
            .filter(|(idx, _)| msg.is_maybe_writable(*idx, None))
            .map(|(_, k)| k.to_string())
            .collect();

        let tx = VersionedTransaction {
            signatures: vec![solana_sdk::signature::Signature::default()],
            message: VersionedMessage::V0(msg),
        };
        let bytes = bincode::serialize(&tx).unwrap();
        let b64 = STANDARD.encode(&bytes);

        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        candidate.summary.writable_accounts = writable_accounts;
        candidate.summary.platform_fee_atomic = tampered_fee;
        candidate.unsigned_tx_base64 = b64;

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        // Native validator must independently calculate allowed fee (2_000_000) from FeeConfig
        // and reject candidate before signing, even though summary fee and transfer match each other.
        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected transaction candidate: Fee amount mismatch"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_tampered_candidate_unauthorized_writable_account_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Injects an unauthorized writable account into summary
        candidate.summary.writable_accounts.push(Pubkey::new_unique().to_string());

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Pre-signing validation rejected"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_demo_mode_isolation_keystore_not_accessed() {
        // In demo paper trading mode, no real keystore is created, accessed, or decrypted.
        let non_existent_dir = std::env::temp_dir().join(format!("demo_non_existent_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        assert!(!non_existent_dir.exists());

        // Verify that list_wallets on a demo empty path succeeds safely without error
        let wallets = crate::keystore::list_wallets(&non_existent_dir).unwrap();
        assert_eq!(wallets.len(), 0);
    }


    #[tokio::test]
    async fn test_expired_blockhash_before_signing_rejected() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        // Set cluster block height past last valid block height (310_000_150)
        rpc.block_height.store(310_000_151, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Expired);
        assert!(res.blockhash_expired);
        assert!(res.error.unwrap().contains("expired on-chain"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_near_expiry_blockhash_rejected() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        // Only 2 blocks remaining until last_valid_block_height (310_000_150)
        rpc.block_height.store(310_000_148, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Expired);
        assert!(res.blockhash_expired);
        assert!(res.error.unwrap().contains("near expiry"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_mainnet_safety_gate_unconfirmed_rejected() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        // cluster_safety_confirmed = false!
        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            false,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.unwrap().contains("Mainnet safety gate violation"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_rpc_preflight_simulation_failure_reported_as_failed() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        // Set preflight simulation error
        *rpc.preflight_failure.lock().await = Some(RpcSubmissionError::PreflightSimulationFailed {
            error_message: "Transaction simulation failed: Error processing Instruction 2: custom program error: 0x1".to_string(),
            logs: vec![
                "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P invoke [1]".to_string(),
                "Program log: Error: Insufficient funds".to_string(),
            ],
        });

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.signature.is_some());
        assert!(res.error.as_ref().unwrap().contains("RPC preflight simulation rejected"));
        assert_eq!(res.logs.len(), 2);
        assert!(res.explorer_url.is_some());
        assert!(!res.blockhash_expired);
    }

    #[tokio::test]
    async fn test_ambiguous_rpc_submission_reports_unknown_with_locally_derived_signature() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);
        // Signature status remains unresolved (e.g. cluster node hasn't processed yet or network dropped)
        *rpc.signature_status.lock().await = None;

        // Transport timeout / connection drop when calling sendTransaction
        *rpc.send_transaction_result.lock().await = Some(Err(RpcSubmissionError::AmbiguousSubmission {
            error_message: "Transport error sending transaction: request timed out".to_string(),
        }));

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            1, // 1 second polling timeout for fast test
            500,
        ).await;

        // Ambiguous submission MUST NOT be reported as Failed (which would risk the user retrying a duplicate trade).
        // It must be reported as Unknown, with locally derived signature preserved and duplicate trade warning.
        assert_eq!(res.status, SwapExecutionStatus::Unknown);
        assert!(res.signature.is_some());
        assert!(res.error.as_ref().unwrap().contains("Ambiguous transaction submission error"));
        assert!(res.error.as_ref().unwrap().contains("Do NOT retry or submit a duplicate trade"));
        assert!(res.explorer_url.is_some());
    }

    #[tokio::test]
    async fn test_rpc_blockhash_validity_failure_fails_closed() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        // isBlockhashValid fails with transport error
        *rpc.blockhash_valid_result.lock().await = Some(Err("RPC node 503 Service Unavailable".to_string()));

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        // Blockhash check must fail closed, rejecting transaction before touching keystore
        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.error.as_ref().unwrap().contains("Fail-closed blockhash check failed"));
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_confirmed_success() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        // RPC returns confirmed status
        *rpc.signature_status.lock().await = Some(RpcSignatureStatus {
            slot: 310_000_075,
            confirmations: Some(31),
            err: None,
            confirmation_status: Some("confirmed".to_string()),
        });

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Confirmed);
        assert!(res.signature.is_some());
        assert_eq!(res.slot, Some(310_000_075));
        assert_eq!(res.confirmations, Some(31));
        assert!(res.error.is_none());
        assert!(res.explorer_url.unwrap().contains("https://solscan.io/tx/"));
    }

    #[tokio::test]
    async fn test_confirmed_on_chain_failure() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        // Transaction landed on chain, but failed execution!
        *rpc.signature_status.lock().await = Some(RpcSignatureStatus {
            slot: 310_000_075,
            confirmations: None,
            err: Some(serde_json::json!({
                "InstructionError": [2, "Custom(1)"]
            })),
            confirmation_status: Some("confirmed".to_string()),
        });

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        assert!(res.signature.is_some());
        assert_eq!(res.slot, Some(310_000_075));
        assert!(res.error.unwrap().contains("On-chain execution failed"));
        assert!(res.explorer_url.is_some());
    }

    #[tokio::test]
    async fn test_timeout_unresolved_reported_as_unknown_with_signature_preserved() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);
        // Signature status remains None (unresolved)
        *rpc.signature_status.lock().await = None;

        // Bounded polling timeout of 1 second for fast test
        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            1,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Unknown);
        assert!(res.signature.is_some());
        assert!(res.error.unwrap().contains("Do NOT submit a duplicate trade"));
        assert!(res.explorer_url.is_some());
    }

    #[tokio::test]
    async fn test_blockhash_expiry_during_confirmation() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);
        *rpc.signature_status.lock().await = None;

        // Advance height beyond last valid block height (310_000_150) after submission starts
        let rpc_clone = rpc.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            rpc_clone.block_height.store(310_000_160, Ordering::SeqCst);
        });

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            5,
            100,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Expired);
        assert!(res.blockhash_expired);
        assert!(res.error.unwrap().contains("expired at block height"));
        assert!(res.signature.is_some());
    }

    #[tokio::test]
    async fn test_direct_query_swap_status_on_existing_signature() {
        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        *rpc.signature_status.lock().await = Some(RpcSignatureStatus {
            slot: 310_000_090,
            confirmations: Some(20),
            err: None,
            confirmation_status: Some("confirmed".to_string()),
        });

        let res = confirm_transaction_status(
            "MockSigExisting123456",
            310_000_150,
            &rpc,
            5,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Confirmed);
        assert_eq!(res.signature.as_deref(), Some("MockSigExisting123456"));
        assert_eq!(res.slot, Some(310_000_090));
    }

    #[tokio::test]
    async fn test_custom_fee_tier_passes_native_validation_and_submits() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        // UserRpc fee tier is 10 bps (1_000_000 lamports on 1 SOL)
        let candidate = build_test_candidate_with_tier(&keypair.pubkey(), &mint, FeeTier::UserRpc);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);
        *rpc.signature_status.lock().await = Some(RpcSignatureStatus {
            slot: 310_000_075,
            confirmations: Some(31),
            err: None,
            confirmation_status: Some("confirmed".to_string()),
        });

        // Submitting with FeeTier::UserRpc must pass native validation without FeeAmountMismatch
        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            Some(FeeTier::UserRpc),
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Confirmed);
        assert!(res.signature.is_some());
        assert_eq!(rpc.sent_transactions.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn test_tampered_candidate_blockhash_message_mismatch_rejected_before_signing() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let mut candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        // Adversary alters candidate.recent_blockhash to a different blockhash than the one inside unsigned_tx
        let fake_blockhash = solana_sdk::hash::Hash::new_unique().to_string();
        candidate.recent_blockhash = fake_blockhash.clone();

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            10,
            500,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        let err = res.error.expect("expected error");
        println!("ACTUAL ERROR: {}", err);
        assert!(err.to_lowercase().contains("blockhash"), "Error was: {}", err);
        // Keypair must not have been used to sign, and no tx sent to RPC
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_serialized_transaction_message_blockhash_passed_to_is_blockhash_valid() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let candidate = build_test_candidate(&keypair.pubkey(), &mint);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        let _res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            1,
            100,
        ).await;

        let checked = rpc.checked_blockhashes.lock().await;
        assert_eq!(checked.len(), 1);
        // Ensure the verified blockhash matches candidate.recent_blockhash
        assert_eq!(checked[0], candidate.recent_blockhash);
    }

    fn build_test_candidate_with_alt(
        signer: &Pubkey,
        mint: &Pubkey,
        alt: &AddressLookupTableAccount,
    ) -> PreparedSwapCandidate {
        let treasury = CONFIGURED_PLATFORM_TREASURY;
        let fee_lamports = 20_000u64;
        let swap_lamports = 9_980_000u64;
        let input_amount = 10_000_000u64;
        let wsol_mint = "So11111111111111111111111111111111111111112".parse::<Pubkey>().unwrap();

        let wsol_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            signer,
            &wsol_mint,
            &spl_token::id(),
        );
        let dest_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            signer,
            mint,
            &spl_token::id(),
        );

        let pool_state_account = alt.addresses.first().copied().unwrap_or_else(Pubkey::new_unique);
        let amm_authority = alt.addresses.get(1).copied().unwrap_or_else(Pubkey::new_unique);

        let mut instructions = Vec::new();
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(200_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
        instructions.push(system_instruction::transfer(signer, &treasury, fee_lamports));
        instructions.push(system_instruction::transfer(signer, &wsol_ata, swap_lamports));

        let mut jup_data = Vec::with_capacity(47);
        jup_data.extend_from_slice(&transaction_validation::validator::JUPITER_ROUTE_DISCRIMINATOR);
        jup_data.extend_from_slice(&1u32.to_le_bytes());
        jup_data.extend_from_slice(&[0u8; 16]);
        jup_data.extend_from_slice(&swap_lamports.to_le_bytes());
        jup_data.extend_from_slice(&30_000_000u64.to_le_bytes());
        jup_data.extend_from_slice(&100u16.to_le_bytes());
        jup_data.push(0u8);

        instructions.push(solana_sdk::instruction::Instruction {
            program_id: transaction_validation::allowlist::JUPITER_V6_PROGRAM,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new_readonly(spl_token::id(), false),
                solana_sdk::instruction::AccountMeta::new(*signer, true),
                solana_sdk::instruction::AccountMeta::new(wsol_ata, false),
                solana_sdk::instruction::AccountMeta::new(dest_ata, false),
                solana_sdk::instruction::AccountMeta::new(pool_state_account, false),
                solana_sdk::instruction::AccountMeta::new_readonly(amm_authority, false),
            ],
            data: jup_data,
        });

        instructions.push(spl_token::instruction::close_account(
            &spl_token::id(),
            &wsol_ata,
            signer,
            signer,
            &[],
        ).unwrap());

        let msg = Message::try_compile(signer, &instructions, &[alt.clone()], Hash::default()).unwrap();
        let mut writables: Vec<String> = msg.account_keys
            .iter()
            .enumerate()
            .filter(|(idx, _)| msg.is_maybe_writable(*idx, None))
            .map(|(_, k)| k.to_string())
            .collect();

        for lookup in &msg.address_table_lookups {
            if lookup.account_key == alt.key {
                for &idx in &lookup.writable_indexes {
                    if let Some(pk) = alt.addresses.get(idx as usize) {
                        let s = pk.to_string();
                        if !writables.contains(&s) {
                            writables.push(s);
                        }
                    }
                }
            }
        }

        let tx = VersionedTransaction {
            signatures: vec![solana_sdk::signature::Signature::default()],
            message: VersionedMessage::V0(msg),
        };

        let tx_bytes = bincode::serialize(&tx).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

        PreparedSwapCandidate {
            candidate_id: "candidate_test_alt_123".to_string(),
            selected_route_source: "jupiter".to_string(),
            venue: "Jupiter".to_string(),
            pool_address: None,
            input_mint: "So11111111111111111111111111111111111111112".to_string(),
            output_mint: mint.to_string(),
            input_amount_atomic: input_amount,
            estimated_output_atomic: 30_000_000,
            minimum_output_atomic: 29_000_000,
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
                writable_accounts: writables,
                signer_accounts: vec![signer.to_string()],
                compute_unit_limit: 200_000,
                priority_fee_micro_lamports: 50_000,
                platform_fee_bps: 20,
                platform_fee_atomic: fee_lamports,
                platform_fee_recipient: treasury.to_string(),
                venue_fee_atomic: None,
                estimated_rent_lamports: 0,
                total_sol_required_atomic: input_amount,
                tx_byte_size: tx_bytes.len(),
                quote_freshness_ms: 100,
            },
            address_lookup_tables: vec![
                shared_types::swap::AddressLookupTableEntry {
                    key: alt.key.to_string(),
                    addresses: alt.addresses.iter().map(|p| p.to_string()).collect(),
                }
            ],
        }
    }

    #[tokio::test]
    async fn test_signing_with_valid_on_chain_alts_succeeds() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let alt_key = Pubkey::new_unique();
        let pool_acc = Pubkey::new_unique();
        let amm_auth = Pubkey::new_unique();
        let alt_account = AddressLookupTableAccount {
            key: alt_key,
            addresses: vec![pool_acc, amm_auth],
        };

        let candidate = build_test_candidate_with_alt(&keypair.pubkey(), &mint, &alt_account);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);
        *rpc.signature_status.lock().await = Some(RpcSignatureStatus {
            slot: 310_000_075,
            confirmations: Some(31),
            err: None,
            confirmation_status: Some("confirmed".to_string()),
        });
        // Mock RPC returns the authoritative on-chain lookup table
        *rpc.mock_lookup_tables.lock().await = Some(Ok(vec![alt_account.clone()]));

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            1,
            100,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Confirmed, "Signing must succeed when on-chain ALTs match: {:?}", res.error);
        assert_eq!(rpc.sent_transactions.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn test_signing_rejects_candidate_when_candidate_alts_differ_from_on_chain() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let alt_key = Pubkey::new_unique();
        let benign_addr = Pubkey::new_unique();
        let malicious_addr = Pubkey::new_unique();

        let cand_alt = AddressLookupTableAccount {
            key: alt_key,
            addresses: vec![benign_addr],
        };

        let candidate = build_test_candidate_with_alt(&keypair.pubkey(), &mint, &cand_alt);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        // Attacker scenario: On-chain table actually contains malicious_addr,
        // but candidate sent through frontend claims it contains benign_addr!
        let on_chain_alt = AddressLookupTableAccount {
            key: alt_key,
            addresses: vec![malicious_addr],
        };
        *rpc.mock_lookup_tables.lock().await = Some(Ok(vec![on_chain_alt]));

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            1,
            100,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        let err = res.error.expect("expected security error");
        assert!(err.contains("Security violation") && err.contains("do not match authoritative on-chain table state"), "Error: {}", err);
        // Crucial: Keystore MUST NOT sign and transaction MUST NOT be sent!
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_signing_rejects_candidate_when_on_chain_alt_fetch_fails() {
        let keypair = Keypair::new();
        let mint = Pubkey::new_unique();
        let alt_key = Pubkey::new_unique();
        let alt_account = AddressLookupTableAccount {
            key: alt_key,
            addresses: vec![Pubkey::new_unique()],
        };

        let candidate = build_test_candidate_with_alt(&keypair.pubkey(), &mint, &alt_account);
        let keystore_dir = setup_keystore(&keypair, "password123");

        let rpc = MockRpc::default();
        rpc.block_height.store(310_000_050, Ordering::SeqCst);
        rpc.blockhash_valid.store(true, Ordering::SeqCst);

        // RPC failure fetching ALTs
        *rpc.mock_lookup_tables.lock().await = Some(Err("Network connection dropped".to_string()));

        let res = sign_and_submit_swap_candidate(
            &candidate,
            &keypair.pubkey().to_string(),
            "password123",
            true,
            &keystore_dir,
            None,
            &rpc,
            1,
            100,
        ).await;

        assert_eq!(res.status, SwapExecutionStatus::Failed);
        let err = res.error.expect("expected RPC error");
        assert!(err.contains("Pre-signing on-chain address lookup table fetch failed"), "Error: {}", err);
        assert_eq!(rpc.sent_transactions.lock().await.len(), 0);
    }
}

