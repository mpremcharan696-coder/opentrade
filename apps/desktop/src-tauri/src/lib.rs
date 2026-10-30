//! Tauri 2 desktop application commands and security boundary.

pub mod keystore;
pub mod submission;


use keystore::{
    decrypt_keypair, encrypt_keypair, list_wallets, load_keystore, save_keystore, WalletSummary,
};
use shared_types::fee::{FeeConfig, FeeTier};
use shared_types::intent::{TradeIntent, TradeResult};
use shared_types::platform::Platform;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use std::path::PathBuf;
use transaction_validation::validator::{validate_transaction, TradePreview};

fn get_wallet_dir() -> PathBuf {
    let mut dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("./data"));
    dir.push("axiom-trading-terminal");
    dir.push("wallets");
    dir
}

/// Create a new Solana wallet, encrypt it locally with password, and store to disk.
#[tauri::command]
fn create_new_wallet(label: String, password: String) -> Result<WalletSummary, String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters long".to_string());
    }

    let keypair = Keypair::new();
    let keystore = encrypt_keypair(&keypair, &label, &password)
        .map_err(|e| format!("Keystore encryption error: {}", e))?;

    let dir = get_wallet_dir();
    save_keystore(&keystore, &dir).map_err(|e| format!("Failed to save keystore: {}", e))?;

    Ok(WalletSummary {
        pubkey: keystore.pubkey,
        label: keystore.label,
        is_default: true,
    })
}

/// Import an existing private key (base58 encoded or JSON byte array), encrypt it, and save to disk.
#[tauri::command]
fn import_wallet_private_key(
    secret_base58: String,
    label: String,
    password: String,
) -> Result<WalletSummary, String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters long".to_string());
    }

    let trimmed = secret_base58.trim().trim_matches('"').trim();

    let secret_bytes = if trimmed.starts_with('[') && trimmed.ends_with(']') {
        serde_json::from_str::<Vec<u8>>(trimmed)
            .map_err(|e| format!("Invalid JSON byte array key: {}", e))?
    } else {
        bs58::decode(trimmed)
            .into_vec()
            .map_err(|e| format!("Invalid base58 private key: {}", e))?
    };

    let keypair = if secret_bytes.len() == 64 {
        Keypair::try_from(secret_bytes.as_slice())
            .map_err(|e| format!("Invalid 64-byte keypair: {}", e))?
    } else if secret_bytes.len() == 32 {
        let seed: [u8; 32] = secret_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "Failed to convert 32-byte seed".to_string())?;
        Keypair::new_from_array(seed)
    } else {
        return Err(format!(
            "Invalid key length ({} bytes). Expected 32-byte seed or 64-byte keypair",
            secret_bytes.len()
        ));
    };

    let keystore = encrypt_keypair(&keypair, &label, &password)
        .map_err(|e| format!("Keystore encryption error: {}", e))?;

    let dir = get_wallet_dir();
    save_keystore(&keystore, &dir).map_err(|e| format!("Failed to save keystore: {}", e))?;

    Ok(WalletSummary {
        pubkey: keystore.pubkey,
        label: keystore.label,
        is_default: true,
    })
}

/// List all wallets stored locally.
#[tauri::command]
fn list_local_wallets() -> Result<Vec<WalletSummary>, String> {
    let dir = get_wallet_dir();
    list_wallets(&dir).map_err(|e| format!("Failed to list wallets: {}", e))
}

/// Validate a trade intent against local security bounds and produce a TradePreview.
///
/// Independently validates that:
/// 1. The signer matches the local wallet.
/// 2. Platform fee recipient matches our configured treasury, NOT an arbitrary wallet from the intent.
/// 3. The transaction instructions compile a valid fee transfer to the configured treasury.
#[tauri::command]
fn validate_intent_command(
    intent: TradeIntent,
    expected_signer: String,
) -> Result<TradePreview, String> {
    let signer = expected_signer
        .parse::<Pubkey>()
        .map_err(|e| format!("Invalid signer pubkey: {}", e))?;

    let configured_treasury = shared_types::fee::CONFIGURED_PLATFORM_TREASURY;

    // Independent check: the intent cannot redirect platform fees to an arbitrary wallet
    if intent.fee.recipient != configured_treasury {
        return Err(format!(
            "Fee security violation: intent fee recipient ({}) does not match configured treasury ({})",
            intent.fee.recipient, configured_treasury
        ));
    }

    let fee_config = FeeConfig {
        version: 1,
        fee_wallet: configured_treasury,
        tier: FeeTier::Default,
    };

    let preview = validate_transaction(&intent, &signer, &fee_config)
        .map_err(|e| format!("Transaction validation failed: {}", e))?;

    // Independent check: verified transaction must route the fee to our configured treasury
    if preview.platform_fee_recipient != configured_treasury {
        return Err(format!(
            "Fee security violation: transaction routes fee to {}, but configured treasury is {}",
            preview.platform_fee_recipient, configured_treasury
        ));
    }

    // Check that fee transfer was not missing
    if intent.fee.amount_atomic > 0
        && preview
            .warnings
            .iter()
            .any(|w| w.contains("Platform fee transfer instruction was not found"))
    {
        return Err(
            "Fee security violation: missing platform fee transfer to configured treasury"
                .to_string(),
        );
    }

    Ok(preview)
}

