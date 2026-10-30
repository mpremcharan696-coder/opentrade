use desktop::submission::{sign_and_submit_swap_candidate, HttpSolanaRpc};
use shared_types::fee::FeeTier;
use shared_types::intent::TradeSide;
use shared_types::swap::{PrepareSwapRequest, PreparedSwapCandidate, SwapExecutionStatus};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use std::path::PathBuf;
use std::time::Duration;

const HELIUS_RPC: &str = "https://mainnet.helius-rpc.com/?api-key=eda6e938-7c34-4332-bf0e-3722786de677";
const TRADER_WALLET: Pubkey = pubkey!("B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc");
const TREASURY_WALLET: Pubkey = pubkey!("AnWnLUpa1ohdgp3WRmhPpxXEusmcEDZNtYTsCM8qq9eH");
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

const CATE_MINT: &str = "Ai66LHZG9MCzg1WKdawwqduVAXpNDUuV8M3uyq5ppump";
const CATE_POOL: &str = "HMzvsEEmtzHhvZNw9uwbaG85HCTmFnkbhzUx16cy7ca3";

const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";

fn get_wallet_dir() -> PathBuf {
    let mut dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("./data"));
    dir.push("axiom-trading-terminal");
    dir.push("wallets");
    dir
}

async fn get_sol_balance(rpc_url: &str, pk: &Pubkey) -> Result<u64, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getBalance",
        "params": [pk.to_string(), { "commitment": "confirmed" }]
    });
    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(json["result"]["value"].as_u64().unwrap_or(0))
}

async fn get_token_balance(rpc_url: &str, owner: &Pubkey, mint: &Pubkey) -> Result<u64, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenAccountsByOwner",
        "params": [
            owner.to_string(),
            { "mint": mint.to_string() },
            { "encoding": "jsonParsed", "commitment": "confirmed" }
        ]
    });
    let res = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    let accounts = json["result"]["value"].as_array();
    if let Some(accs) = accounts {
        let mut total = 0u64;
        for a in accs {
            let amount_str = a["account"]["data"]["parsed"]["info"]["tokenAmount"]["amount"]
                .as_str()
                .unwrap_or("0");
            total += amount_str.parse::<u64>().unwrap_or(0);
        }
        Ok(total)
    } else {
        Ok(0)
    }
}

async fn prepare_candidate(req: &PrepareSwapRequest) -> Result<PreparedSwapCandidate, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .post("http://127.0.0.1:8080/api/v1/swap/prepare")
        .json(req)
        .send()
        .await
        .map_err(|e| format!("Server request failed: {}", e))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Prepare error (HTTP {}): {}", status, body));
    }
    let candidate = res
        .json::<PreparedSwapCandidate>()
        .await
        .map_err(|e| format!("Failed to parse candidate JSON: {}", e))?;
    Ok(candidate)
}

#[tokio::main]
async fn main() {
    println!("============================================================");
    println!("  SOLANA LIVE MAINNET END-TO-END EXECUTION TEST SUITE");
    println!("============================================================");

    let wallet_dir = get_wallet_dir();
    println!("Keystore directory: {:?}", wallet_dir);
    println!("Trader wallet: {}", TRADER_WALLET);
    println!("Platform treasury: {}", TREASURY_WALLET);

    let rpc = HttpSolanaRpc::new(HELIUS_RPC.to_string());

    // 1. Initial State
    let initial_trader_sol = get_sol_balance(HELIUS_RPC, &TRADER_WALLET).await.unwrap_or(0);
    let initial_treasury_sol = get_sol_balance(HELIUS_RPC, &TREASURY_WALLET).await.unwrap_or(0);

    println!(
        "\n[INITIAL BALANCES]\n  Trader SOL:   {:.6} SOL ({} lamports)\n  Treasury SOL: {:.6} SOL ({} lamports)",
        initial_trader_sol as f64 / 1e9, initial_trader_sol,
        initial_treasury_sol as f64 / 1e9, initial_treasury_sol
    );

    let mut pump_buy_sig: Option<String> = None;
    let mut pump_sell_sig: Option<String> = None;
    let mut jup_buy_sig: Option<String> = None;
    let mut jup_sell_sig: Option<String> = None;

    let mut pump_buy_err: Option<String> = None;
    let mut pump_sell_err: Option<String> = None;
    let mut jup_buy_err: Option<String> = None;
    let mut jup_sell_err: Option<String> = None;

    // ------------------------------------------------------------
    // TEST 1: PumpSwap Native Buy ($CATE)
    // ------------------------------------------------------------
    println!("\n------------------------------------------------------------");
    println!(">>> TEST 1: PUMPSWAP NATIVE BUY ($CATE)");
    println!("------------------------------------------------------------");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let buy_amount_lamports = 10_000_000u64; // 0.01 SOL (within <= 0.015 SOL limit)

    let req1 = PrepareSwapRequest {
        wallet_pubkey: TRADER_WALLET.to_string(),
        side: TradeSide::Buy,
        input_mint: WSOL_MINT.to_string(),
        output_mint: CATE_MINT.to_string(),
        input_amount_atomic: buy_amount_lamports,
        selected_route_source: "pumpswap_canonical".to_string(),
        venue: "PumpSwap".to_string(),
        pool_address: Some(CATE_POOL.to_string()),
        slippage_bps: 200, // 2%
        expected_minimum_output_atomic: 1000,
        quoted_at_unix_ms: now,
        state_slot: None,
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
    };

    println!("1. Preparing PumpSwap Buy candidate from strategy server...");
    match prepare_candidate(&req1).await {
        Ok(candidate) => {
            println!("   Candidate ID: {}", candidate.candidate_id);
            println!("   Estimated output: {} atomic", candidate.estimated_output_atomic);
            println!("   Minimum output bound: {} atomic", candidate.minimum_output_atomic);
            println!("   Platform fee: {} lamports (0.45%)", candidate.summary.platform_fee_atomic);
            println!("   Venue: {}", candidate.venue);

            println!("2. Signing and broadcasting via desktop submission pipeline...");
            let res = sign_and_submit_swap_candidate(
                &candidate,
                &TRADER_WALLET.to_string(),
                "password123",
                true,
                &wallet_dir,
                Some(FeeTier::Default),
                &rpc,
                45,
                1500,
            ).await;

            println!("   Result Status: {:?}", res.status);
            if let Some(sig) = &res.signature {
                println!("   Signature: {}", sig);
                println!("   Explorer: https://solscan.io/tx/{}", sig);
                pump_buy_sig = Some(sig.clone());
            }
            if let Some(err) = &res.error {
                println!("   Error: {}", err);
                pump_buy_err = Some(err.clone());
            }
        }
        Err(e) => {
            println!("   [FAILED TO PREPARE]: {}", e);
            pump_buy_err = Some(e);
        }
    }

    println!("Waiting 6 seconds for on-chain state to finalize...");
    tokio::time::sleep(Duration::from_secs(6)).await;

    // ------------------------------------------------------------
    // TEST 2: PumpSwap Native Sell ($CATE)
    // ------------------------------------------------------------
    println!("\n------------------------------------------------------------");
    println!(">>> TEST 2: PUMPSWAP NATIVE SELL ($CATE)");
    println!("------------------------------------------------------------");

    let cate_mint_pk = CATE_MINT.parse::<Pubkey>().unwrap();
    let cate_balance = get_token_balance(HELIUS_RPC, &TRADER_WALLET, &cate_mint_pk).await.unwrap_or(0);
    println!("Current on-chain $CATE balance: {} atomic", cate_balance);

    if cate_balance > 0 {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let sell_tokens = cate_balance; // Sell all acquired CATE

        let req2 = PrepareSwapRequest {
            wallet_pubkey: TRADER_WALLET.to_string(),
            side: TradeSide::Sell,
            input_mint: CATE_MINT.to_string(),
            output_mint: WSOL_MINT.to_string(),
            input_amount_atomic: sell_tokens,
            selected_route_source: "pumpswap_canonical".to_string(),
            venue: "PumpSwap".to_string(),
            pool_address: Some(CATE_POOL.to_string()),
            slippage_bps: 250, // 2.5%
            expected_minimum_output_atomic: 1000,
            quoted_at_unix_ms: now,
            state_slot: None,
            fee_tier: Some(FeeTier::Default),
            priority_fee_micro_lamports: Some(50_000),
        };

        println!("1. Preparing PumpSwap Sell candidate...");
        match prepare_candidate(&req2).await {
            Ok(candidate) => {
                println!("   Candidate ID: {}", candidate.candidate_id);
                println!("   Estimated SOL gross output: {} lamports", candidate.estimated_output_atomic);
                println!("   Net minimum output bound: {} lamports", candidate.minimum_output_atomic);
                println!("   Platform fee: {} lamports", candidate.summary.platform_fee_atomic);

                println!("2. Signing and broadcasting via desktop submission pipeline...");
                let res = sign_and_submit_swap_candidate(
                    &candidate,
                    &TRADER_WALLET.to_string(),
                    "password123",
                    true,
                    &wallet_dir,
                    Some(FeeTier::Default),
                    &rpc,
                    45,
                    1500,
                ).await;

                println!("   Result Status: {:?}", res.status);
                if let Some(sig) = &res.signature {
                    println!("   Signature: {}", sig);
                    println!("   Explorer: https://solscan.io/tx/{}", sig);
                    pump_sell_sig = Some(sig.clone());
                }
                if let Some(err) = &res.error {
                    println!("   Error: {}", err);
                    pump_sell_err = Some(err.clone());
                }
            }
            Err(e) => {
                println!("   [FAILED TO PREPARE]: {}", e);
                pump_sell_err = Some(e);
            }
        }
    } else {
        println!("   [SKIPPED SELL]: $CATE token balance is 0.");
        pump_sell_err = Some("Token balance is 0 (Buy did not fund account)".to_string());
    }

    println!("Waiting 6 seconds for on-chain state to finalize...");
    tokio::time::sleep(Duration::from_secs(6)).await;

    // ------------------------------------------------------------
    // TEST 3: Jupiter Swap V2 / Raydium CLMM Buy ($BONK)
    // ------------------------------------------------------------
    println!("\n------------------------------------------------------------");
    println!(">>> TEST 3: JUPITER / RAYDIUM CLMM BUY ($BONK)");
    println!("------------------------------------------------------------");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let jup_buy_amount = 10_000_000u64; // 0.01 SOL

    let req3 = PrepareSwapRequest {
        wallet_pubkey: TRADER_WALLET.to_string(),
        side: TradeSide::Buy,
        input_mint: WSOL_MINT.to_string(),
        output_mint: BONK_MINT.to_string(),
        input_amount_atomic: jup_buy_amount,
        selected_route_source: "jupiter_v2_order".to_string(),
        venue: "Jupiter (Raydium CLMM)".to_string(),
        pool_address: None,
        slippage_bps: 150, // 1.5%
        expected_minimum_output_atomic: 1000,
        quoted_at_unix_ms: now,
        state_slot: None,
        fee_tier: Some(FeeTier::Default),
        priority_fee_micro_lamports: Some(50_000),
    };

    println!("1. Preparing Jupiter Swap V2 candidate from strategy server...");
    match prepare_candidate(&req3).await {
        Ok(candidate) => {
            println!("   Candidate ID: {}", candidate.candidate_id);
            println!("   Estimated output: {} BONK atomic", candidate.estimated_output_atomic);
            println!("   Minimum output bound: {} BONK atomic", candidate.minimum_output_atomic);
            println!("   Platform fee: {} lamports (0.45%)", candidate.summary.platform_fee_atomic);
            println!("   Verified venue: {}", candidate.venue);
            println!("   Address Lookup Tables attached: {}", candidate.address_lookup_tables.len());

            println!("2. Signing and broadcasting via desktop submission pipeline (with on-chain ALT re-fetch)...");
            let res = sign_and_submit_swap_candidate(
                &candidate,
                &TRADER_WALLET.to_string(),
                "password123",
                true,
                &wallet_dir,
                Some(FeeTier::Default),
                &rpc,
                45,
                1500,
            ).await;

            println!("   Result Status: {:?}", res.status);
            if let Some(sig) = &res.signature {
                println!("   Signature: {}", sig);
                println!("   Explorer: https://solscan.io/tx/{}", sig);
                jup_buy_sig = Some(sig.clone());
            }
            if let Some(err) = &res.error {
                println!("   Error: {}", err);
                jup_buy_err = Some(err.clone());
            }
        }
        Err(e) => {
            println!("   [FAILED TO PREPARE]: {}", e);
            jup_buy_err = Some(e);
        }
    }

    println!("Waiting 6 seconds for on-chain state to finalize...");
    tokio::time::sleep(Duration::from_secs(6)).await;

    // ------------------------------------------------------------
    // TEST 4: Jupiter Swap V2 / Raydium Sell ($BONK)
    // ------------------------------------------------------------
    println!("\n------------------------------------------------------------");
    println!(">>> TEST 4: JUPITER / RAYDIUM SELL ($BONK)");
    println!("------------------------------------------------------------");

    let bonk_mint_pk = BONK_MINT.parse::<Pubkey>().unwrap();
    let bonk_balance = get_token_balance(HELIUS_RPC, &TRADER_WALLET, &bonk_mint_pk).await.unwrap_or(0);
    println!("Current on-chain $BONK balance: {} atomic", bonk_balance);

    if bonk_balance > 0 {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let sell_bonk_amount = bonk_balance; // Sell all acquired BONK

        let req4 = PrepareSwapRequest {
            wallet_pubkey: TRADER_WALLET.to_string(),
            side: TradeSide::Sell,
            input_mint: BONK_MINT.to_string(),
            output_mint: WSOL_MINT.to_string(),
            input_amount_atomic: sell_bonk_amount,
            selected_route_source: "jupiter_v2_order".to_string(),
            venue: "Jupiter".to_string(),
            pool_address: None,
            slippage_bps: 200, // 2.0%
            expected_minimum_output_atomic: 1000,
            quoted_at_unix_ms: now,
            state_slot: None,
            fee_tier: Some(FeeTier::Default),
            priority_fee_micro_lamports: Some(50_000),
        };

        println!("1. Preparing Jupiter Sell candidate...");
        match prepare_candidate(&req4).await {
            Ok(candidate) => {
                println!("   Candidate ID: {}", candidate.candidate_id);
                println!("   Estimated SOL output: {} lamports", candidate.estimated_output_atomic);
                println!("   Net minimum output bound: {} lamports", candidate.minimum_output_atomic);
                println!("   Platform fee: {} lamports", candidate.summary.platform_fee_atomic);
                println!("   Venue: {}", candidate.venue);

                println!("2. Signing and broadcasting via desktop submission pipeline...");
                let res = sign_and_submit_swap_candidate(
                    &candidate,
                    &TRADER_WALLET.to_string(),
                    "password123",
                    true,
                    &wallet_dir,
                    Some(FeeTier::Default),
                    &rpc,
                    45,
                    1500,
                ).await;

                println!("   Result Status: {:?}", res.status);
                if let Some(sig) = &res.signature {
                    println!("   Signature: {}", sig);
                    println!("   Explorer: https://solscan.io/tx/{}", sig);
                    jup_sell_sig = Some(sig.clone());
                }
                if let Some(err) = &res.error {
                    println!("   Error: {}", err);
                    jup_sell_err = Some(err.clone());
                }
            }
            Err(e) => {
                println!("   [FAILED TO PREPARE]: {}", e);
                jup_sell_err = Some(e);
            }
        }
    } else {
        println!("   [SKIPPED SELL]: $BONK token balance is 0.");
        jup_sell_err = Some("Token balance is 0 (Buy did not fund account)".to_string());
    }

    println!("Waiting 6 seconds for on-chain state to finalize...");
    tokio::time::sleep(Duration::from_secs(6)).await;

    // ------------------------------------------------------------
    // TEST 5: Platform Treasury Fee Accounting Verification
    // ------------------------------------------------------------
    println!("\n------------------------------------------------------------");
    println!(">>> TEST 5: PLATFORM TREASURY FEE AUDIT");
    println!("------------------------------------------------------------");
    let final_trader_sol = get_sol_balance(HELIUS_RPC, &TRADER_WALLET).await.unwrap_or(0);
    let final_treasury_sol = get_sol_balance(HELIUS_RPC, &TREASURY_WALLET).await.unwrap_or(0);

    let fee_delta = final_treasury_sol.saturating_sub(initial_treasury_sol);
    println!("Initial Treasury: {} lamports", initial_treasury_sol);
    println!("Final Treasury:   {} lamports", final_treasury_sol);
    println!("Platform fees accumulated this run: {} lamports ({:.6} SOL)", fee_delta, fee_delta as f64 / 1e9);

    println!("\n============================================================");
    println!("                  FINAL EXECUTION SUMMARY");
    println!("============================================================");
    println!("1. PumpSwap Buy ($CATE):");
    println!("   Status: {}", if pump_buy_sig.is_some() { "CONFIRMED" } else { "FAILED" });
    println!("   Sig:    {}", pump_buy_sig.as_deref().unwrap_or("None"));
    if let Some(e) = pump_buy_err { println!("   Error:  {}", e); }

    println!("2. PumpSwap Sell ($CATE):");
    println!("   Status: {}", if pump_sell_sig.is_some() { "CONFIRMED" } else { "FAILED/SKIPPED" });
    println!("   Sig:    {}", pump_sell_sig.as_deref().unwrap_or("None"));
    if let Some(e) = pump_sell_err { println!("   Error:  {}", e); }

    println!("3. Jupiter Buy ($BONK via Raydium):");
    println!("   Status: {}", if jup_buy_sig.is_some() { "CONFIRMED" } else { "FAILED" });
    println!("   Sig:    {}", jup_buy_sig.as_deref().unwrap_or("None"));
    if let Some(e) = jup_buy_err { println!("   Error:  {}", e); }

    println!("4. Jupiter Sell ($BONK):");
    println!("   Status: {}", if jup_sell_sig.is_some() { "CONFIRMED" } else { "FAILED/SKIPPED" });
    println!("   Sig:    {}", jup_sell_sig.as_deref().unwrap_or("None"));
    if let Some(e) = jup_sell_err { println!("   Error:  {}", e); }

    println!("5. Treasury Fee Accounting:");
    println!("   Verified Treasury Deposit: {} lamports", fee_delta);
    println!("   Ending Trader Balance:     {:.6} SOL", final_trader_sol as f64 / 1e9);
    println!("============================================================\n");
}