/// Unlock keystore locally, verify intent, and submit.
///
/// Fail closed: Live on-chain broadcast and confirmation are not implemented yet.
/// In accordance with Phase 1 safety requirements, live trades must never sign
/// and report success without actual on-chain submission.
#[tauri::command]
async fn unlock_and_sign_intent(
    intent: TradeIntent,
    password: String,
) -> Result<TradeResult, String> {
    let dir = get_wallet_dir();
    let keystore = load_keystore(&intent.wallet_pubkey.to_string(), &dir)
        .map_err(|e| format!("Failed to load wallet keystore: {}", e))?;

    let keypair = decrypt_keypair(&keystore, &password)
        .map_err(|e| format!("Keystore unlock failed: {}", e))?;

    // Validate that the keypair matches the transaction signer
    if keypair.pubkey() != intent.wallet_pubkey {
        return Err("Unlocked keypair does not match intent wallet".to_string());
    }

    // Fail closed: Live on-chain transaction submission and confirmation are not available yet.
    // Do not sign or report success without actual on-chain broadcast.
    Err("Live trading is not available yet. No transaction was submitted.".to_string())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LiveTokenData {
    pub symbol: String,
    pub name: String,
    pub mint: String,
    pub price_sol: f64,
    pub price_usd: f64,
    pub mcap_usd: f64,
    pub volume_24h: f64,
    pub change_24h: f64,
    pub dex_id: String,
    pub pair_address: String,
    pub image_url: Option<String>,
}

/// Query real on-chain SOL balance for a given wallet address.
#[tauri::command]
async fn get_wallet_balance(pubkey: String, rpc_url: Option<String>) -> Result<f64, String> {
    let rpc = rpc_url.unwrap_or_else(|| PUBLIC_SOLANA_RPC.to_string());
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getBalance",
        "params": [pubkey]
    });
    let res = client
        .post(&rpc)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {}", e))?;
    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse RPC response: {}", e))?;
    if let Some(err) = json.get("error") {
        return Err(format!("RPC error: {}", err));
    }
    let lamports = json["result"]["value"].as_u64().unwrap_or(0);
    Ok(lamports as f64 / 1_000_000_000.0)
}

/// Fetch real-time SOL / USD market price.
#[tauri::command]
async fn fetch_live_sol_price() -> Result<f64, String> {
    let client = reqwest::Client::new();
    let res = client
        .get("https://api.binance.com/api/v3/ticker/price?symbol=SOLUSDT")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch SOL price: {}", e))?;
    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse price JSON: {}", e))?;
    let price_str = json["price"].as_str().unwrap_or("0");
    price_str
        .parse::<f64>()
        .map_err(|e| format!("Invalid price: {}", e))
}

/// Search live Solana tokens from DexScreener by query, symbol, or contract address.
#[tauri::command]
async fn search_live_tokens(query: String) -> Result<Vec<LiveTokenData>, String> {
    let client = reqwest::Client::new();
    let q = if query.trim().is_empty() {
        "solana"
    } else {
        query.trim()
    };
    let url = format!("https://api.dexscreener.com/latest/dex/search?q={}", q);
    let res = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {}", e))?;
    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Invalid search JSON: {}", e))?;

    let mut tokens = Vec::new();
    if let Some(pairs) = json["pairs"].as_array() {
        for p in pairs {
            if p["chainId"].as_str() == Some("solana") {
                let base = &p["baseToken"];
                let symbol = base["symbol"].as_str().unwrap_or("UNKNOWN").to_string();
                let name = base["name"].as_str().unwrap_or(&symbol).to_string();
                let mint = base["address"].as_str().unwrap_or("").to_string();
                let price_usd = p["priceUsd"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let price_sol = p["priceNative"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let mcap_usd = p["marketCap"]
                    .as_f64()
                    .or_else(|| p["fdv"].as_f64())
                    .unwrap_or(0.0);
                let volume_24h = p["volume"]["h24"].as_f64().unwrap_or(0.0);
                let change_24h = p["priceChange"]["h24"].as_f64().unwrap_or(0.0);
                let dex_id = p["dexId"].as_str().unwrap_or("raydium").to_string();
                let pair_address = p["pairAddress"].as_str().unwrap_or("").to_string();
                let image_url = p["info"]["imageUrl"].as_str().map(|s| s.to_string());

                if !mint.is_empty() && !tokens.iter().any(|t: &LiveTokenData| t.mint == mint) {
                    tokens.push(LiveTokenData {
                        symbol,
                        name,
                        mint,
                        price_sol,
                        price_usd,
                        mcap_usd,
                        volume_24h,
                        change_24h,
                        dex_id,
                        pair_address,
                        image_url,
                    });
                }
                if tokens.len() >= 30 {
                    break;
                }
            }
        }
    }
    Ok(tokens)
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct IntentRequestParams {
    pub wallet_pubkey: String,
    pub mint: String,
    pub amount_atomic: u64,
    pub slippage_bps: Option<u16>,
    pub fee_tier: Option<FeeTier>,
    pub priority_fee_micro_lamports: Option<u64>,
    pub platform: Option<Platform>,
    pub dex_id: Option<u8>,
}

/// Ping Axum strategy server health.
#[tauri::command]
async fn check_server_health(server_url: Option<String>) -> Result<bool, String> {
    let base_url = server_url.unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let endpoint = format!("{}/health", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
        .unwrap_or_default();
    match client.get(&endpoint).send().await {
        Ok(res) => Ok(res.status().is_success()),
        Err(_) => Ok(false),
    }
}

/// Request an unsigned trade intent from the Axum strategy server.
/// Fails explicitly if the server is unreachable or returns an error.
#[tauri::command]
async fn request_trade_intent(
    params: IntentRequestParams,
    side: String,
    server_url: Option<String>,
) -> Result<TradeIntent, String> {
    let base_url = server_url.unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let side_lower = side.to_lowercase();
    let endpoint = format!(
        "{}/api/v1/intent/{}",
        base_url.trim_end_matches('/'),
        side_lower
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2500))
        .build()
        .map_err(|e| e.to_string())?;

    // Attempt calling strategy server
    match client.post(&endpoint).json(&params).send().await {
        Ok(res) if res.status().is_success() => {
            let intent = res
                .json::<TradeIntent>()
                .await
                .map_err(|e| format!("Failed to parse intent from server: {}", e))?;
            Ok(intent)
        }
        Ok(res) => {
            let status = res.status();
            let err_text = res.text().await.unwrap_or_default();
            Err(format!("Strategy server error ({}): {}", status, err_text))
        }
        Err(e) => Err(format!(
            "Strategy server at {} is unreachable ({}). Live trade intent cannot be generated.",
            endpoint, e
        )),
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CompareRoutesParams {
    pub mint: String,
    pub side: String,
    pub amount_atomic: u64,
    pub fee_tier: Option<FeeTier>,
    #[serde(default)]
    pub min_liquidity_usd: Option<f64>,
    #[serde(default)]
    pub min_volume_usd: Option<f64>,
}

/// Request live read-only best-route comparison from the Axum strategy server.
#[tauri::command]
async fn compare_routes_command(
    params: CompareRoutesParams,
    server_url: Option<String>,
) -> Result<serde_json::Value, String> {
    let base_url = server_url.unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let endpoint = format!("{}/api/v1/routes/compare", base_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(10000))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    match client.post(&endpoint).json(&params).send().await {
        Ok(res) if res.status().is_success() => {
            let data = res
                .json::<serde_json::Value>()
                .await
                .map_err(|e| format!("Failed to parse route comparison response: {}", e))?;
            Ok(data)
        }
        Ok(res) => {
            let status = res.status();
            let err_text = res.text().await.unwrap_or_default();
            Err(format!("Route comparator error ({}): {}", status, err_text))
        }
        Err(e) => Err(format!(
            "Strategy server at {} is unreachable ({}). Route comparison unavailable.",
            endpoint, e
        )),
    }
}

pub const PUBLIC_SOLANA_RPC: &str = "https://mainnet.helius-rpc.com/?api-key=eda6e938-7c34-4332-bf0e-3722786de677";

pub fn is_custom_rpc(rpc_url: &str) -> bool {
    let trimmed = rpc_url.trim();
    !trimmed.is_empty() && !trimmed.contains("api.mainnet-beta.solana.com")
}

/// Prepare an unsigned swap transaction candidate and independently validate/simulate it natively in Rust.
#[tauri::command]
async fn prepare_and_validate_swap_command(
    params: shared_types::swap::PrepareSwapRequest,
    expected_signer: String,
    server_url: Option<String>,
    rpc_url: Option<String>,
) -> Result<shared_types::swap::SwapValidationResult, String> {
    let signer = expected_signer
        .parse::<Pubkey>()
        .map_err(|e| format!("Invalid expected signer: {}", e))?;

    let base_url = server_url.unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
    let endpoint = format!("{}/api/v1/swap/prepare", base_url.trim_end_matches('/'));

    let rpc = rpc_url.unwrap_or_else(|| PUBLIC_SOLANA_RPC.to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(8000))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let res = match client.post(&endpoint).json(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            return Err(format!(
                "Strategy server at {} is unreachable ({}). Cannot prepare transaction candidate.",
                endpoint, e
            ));
        }
    };

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Strategy server error ({}): {}", status, body));
    }

    let candidate = res
        .json::<shared_types::swap::PreparedSwapCandidate>()
        .await
        .map_err(|e| format!("Failed to parse candidate from server: {}", e))?;

    let effective_tier = match params.fee_tier {
        Some(shared_types::fee::FeeTier::UserRpc) => {
            if is_custom_rpc(&rpc) {
                shared_types::fee::FeeTier::UserRpc
            } else {
                shared_types::fee::FeeTier::Default
            }
        }
        Some(shared_types::fee::FeeTier::Default) | None => shared_types::fee::FeeTier::Default,
    };

    let fee_config = shared_types::fee::FeeConfig {
        version: 1,
        fee_wallet: shared_types::fee::CONFIGURED_PLATFORM_TREASURY,
        tier: effective_tier,
    };

    // Run independent native validation and read-only RPC simulation in native Rust desktop app!
    let mut validation_result = transaction_validation::validator::validate_and_simulate_swap_candidate(
        &candidate,
        &signer,
        &fee_config,
        &rpc,
    )
    .await;
    validation_result.candidate = Some(candidate);

    Ok(validation_result)
}

/// Phase 4: Sign an independently validated candidate, submit to Solana RPC with preflight, and track confirmation.
#[tauri::command]
async fn sign_and_submit_swap_candidate_command(
    candidate: shared_types::swap::PreparedSwapCandidate,
    expected_signer: String,
    password: String,
    cluster_safety_confirmed: bool,
    fee_tier: Option<shared_types::fee::FeeTier>,
    rpc_url: Option<String>,
) -> Result<shared_types::swap::SwapSubmissionResult, String> {
    let rpc_endpoint = rpc_url.unwrap_or_else(|| PUBLIC_SOLANA_RPC.to_string());
    let rpc = submission::HttpSolanaRpc::new(rpc_endpoint.clone());
    let dir = get_wallet_dir();

    let effective_tier = match fee_tier {
        Some(shared_types::fee::FeeTier::UserRpc) => {
            if is_custom_rpc(&rpc_endpoint) {
                shared_types::fee::FeeTier::UserRpc
            } else {
                shared_types::fee::FeeTier::Default
            }
        }
        Some(shared_types::fee::FeeTier::Default) | None => shared_types::fee::FeeTier::Default,
    };

    let result = submission::sign_and_submit_swap_candidate(
        &candidate,
        &expected_signer,
        &password,
        cluster_safety_confirmed,
        &dir,
        Some(effective_tier),
        &rpc,
        45,   // 45 seconds polling timeout
        1500, // 1.5 seconds polling interval
    )
    .await;

    Ok(result)
}

/// Phase 4: Query on-chain status of an existing submitted transaction signature.
#[tauri::command]
async fn query_swap_status_command(
    signature: String,
    last_valid_block_height: u64,
    rpc_url: Option<String>,
) -> Result<shared_types::swap::SwapSubmissionResult, String> {
    let rpc_endpoint = rpc_url.unwrap_or_else(|| PUBLIC_SOLANA_RPC.to_string());
    let rpc = submission::HttpSolanaRpc::new(rpc_endpoint);

    let result = submission::confirm_transaction_status(
        &signature,
        last_valid_block_height,
        &rpc,
        30,
        1500,
    )
    .await;

    Ok(result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            create_new_wallet,
            import_wallet_private_key,
            list_local_wallets,
            validate_intent_command,
            unlock_and_sign_intent,
            get_wallet_balance,
            fetch_live_sol_price,
            search_live_tokens,
            check_server_health,
            request_trade_intent,
            compare_routes_command,
            prepare_and_validate_swap_command,
            sign_and_submit_swap_candidate_command,
            query_swap_status_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

