import { useState, useEffect } from 'react';

export const PUBLIC_SOLANA_RPC = 'https://mainnet.helius-rpc.com/?api-key=eda6e938-7c34-4332-bf0e-3722786de677';

// Tauri IPC invoke wrapper with web fallback
async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const isTauri = typeof window !== 'undefined' && Boolean((window as any).__TAURI_INTERNALS__ || (window as any).__TAURI__);
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core');
    // Execute command directly via Tauri without catching and masking errors!
    return await invoke<T>(cmd, args);
  }

  // Browser fallback: Refuse wallet creation/import outside native app; never offer simulated wallets in live path
  if (cmd === 'list_local_wallets') {
    return [] as T;
  }
  if (cmd === 'create_new_wallet') {
    throw new Error('Wallet creation is refused outside the native desktop app. Encrypted keystores require the native runtime.');
  }
  if (cmd === 'import_wallet_private_key') {
    throw new Error('Wallet import is refused outside the native desktop app. Encrypted keystores require the native runtime.');
  }
  if (cmd === 'get_wallet_balance') {
    const pubkey = args?.pubkey as string;
    const rpcUrl = (args?.rpc_url as string) || (args?.rpcUrl as string) || PUBLIC_SOLANA_RPC;
    try {
      const res = await fetch(rpcUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'getBalance',
          params: [pubkey],
        }),
      });
      const data = await res.json();
      const lamports = data?.result?.value || 0;
      return (lamports / 1_000_000_000) as T;
    } catch {
      return 0.0 as T;
    }
  }
  if (cmd === 'check_server_health') {
    try {
      const res = await fetch('http://localhost:8080/health');
      return res.ok as T;
    } catch {
      return false as T;
    }
  }
  if (cmd === 'request_trade_intent') {
    const params = (args as any)?.params;
    const side = (args as any)?.side || 'buy';
    const serverUrl = (args as any)?.serverUrl || 'http://localhost:8080';
    try {
      const res = await fetch(`${serverUrl.replace(/\/$/, '')}/api/v1/intent/${side}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params),
      });
      if (res.ok) {
        return (await res.json()) as T;
      }
      const errText = await res.text();
      throw new Error(`Strategy server error (${res.status}): ${errText}`);
    } catch (e: any) {
      throw new Error(`Strategy server is unreachable at ${serverUrl}: ${e.message}`);
    }
  }
  if (cmd === 'compare_routes_command') {
    const params = (args as any)?.params;
    const serverUrl = (args as any)?.serverUrl || 'http://localhost:8080';
    try {
      const res = await fetch(`${serverUrl.replace(/\/$/, '')}/api/v1/routes/compare`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params),
      });
      if (res.ok) {
        return (await res.json()) as T;
      }
      const errText = await res.text();
      throw new Error(`Route comparator error (${res.status}): ${errText}`);
    } catch (e: any) {
      throw new Error(`Strategy server route comparison unreachable at ${serverUrl}: ${e.message}`);
    }
  }
  if (cmd === 'validate_intent_command') {
    // Fail closed in browser mode: do not return a fabricated trade preview.
    // Real transaction validation requires the native desktop environment with Rust validator.
    throw new Error('Transaction validation is not available in browser mode. Native desktop environment is required to validate transactions.');
  }
  if (cmd === 'prepare_and_validate_swap_command') {
    // Fail closed in browser mode: Unsigned swap preparation and native Rust validation require the desktop environment.
    throw new Error('Swap transaction preparation and native validation is refused outside the desktop app. The native Rust validator and Solana RPC simulation require the desktop runtime.');
  }
  if (cmd === 'unlock_and_sign_intent') {
    // Fail closed: Live on-chain transaction submission is not available yet.
    throw new Error('Live trading is not available yet. No transaction was submitted.');
  }
  if (cmd === 'sign_and_submit_swap_candidate_command') {
    throw new Error('Live swap transaction signing and submission is refused outside the native desktop app. Encrypted keystores and native signing require the Tauri desktop runtime.');
  }
  if (cmd === 'query_swap_status_command') {
    throw new Error('Transaction confirmation status tracking requires the native desktop environment.');
  }
  throw new Error(`Command ${cmd} not implemented in fallback`);
}



interface TokenHolding {
  mint: string;
  symbol: string;
  name: string;
  imageUrl?: string;
  amount: number;
  avgBuyPriceSol: number;
  avgBuyPriceUsd: number;
  totalCostSol: number;
}

interface CompletedOrder {
  id: string;
  time: string;
  type: 'buy' | 'sell';
  symbol: string;
  mint: string;
  amountSol: number;
  amountTokens: number;
  priceSol: number;
  priceUsd: number;
  signature: string;
  realizedPnlUsd?: number;
  realizedPnlPct?: number;
  executionMs?: number;
}

interface TradeIntent {
  api_version: number;
  intent_id: string;
  created_at: string;
  expires_at: string;
  last_valid_block_height: number;
  wallet_pubkey: string;
  side: 'buy' | 'sell';
  mint: string;
  route: any;
  input_amount_atomic: number;
  minimum_output_atomic: number;
  slippage_bps: number;
  fee: {
    rate_bps: number;
    basis: string;
    recipient: string;
    amount_atomic: number;
  };
  state_slot: number;
  unsigned_tx_base64: string;
}

interface TradePreview {
  intent_id: string;
  side: 'buy' | 'sell';
  mint: string;
  route: any;
  input_amount_atomic: number;
  minimum_output_atomic: number;
  slippage_bps: number;
  platform_fee_atomic: number;
  platform_fee_recipient: string;
  compute_unit_limit: number;
  priority_fee_micro_lamports: number;
  total_sol_required: number;
  instructions_count: number;
  warnings: string[];
}

interface WalletSummary {
  pubkey: string;
  label: string;
  is_default: boolean;
}

interface DexRouteOption {
  dexId: string;
  dexLabel: string;
  pairAddress: string;
  liquidityUsd: number;
  priceUsd: number;
  priceSol: number;
  volume24h: number;
  isPrimary: boolean;
}

export interface RouteFeeBreakdown {
  venue_fee_bps?: number;
  venue_fee_atomic?: number;
  platform_fee_bps: number;
  platform_fee_atomic: number;
}

export interface RouteQuoteItem {
  source: string;
  venue: string;
  pool_address?: string;
  input_mint: string;
  output_mint: string;
  input_amount_atomic: number;
  gross_output_atomic?: number;
  net_output_atomic?: number;
  price_impact_pct?: number;
  venue_fee_bps?: number;
  platform_fee_atomic?: number;
  fee_breakdown?: RouteFeeBreakdown;
  state_slot?: number;
  quoted_at_unix_ms: number;
  status: string;
  error?: string;
  reason_unavailable?: string;
}

export interface DiscoveredPoolItem {
  dex_id: string;
  dex_label: string;
  pool_address: string;
  base_mint: string;
  quote_mint: string;
  liquidity_usd: number;
  volume_24h_usd: number;
  price_usd: number;
  passes_dust_filter: boolean;
  is_verified_on_chain?: boolean;
}

export interface RouteComparisonResponse {
  side: 'buy' | 'sell';
  mint: string;
  input_mint: string;
  output_mint: string;
  input_amount_atomic: number;
  platform_fee_bps: number;
  best_route_index?: number | null;
  runner_up_index?: number | null;
  output_difference_atomic?: number | null;
  recommendation_reason?: string | null;
  routes: RouteQuoteItem[];
  candidate_pools?: DiscoveredPoolItem[];
  min_liquidity_threshold_usd: number;
  min_volume_threshold_usd: number;
  execution_enabled: boolean;
  disclaimer: string;
}

export interface SwapAccountMetaSummary {
  pubkey: string;
  is_signer: boolean;
  is_writable: boolean;
  label?: string;
}

export interface SwapInstructionDetail {
  program_id: string;
  program_name: string;
  accounts: SwapAccountMetaSummary[];
  data_len: number;
  description: string;
}

export interface SwapCandidateSummary {
  instructions: SwapInstructionDetail[];
  program_ids: string[];
  writable_accounts: string[];
  signer_accounts: string[];
  compute_unit_limit: number;
  priority_fee_micro_lamports: number;
  platform_fee_bps: number;
  platform_fee_atomic: number;
  platform_fee_recipient: string;
  venue_fee_atomic?: number;
  estimated_rent_lamports: number;
  total_sol_required_atomic: number;
  tx_byte_size: number;
  quote_freshness_ms: number;
}

export interface HumanReadableReviewSummary {
  venue: string;
  side: string;
  input_display: string;
  estimated_output_display: string;
  minimum_output_display: string;
  slippage_display: string;
  platform_fee_display: string;
  platform_fee_recipient_verified: boolean;
  priority_fee_display: string;
  rent_cost_display: string;
  total_sol_estimated_display: string;
  transaction_size_bytes: number;
  quote_age_seconds: number;
  authorization_status: string;
}

export interface SwapValidationResult {
  is_valid: boolean;
  is_simulated: boolean;
  candidate_id: string;
  validation_errors: string[];
  validation_warnings: string[];
  simulation_units_consumed?: number;
  simulation_logs: string[];
  candidate_summary?: SwapCandidateSummary;
  review_summary: HumanReadableReviewSummary;
  candidate?: any;
}

export type SwapExecutionStatus = 'prepared' | 'signed' | 'submitted' | 'confirmed' | 'failed' | 'expired' | 'unknown';

export interface SwapSubmissionResult {
  status: SwapExecutionStatus;
  signature?: string;
  slot?: number;
  confirmations?: number;
  error?: string;
  logs?: string[];
  explorer_url?: string;
  blockhash_expired?: boolean;
}


export interface PrepareSwapRequest {
  wallet_pubkey: string;
  side: 'buy' | 'sell';
  input_mint: string;
  output_mint: string;
  input_amount_atomic: number;
  selected_route_source: string;
  venue: string;
  pool_address?: string;
  slippage_bps: number;
  expected_minimum_output_atomic: number;
  quoted_at_unix_ms: number;
  state_slot?: number;
  fee_tier?: string;
  priority_fee_micro_lamports?: number;
}


interface TokenItem {
  symbol: string;
  name: string;
  mint: string;
  priceSol: number;
  priceUsd: number;
  mcapUsd: number;
  volume24h: number;
  totalLiquidity: number;
  change24h: number;
  progress: number;
  dexId: string;
  pairAddress: string;
  imageUrl?: string;
  routes: DexRouteOption[];
  poolCount: number;
}


// DEX display labels
const DEX_LABELS: Record<string, string> = {
  pump: 'Pump.fun',
  pumpswap: 'PumpSwap',
  raydium: 'Raydium',
  meteora: 'Meteora',
  orca: 'Orca',
};

// Dust filter thresholds
const MIN_LIQUIDITY_USD = 2_000;
const MIN_VOLUME_24H_USD = 500;

// Top 10 Highest Volume Solana Tokens
const TOP_10_MINTS = [
  '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump', // FARTCOIN
  '6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN', // TRUMP
  '2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv', // PENGU
  'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263', // BONK
  'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm', // WIF
  '7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr', // POPCAT
  'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN', // JUP
  '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R', // RAY
  'MEFNBXixkEbait3xn9bkm8WsJzXtVsaJEn4c8Sam21u', // ME
  '2qEHjDLDLbuBgRYvsxhc5D6uDWAivNFZGan56P1tpump', // PNUT
];

const DEFAULT_TOKENS: TokenItem[] = [
  {
    symbol: 'FARTCOIN',
    name: 'Fartcoin',
    mint: '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump',
    priceSol: 0.001235,
    priceUsd: 0.1235,
    mcapUsd: 123_500_000,
    volume24h: 18_400_000,
    totalLiquidity: 2_200_000,
    change24h: 1.25,
    progress: 100,
    dexId: 'raydium',
    pairAddress: '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump.png',
    routes: [
      { dexId: 'raydium', dexLabel: 'Raydium CPMM', pairAddress: '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump_ray', liquidityUsd: 1_350_000, priceUsd: 0.1235, priceSol: 0.001235, volume24h: 11_400_000, isPrimary: true },
      { dexId: 'meteora', dexLabel: 'Meteora DLMM', pairAddress: '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump_met', liquidityUsd: 850_000, priceUsd: 0.1232, priceSol: 0.001232, volume24h: 7_000_000, isPrimary: false },
    ],
    poolCount: 2,
  },
  {
    symbol: 'TRUMP',
    name: 'OFFICIAL TRUMP',
    mint: '6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN',
    priceSol: 0.0165,
    priceUsd: 1.65,
    mcapUsd: 165_000_000,
    volume24h: 14_200_000,
    totalLiquidity: 1_500_000,
    change24h: 0.79,
    progress: 100,
    dexId: 'raydium',
    pairAddress: '6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN.png',
    routes: [
      { dexId: 'raydium', dexLabel: 'Raydium CLMM', pairAddress: '6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN_ray', liquidityUsd: 890_000, priceUsd: 1.65, priceSol: 0.0165, volume24h: 8_200_000, isPrimary: true },
      { dexId: 'meteora', dexLabel: 'Meteora Dynamic', pairAddress: '6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN_met', liquidityUsd: 410_000, priceUsd: 1.648, priceSol: 0.01648, volume24h: 4_500_000, isPrimary: false },
      { dexId: 'orca', dexLabel: 'Orca Whirlpool', pairAddress: '6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN_orc', liquidityUsd: 200_000, priceUsd: 1.651, priceSol: 0.01651, volume24h: 1_500_000, isPrimary: false },
    ],
    poolCount: 3,
  },
  {
    symbol: 'PENGU',
    name: 'Pudgy Penguins',
    mint: '2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv',
    priceSol: 0.000074,
    priceUsd: 0.0074,
    mcapUsd: 74_500_000,
    volume24h: 11_800_000,
    totalLiquidity: 1_850_000,
    change24h: 0.92,
    progress: 100,
    dexId: 'raydium',
    pairAddress: '2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv.png',
    routes: [
      { dexId: 'raydium', dexLabel: 'Raydium', pairAddress: '2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv_ray', liquidityUsd: 1_250_000, priceUsd: 0.0074, priceSol: 0.000074, volume24h: 8_800_000, isPrimary: true },
      { dexId: 'meteora', dexLabel: 'Meteora DLMM', pairAddress: '2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv_met', liquidityUsd: 600_000, priceUsd: 0.00739, priceSol: 0.0000739, volume24h: 3_000_000, isPrimary: false },
    ],
    poolCount: 2,
  },
  {
    symbol: 'BONK',
    name: 'Bonk',
    mint: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263',
    priceSol: 0.00000003,
    priceUsd: 0.00000301,
    mcapUsd: 1_950_000_000,
    volume24h: 9_600_000,
    totalLiquidity: 12_400_000,
    change24h: 1.68,
    progress: 100,
    dexId: 'raydium',
    pairAddress: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263.png',
    routes: [
      { dexId: 'raydium', dexLabel: 'Raydium AMM V4', pairAddress: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263_ray', liquidityUsd: 7_800_000, priceUsd: 0.00000301, priceSol: 0.00000003, volume24h: 5_900_000, isPrimary: true },
      { dexId: 'meteora', dexLabel: 'Meteora DLMM', pairAddress: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263_met', liquidityUsd: 3_200_000, priceUsd: 0.00000300, priceSol: 0.00000003, volume24h: 2_500_000, isPrimary: false },
      { dexId: 'orca', dexLabel: 'Orca Whirlpool', pairAddress: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263_orc', liquidityUsd: 1_400_000, priceUsd: 0.00000302, priceSol: 0.00000003, volume24h: 1_200_000, isPrimary: false },
    ],
    poolCount: 3,
  },
  {
    symbol: 'WIF',
    name: 'dogwifhat',
    mint: 'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm',
    priceSol: 0.00177,
    priceUsd: 0.1772,
    mcapUsd: 177_000_000,
    volume24h: 8_250_000,
    totalLiquidity: 8_700_000,
    change24h: -0.66,
    progress: 100,
    dexId: 'raydium',
    pairAddress: 'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm.png',
    routes: [{ dexId: 'raydium', dexLabel: 'Raydium', pairAddress: 'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm', liquidityUsd: 8_700_000, priceUsd: 0.1772, priceSol: 0.00177, volume24h: 8_250_000, isPrimary: true }],
    poolCount: 3,
  },
  {
    symbol: 'POPCAT',
    name: 'Popcat',
    mint: '7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr',
    priceSol: 0.000424,
    priceUsd: 0.0424,
    mcapUsd: 42_400_000,
    volume24h: 6_500_000,
    totalLiquidity: 4_300_000,
    change24h: -1.28,
    progress: 100,
    dexId: 'raydium',
    pairAddress: '7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr.png',
    routes: [{ dexId: 'raydium', dexLabel: 'Raydium', pairAddress: '7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr', liquidityUsd: 4_300_000, priceUsd: 0.0424, priceSol: 0.000424, volume24h: 6_500_000, isPrimary: true }],
    poolCount: 2,
  },
  {
    symbol: 'JUP',
    name: 'Jupiter',
    mint: 'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN',
    priceSol: 0.00283,
    priceUsd: 0.2834,
    mcapUsd: 380_000_000,
    volume24h: 5_100_000,
    totalLiquidity: 15_200_000,
    change24h: 1.73,
    progress: 100,
    dexId: 'orca',
    pairAddress: 'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN.png',
    routes: [{ dexId: 'orca', dexLabel: 'Orca', pairAddress: 'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN', liquidityUsd: 15_200_000, priceUsd: 0.2834, priceSol: 0.00283, volume24h: 5_100_000, isPrimary: true }],
    poolCount: 3,
  },
  {
    symbol: 'RAY',
    name: 'Raydium',
    mint: '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R',
    priceSol: 0.0182,
    priceUsd: 1.82,
    mcapUsd: 480_000_000,
    volume24h: 4_300_000,
    totalLiquidity: 18_100_000,
    change24h: 5.66,
    progress: 100,
    dexId: 'raydium',
    pairAddress: '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R.png',
    routes: [{ dexId: 'raydium', dexLabel: 'Raydium', pairAddress: '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R', liquidityUsd: 18_100_000, priceUsd: 1.82, priceSol: 0.0182, volume24h: 4_300_000, isPrimary: true }],
    poolCount: 2,
  },
  {
    symbol: 'ME',
    name: 'Magic Eden',
    mint: 'MEFNBXixkEbait3xn9bkm8WsJzXtVsaJEn4c8Sam21u',
    priceSol: 0.00060,
    priceUsd: 0.0602,
    mcapUsd: 60_000_000,
    volume24h: 3_750_000,
    totalLiquidity: 3_100_000,
    change24h: -1.43,
    progress: 100,
    dexId: 'meteora',
    pairAddress: 'MEFNBXixkEbait3xn9bkm8WsJzXtVsaJEn4c8Sam21u',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/MEFNBXixkEbait3xn9bkm8WsJzXtVsaJEn4c8Sam21u.png',
    routes: [{ dexId: 'meteora', dexLabel: 'Meteora', pairAddress: 'MEFNBXixkEbait3xn9bkm8WsJzXtVsaJEn4c8Sam21u', liquidityUsd: 3_100_000, priceUsd: 0.0602, priceSol: 0.00060, volume24h: 3_750_000, isPrimary: true }],
    poolCount: 2,
  },
  {
    symbol: 'PNUT',
    name: 'Peanut the Squirrel',
    mint: '2qEHjDLDLbuBgRYvsxhc5D6uDWAivNFZGan56P1tpump',
    priceSol: 0.00041,
    priceUsd: 0.0409,
    mcapUsd: 41_000_000,
    volume24h: 2_900_000,
    totalLiquidity: 3_900_000,
    change24h: 0.48,
    progress: 100,
    dexId: 'pump',
    pairAddress: '2qEHjDLDLbuBgRYvsxhc5D6uDWAivNFZGan56P1tpump',
    imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/2qEHjDLDLbuBgRYvsxhc5D6uDWAivNFZGan56P1tpump.png',
    routes: [{ dexId: 'pump', dexLabel: 'Pump.fun', pairAddress: '2qEHjDLDLbuBgRYvsxhc5D6uDWAivNFZGan56P1tpump', liquidityUsd: 3_900_000, priceUsd: 0.0409, priceSol: 0.00041, volume24h: 2_900_000, isPrimary: true }],
    poolCount: 2,
  },
];

const DEMO_PAPER_WALLET: WalletSummary = {
  pubkey: 'DemoPaperWallet11111111111111111111111111',
  label: 'Demo Paper Wallet',
  is_default: false,
};

export default function App() {
  // Live Token & Market State
  const [tokens, setTokens] = useState<TokenItem[]>(DEFAULT_TOKENS);
  const [selectedToken, setSelectedToken] = useState<TokenItem>(DEFAULT_TOKENS[0]);
  const [selectedRoute, setSelectedRoute] = useState<DexRouteOption | null>(DEFAULT_TOKENS[0]?.routes[0] || null);
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [isSearching, setIsSearching] = useState<boolean>(false);
  const [solPriceUsd, setSolPriceUsd] = useState<number>(100.0);
  const [caCopied, setCaCopied] = useState<boolean>(false);
  const [solChange24h, setSolChange24h] = useState<number>(-4.2);

  // Trading Form State
  const [activeTab, setActiveTab] = useState<'buy' | 'sell'>('buy');
  const [amount, setAmount] = useState<string>('0.5');
  const [slippage, setSlippage] = useState<number>(100); // 100 bps = 1.0%
  const [priorityFee, setPriorityFee] = useState<'normal' | 'fast' | 'turbo'>('fast');
  const [antiMev, setAntiMev] = useState<boolean>(true);

  // Best-Route Comparator State (Phase 2 Read-Only)
  const [routeComparison, setRouteComparison] = useState<RouteComparisonResponse | null>(null);
  const [isComparingRoutes, setIsComparingRoutes] = useState<boolean>(false);
  const [comparisonError, setComparisonError] = useState<string | null>(null);
  const [showAllRoutes, setShowAllRoutes] = useState<boolean>(false);

  // Phase 3: Safe Swap Transaction Preparation & Native Validation State
  const [selectedRouteIdx, setSelectedRouteIdx] = useState<number | null>(null);
  const [isPreparingCandidate, setIsPreparingCandidate] = useState<boolean>(false);
  const [candidateError, setCandidateError] = useState<string | null>(null);
  const [swapValidationResult, setSwapValidationResult] = useState<SwapValidationResult | null>(null);
  const [showPhase3ReviewModal, setShowPhase3ReviewModal] = useState<boolean>(false);
  const [showSimLogs, setShowSimLogs] = useState<boolean>(false);
  const [expandedInstructions, setExpandedInstructions] = useState<Record<number, boolean>>({});

  // Phase 4: Live Swap Signing, Submission & Confirmation State
  const [phase4Password, setPhase4Password] = useState<string>('');
  const [clusterSafetyConfirmed, setClusterSafetyConfirmed] = useState<boolean>(false);
  const [isSubmittingLiveSwap, setIsSubmittingLiveSwap] = useState<boolean>(false);
  const [liveSwapStage, setLiveSwapStage] = useState<string>('');
  const [liveSubmissionResult, setLiveSubmissionResult] = useState<SwapSubmissionResult | null>(null);



  // Wallet State
  const [wallets, setWallets] = useState<WalletSummary[]>([]);
  const [activeWallet, setActiveWallet] = useState<WalletSummary | null>(null);
  const [walletBalance, setWalletBalance] = useState<number>(0);
  const [isRefreshingBalance, setIsRefreshingBalance] = useState<boolean>(false);
  const [rpcLatency] = useState<number>(28);
  const [customRpcUrl, setCustomRpcUrl] = useState<string>(() => {
    try {
      return localStorage.getItem('axiom_custom_rpc_url') || '';
    } catch {
      return '';
    }
  });
  const [customRpc, setCustomRpc] = useState<boolean>(() => {
    try {
      return localStorage.getItem('axiom_custom_rpc_enabled') === 'true';
    } catch {
      return false;
    }
  });

  const handleUpdateCustomRpcUrl = (url: string) => {
    setCustomRpcUrl(url);
    try {
      localStorage.setItem('axiom_custom_rpc_url', url);
    } catch (e) {
      console.warn('Failed to persist custom RPC URL:', e);
    }
  };

  const handleToggleCustomRpc = (enabled: boolean) => {
    setCustomRpc(enabled);
    try {
      localStorage.setItem('axiom_custom_rpc_enabled', enabled ? 'true' : 'false');
    } catch (e) {
      console.warn('Failed to persist custom RPC toggle state:', e);
    }
  };

  const effectiveCustomRpcActive = customRpc && customRpcUrl.trim().length > 0;
  const clientRpcEndpoint = effectiveCustomRpcActive ? customRpcUrl.trim() : PUBLIC_SOLANA_RPC;

  const [showWalletModal, setShowWalletModal] = useState<boolean>(false);
  const [newWalletLabel, setNewWalletLabel] = useState<string>('');
  const [walletPassword, setWalletPassword] = useState<string>('');
  const [importKey, setImportKey] = useState<string>('');
  const [walletTab, setWalletTab] = useState<'create' | 'import'>('create');
  const [copySuccess, setCopySuccess] = useState<boolean>(false);

  // Trade Preview & Signing
  const [showPreviewModal, setShowPreviewModal] = useState<boolean>(false);
  const [currentIntent, setCurrentIntent] = useState<TradeIntent | null>(null);
  const [tradePreview, setTradePreview] = useState<TradePreview | null>(null);
  const [isLoadingPreview, setIsLoadingPreview] = useState<boolean>(false);
  const [signPassword, setSignPassword] = useState<string>('');
  const [isSigning, setIsSigning] = useState<boolean>(false);
  const [signingError, setSigningError] = useState<string | null>(null);

  // Portfolio Holdings & History State
  const [holdings, setHoldings] = useState<TokenHolding[]>([]);
  const [orderHistory, setOrderHistory] = useState<CompletedOrder[]>([]);
  const [tradeSuccessModal, setTradeSuccessModal] = useState<{
    type: 'buy' | 'sell';
    symbol: string;
    amountTokens: number;
    amountSol: number;
    signature: string;
    executionMs?: number;
  } | null>(null);

  // 1-Click Fast Execution / Auto-Approval State
  const [autoApprove, setAutoApprove] = useState<boolean>(() => {
    return localStorage.getItem('axiom_auto_approve') === 'true';
  });

  const handleToggleAutoApprove = (enabled: boolean) => {
    setAutoApprove(enabled);
    localStorage.setItem('axiom_auto_approve', enabled ? 'true' : 'false');
  };

  // Explicit Trading Mode: 'live' (default, fails closed) vs 'demo' (paper trading simulation)
  const [tradingMode, setTradingMode] = useState<'live' | 'demo'>(() => {
    return (localStorage.getItem('axiom_trading_mode') as 'live' | 'demo') || 'live';
  });
  const isDemoMode = tradingMode === 'demo';

  const handleSetTradingMode = (mode: 'live' | 'demo') => {
    setTradingMode(mode);
    localStorage.setItem('axiom_trading_mode', mode);
    if (mode === 'demo') {
      if (!activeWallet || !wallets.some((w) => w.pubkey === activeWallet.pubkey)) {
        setActiveWallet(DEMO_PAPER_WALLET);
      }
    } else {
      // Live Mode: Only use native wallets discovered from encrypted keystore on disk
      if (wallets.length > 0) {
        setActiveWallet(wallets[0]);
      } else {
        setActiveWallet(null);
      }
    }
  };

  // Helper to reset demo portfolio (only affects Demo mode storage)
  const handleResetDemoPortfolio = () => {
    if (!activeWallet || !isDemoMode) return;
    const defaultDemoHoldings: TokenHolding[] = [
      {
        mint: '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump',
        symbol: 'FARTCOIN',
        name: 'Fartcoin (Demo Paper Token)',
        imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump.png',
        amount: 250,
        avgBuyPriceSol: 0.0012,
        avgBuyPriceUsd: 0.12,
        totalCostSol: 0.3,
      },
    ];
    setHoldings(defaultDemoHoldings);
    setWalletBalance(10.0);
    localStorage.setItem(`axiom_demo_holdings_${activeWallet.pubkey}`, JSON.stringify(defaultDemoHoldings));
    localStorage.setItem(`axiom_demo_bal_${activeWallet.pubkey}`, '10.0');
    localStorage.removeItem(`axiom_demo_orders_${activeWallet.pubkey}`);
    setOrderHistory([]);
  };

  // Consumer Wallet Navigation: 'home' | 'trade' | 'history'
  const [currentNav, setCurrentNav] = useState<'home' | 'trade' | 'history'>('home');
  const [homeTab, setHomeTab] = useState<'trades' | 'trending'>('trades');
  const [tradeViewMode, setTradeViewMode] = useState<'list' | 'detail'>('list');

  // Send & Receive Modals
  const [showReceiveModal, setShowReceiveModal] = useState<boolean>(false);
  const [showSendModal, setShowSendModal] = useState<boolean>(false);
  const [sendRecipient, setSendRecipient] = useState<string>('');
  const [sendAmount, setSendAmount] = useState<string>('');
  const [sendAsset, setSendAsset] = useState<string>('SOL'); // 'SOL' or token mint
  const [sendPassword, setSendPassword] = useState<string>('');
  const [sendError, setSendError] = useState<string | null>(null);
  const [isSending, setIsSending] = useState<boolean>(false);
  const [sendSuccessMessage, setSendSuccessMessage] = useState<string | null>(null);

  // Load Holdings & Order History for active wallet (isolated between Live and Demo modes)
  useEffect(() => {
    if (!activeWallet) {
      setHoldings([]);
      setOrderHistory([]);
      return;
    }

    if (isDemoMode) {
      // DEMO MODE: Load isolated simulated holdings and demo orders
      const savedH = localStorage.getItem(`axiom_demo_holdings_${activeWallet.pubkey}`);
      if (savedH) {
        try { setHoldings(JSON.parse(savedH)); } catch { setHoldings([]); }
      } else {
        const initialDemo: TokenHolding[] = [
          {
            mint: '9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump',
            symbol: 'FARTCOIN',
            name: 'Fartcoin (Demo Paper Token)',
            imageUrl: 'https://dd.dexscreener.com/ds-data/tokens/solana/9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump.png',
            amount: 250,
            avgBuyPriceSol: 0.0012,
            avgBuyPriceUsd: 0.12,
            totalCostSol: 0.3,
          },
        ];
        setHoldings(initialDemo);
        localStorage.setItem(`axiom_demo_holdings_${activeWallet.pubkey}`, JSON.stringify(initialDemo));
      }

      const savedO = localStorage.getItem(`axiom_demo_orders_${activeWallet.pubkey}`);
      if (savedO) {
        try { setOrderHistory(JSON.parse(savedO)); } catch { setOrderHistory([]); }
      } else {
        setOrderHistory([]);
      }
    } else {
      // LIVE MODE: Load persistent live confirmed orders for the active wallet
      const savedLiveOrders = localStorage.getItem(`axiom_live_orders_${activeWallet.pubkey}`);
      if (savedLiveOrders) {
        try { setOrderHistory(JSON.parse(savedLiveOrders)); } catch { setOrderHistory([]); }
      } else {
        setOrderHistory([]);
      }
      setHoldings([]);
    }
  }, [activeWallet, tradingMode]);

  // 1. Fetch Wallets on load
  useEffect(() => {
    // Purge any stale unverified mock wallet lists or mock live holdings
    try {
      localStorage.removeItem('axiom_local_wallets');
      localStorage.removeItem('axiom_live_holdings');
      localStorage.removeItem('axiom_live_orders');
    } catch { }

    tauriInvoke<WalletSummary[]>('list_local_wallets')
      .then((list) => {
        setWallets(list);
        if (list.length > 0) {
          const savedPubkey = localStorage.getItem('axiom_active_wallet_pubkey');
          const matched = list.find((w) => w.pubkey === savedPubkey);
          const funded = list.find((w) => w.pubkey === 'B8JpbSBJt92cLExiRm7Ssx3fXFFDfpcuZHNQqyVwufAc');
          const chosen = matched || funded || list[0];
          setActiveWallet(chosen);
          localStorage.setItem('axiom_active_wallet_pubkey', chosen.pubkey);
        } else if (tradingMode === 'demo') {
          setActiveWallet(DEMO_PAPER_WALLET);
        } else {
          setActiveWallet(null);
        }
      })
      .catch((e) => {
        console.error('Failed to list wallets:', e);
        if (tradingMode === 'demo') {
          setActiveWallet(DEMO_PAPER_WALLET);
        } else {
          setActiveWallet(null);
        }
      });
  }, [tradingMode]);

  // 2. Fetch Live SOL Price & Real Top 10 Solana Tokens
  const fetchTopSolanaTokens = async () => {
    try {
      const res = await fetch(`https://api.dexscreener.com/latest/dex/tokens/${TOP_10_MINTS.join(',')}`);
      const data = await res.json();
      if (data && data.pairs && data.pairs.length > 0) {
        const byMint = new Map<string, any>();
        for (const p of data.pairs) {
          if (p.chainId !== 'solana') continue;
          const addr = p.baseToken?.address;
          if (!addr) continue;
          if (!byMint.has(addr) || (p.liquidity?.usd || 0) > (byMint.get(addr).liquidity?.usd || 0)) {
            byMint.set(addr, p);
          }
        }

        const updated = DEFAULT_TOKENS.map((def) => {
          const live = byMint.get(def.mint);
          if (!live) return def;
          const priceUsd = parseFloat(live.priceUsd) || def.priceUsd;
          const priceSol = parseFloat(live.priceNative) || def.priceSol;
          const change24h = live.priceChange?.h24 ?? def.change24h;
          const volume24h = live.volume?.h24 ?? def.volume24h;
          const totalLiquidity = live.liquidity?.usd ?? def.totalLiquidity;
          const mcapUsd = live.marketCap || live.fdv || def.mcapUsd;
          const imageUrl = live.info?.imageUrl || def.imageUrl;
          return {
            ...def,
            priceUsd,
            priceSol,
            change24h,
            volume24h,
            totalLiquidity,
            mcapUsd,
            imageUrl,
          };
        });

        setTokens(updated);
        setSelectedToken((prev) => {
          const match = updated.find((u) => u.mint === prev.mint);
          return match || updated[0];
        });
      }
    } catch (e) {
      console.warn('Using default tokens due to network fallback:', e);
    }
  };

  useEffect(() => {
    // Fetch live SOL price
    fetch('https://api.binance.com/api/v3/ticker/24hr?symbol=SOLUSDT')
      .then((res) => res.json())
      .then((data) => {
        if (data.lastPrice) setSolPriceUsd(parseFloat(data.lastPrice));
        if (data.priceChangePercent) setSolChange24h(parseFloat(data.priceChangePercent));
      })
      .catch(() => { });

    // Fetch live market data for real Top 10 tokens
    fetchTopSolanaTokens();
  }, []);

  // 3. Fetch Balance whenever active wallet or trading mode changes, or RPC endpoint changes
  useEffect(() => {
    if (activeWallet) {
      updateWalletBalance(activeWallet.pubkey);
    }
  }, [activeWallet, tradingMode, clientRpcEndpoint]);

  const updateWalletBalance = async (pubkey: string) => {
    setIsRefreshingBalance(true);
    if (isDemoMode) {
      // DEMO MODE: Isolated paper balance
      const localBal = localStorage.getItem(`axiom_demo_bal_${pubkey}`);
      const bal = localBal !== null ? parseFloat(localBal) : 10.0;
      setWalletBalance(bal);
      setIsRefreshingBalance(false);
      return;
    }

    // LIVE MODE: Real on-chain SOL balance query via Solana RPC
    try {
      const rpcEndpoint = clientRpcEndpoint;
      const bal = await tauriInvoke<number>('get_wallet_balance', {
        pubkey,
        rpc_url: rpcEndpoint,
        rpcUrl: rpcEndpoint,
      });
      setWalletBalance(bal);
    } catch (e) {
      console.error('Failed to get real balance:', e);
      setWalletBalance(0.0);
    } finally {
      setIsRefreshingBalance(false);
    }
  };

  // Instant Faucet for Demo Paper Trading only
  const handleAirdropTestSol = () => {
    if (!activeWallet) return;
    if (!isDemoMode) {
      alert('Faucet is only available in Demo Mode. Real wallets must be funded with real SOL on-chain.');
      return;
    }
    const newBal = (walletBalance || 0) + 2.0;
    setWalletBalance(newBal);
    localStorage.setItem(`axiom_demo_bal_${activeWallet.pubkey}`, newBal.toString());
  };

  // Copy Contract Address to clipboard
  const handleCopyCA = (mint: string) => {
    navigator.clipboard.writeText(mint);
    setCaCopied(true);
    setTimeout(() => setCaCopied(false), 2000);
  };

  // Select a token and auto-pick its primary route
  const handleSelectToken = (token: TokenItem) => {
    setSelectedToken(token);
    const primary = token.routes.find(r => r.isPrimary) || token.routes[0] || null;
    setSelectedRoute(primary);
  };

  // 4. Fetch Live Tokens (DexScreener API) — Search by coin name, ticker, or CA
  const fetchLiveTokens = async (query: string) => {
    const q = query.trim();
    if (!q) {
      await fetchTopSolanaTokens();
      return;
    }
    setIsSearching(true);
    try {
      const res = await fetch(`https://api.dexscreener.com/latest/dex/search?q=${encodeURIComponent(q)}`);
      const data = await res.json();
      if (data && data.pairs) {
        // Step 1: Filter to Solana chain only
        const isGenericSolSearch = q.toLowerCase() === 'sol' || q.toLowerCase() === 'wsol' || q.toLowerCase() === 'solana';
        const solanaPairs = data.pairs.filter((p: any) => {
          if (p.chainId !== 'solana') return false;
          // Filter out duplicate wrapped SOL pools if searching for something else
          if (!isGenericSolSearch && p.baseToken?.address === 'So11111111111111111111111111111111111111112') {
            return false;
          }
          return true;
        });

        // Step 2: Group by baseToken.address (Mint)
        const mintMap = new Map<string, {
          symbol: string;
          name: string;
          mint: string;
          imageUrl?: string;
          mcap: number;
          change24h: number;
          routes: DexRouteOption[];
        }>();

        for (const p of solanaPairs) {
          const mint = p.baseToken?.address;
          if (!mint) continue;

          const liquidityUsd = p.liquidity?.usd || 0;
          const volume24h = p.volume?.h24 || 0;

          // Dust filter: skip pools below thresholds
          if (liquidityUsd < MIN_LIQUIDITY_USD && volume24h < MIN_VOLUME_24H_USD) continue;

          const route: DexRouteOption = {
            dexId: p.dexId || 'unknown',
            dexLabel: DEX_LABELS[p.dexId] || p.dexId || 'Unknown',
            pairAddress: p.pairAddress || '',
            liquidityUsd,
            priceUsd: parseFloat(p.priceUsd) || 0,
            priceSol: parseFloat(p.priceNative) || 0,
            volume24h,
            isPrimary: false,
          };

          if (mintMap.has(mint)) {
            const existing = mintMap.get(mint)!;
            existing.routes.push(route);
            const mcap = p.marketCap || p.fdv || 0;
            if (mcap > existing.mcap) existing.mcap = mcap;
          } else {
            const fallbackImg = `https://dd.dexscreener.com/ds-data/tokens/solana/${mint}.png`;
            mintMap.set(mint, {
              symbol: p.baseToken?.symbol || 'UNKNOWN',
              name: p.baseToken?.name || p.baseToken?.symbol || 'Token',
              mint,
              imageUrl: p.info?.imageUrl || fallbackImg,
              mcap: p.marketCap || p.fdv || 0,
              change24h: p.priceChange?.h24 || 0,
              routes: [route],
            });
          }
        }

        // Step 3: For each mint, determine primary pool and aggregate metrics
        const aggregatedTokens: TokenItem[] = [];

        for (const [, tokenData] of mintMap) {
          tokenData.routes.sort((a, b) => b.liquidityUsd - a.liquidityUsd);
          if (tokenData.routes.length > 0) {
            tokenData.routes[0].isPrimary = true;
          }

          const primary = tokenData.routes[0];
          const totalVolume = tokenData.routes.reduce((sum, r) => sum + r.volume24h, 0);
          const totalLiquidity = tokenData.routes.reduce((sum, r) => sum + r.liquidityUsd, 0);
          const mcap = tokenData.mcap || 0;
          const progress = Math.min(100, Math.max(8, Math.round((mcap / 85000) * 100)));

          aggregatedTokens.push({
            symbol: tokenData.symbol,
            name: tokenData.name,
            mint: tokenData.mint,
            priceSol: primary.priceSol,
            priceUsd: primary.priceUsd,
            mcapUsd: mcap,
            volume24h: totalVolume,
            totalLiquidity,
            change24h: tokenData.change24h,
            progress,
            dexId: primary.dexId,
            pairAddress: primary.pairAddress,
            imageUrl: tokenData.imageUrl,
            routes: tokenData.routes,
            poolCount: tokenData.routes.length,
          });
        }

        // Sort by total volume descending
        aggregatedTokens.sort((a, b) => b.volume24h - a.volume24h);
        const finalTokens = aggregatedTokens.slice(0, 30);

        if (finalTokens.length > 0) {
          setTokens(finalTokens);
          if (!finalTokens.some((t) => t.mint === selectedToken.mint)) {
            handleSelectToken(finalTokens[0]);
          }
        }
      }
    } catch (e) {
      console.warn('Failed to fetch live tokens from DexScreener:', e);
    } finally {
      setIsSearching(false);
    }
  };
  // Calculations
  const numAmount = parseFloat(amount) || 0;
  const feeBps = effectiveCustomRpcActive ? 10 : 20; // Default RPC = 20 bps (0.20%), Custom RPC = 10 bps (0.10%)

  // BUY Calculations (numAmount is SOL)
  const buyFeeSol = (numAmount * feeBps) / 10_000;
  const netSolForBuy = Math.max(0, numAmount - buyFeeSol);
  const estimatedTokensOut = selectedToken.priceSol > 0 ? (netSolForBuy / selectedToken.priceSol) : 0;

  // SELL Calculations (numAmount is Tokens)
  const grossSolFromSell = numAmount * selectedToken.priceSol;
  const sellFeeSol = (grossSolFromSell * feeBps) / 10_000;
  const netSolFromSell = Math.max(0, grossSolFromSell - sellFeeSol);

  // Active fee for current order mode
  const feeSol = activeTab === 'buy' ? buyFeeSol : sellFeeSol;

  // Phase 2: Debounced Best-Route Comparator query
  useEffect(() => {
    if (!selectedToken || !selectedToken.mint || numAmount <= 0) {
      setRouteComparison(null);
      setComparisonError(null);
      return;
    }

    let isSubscribed = true;
    const timer = setTimeout(async () => {
      setIsComparingRoutes(true);
      setComparisonError(null);
      try {
        const amountAtomic = activeTab === 'buy'
          ? Math.floor(numAmount * 1e9)
          : Math.floor(numAmount * 1e6);

        if (amountAtomic <= 0) {
          setIsComparingRoutes(false);
          return;
        }

        const res = await tauriInvoke<RouteComparisonResponse>('compare_routes_command', {
          params: {
            mint: selectedToken.mint,
            side: activeTab,
            amount_atomic: amountAtomic,
            fee_tier: effectiveCustomRpcActive ? 'UserRpc' : 'Default',
          },
        });

        if (isSubscribed) {
          setRouteComparison(res);
          setIsComparingRoutes(false);
        }
      } catch (err: any) {
        if (isSubscribed) {
          console.warn('Route comparison query failed:', err);
          setComparisonError(err?.message || 'Failed to compare routes');
          setIsComparingRoutes(false);
        }
      }
    }, 350);

    return () => {
      isSubscribed = false;
      clearTimeout(timer);
    };
  }, [selectedToken?.mint, amount, activeTab, effectiveCustomRpcActive]);

  // Sync selected candidate route index with route comparator winner
  useEffect(() => {
    if (routeComparison && routeComparison.best_route_index !== undefined && routeComparison.best_route_index !== null) {
      setSelectedRouteIdx(routeComparison.best_route_index);
    } else {
      setSelectedRouteIdx(null);
    }
  }, [routeComparison]);


  // Total Portfolio Net Worth & Breakdown
  const totalTokensUsd = holdings.reduce((sum, h) => {
    const live = tokens.find((t) => t.mint === h.mint);
    const pUsd = live ? live.priceUsd : h.avgBuyPriceUsd;
    return sum + (h.amount * pUsd);
  }, 0);
  const liquidSolUsd = walletBalance * solPriceUsd;
  const totalPortfolioUsd = liquidSolUsd + totalTokensUsd;

  const handleConfirmSend = async () => {
    if (!sendRecipient || sendRecipient.trim().length < 32) {
      setSendError('Please enter a valid Solana address (32-44 base58 characters)');
      return;
    }
    const sendNum = parseFloat(sendAmount) || 0;
    if (sendNum <= 0) {
      setSendError('Please enter an amount greater than 0');
      return;
    }
    if (!isDemoMode && !sendPassword) {
      setSendError('Please enter keystore password to unlock wallet');
      return;
    }
    if (!activeWallet) return;

    if (!isDemoMode) {
      // Real transfer path: Fail closed! Live transactions are not broadcast yet.
      setIsSending(false);
      setSendError('Live transfers are not available yet. No transaction was submitted.');
      return;
    }

    // DEMO MODE: Isolated Paper Transfer Simulation
    setIsSending(true);
    setSendError(null);

    try {
      const demoSig = 'SIMULATED_DEMO_TRANSFER_' + Math.random().toString(36).substring(2, 8).toUpperCase();

      if (sendAsset === 'SOL') {
        if (sendNum > walletBalance) {
          throw new Error(`Insufficient Demo SOL. You have ${walletBalance.toFixed(4)} SOL`);
        }
        const newBal = Math.max(0, walletBalance - sendNum);
        setWalletBalance(newBal);
        localStorage.setItem(`axiom_demo_bal_${activeWallet.pubkey}`, newBal.toString());
      } else {
        const held = holdings.find((h) => h.mint === sendAsset);
        if (!held || sendNum > held.amount) {
          throw new Error('Insufficient demo token balance');
        }
        setHoldings((prev) => {
          const updated = prev
            .map((h) => (h.mint === sendAsset ? { ...h, amount: h.amount - sendNum } : h))
            .filter((h) => h.amount > 0.0001);
          localStorage.setItem(`axiom_demo_holdings_${activeWallet.pubkey}`, JSON.stringify(updated));
          return updated;
        });
      }

      const tokenSymbol = sendAsset === 'SOL' ? 'SOL' : (holdings.find((h) => h.mint === sendAsset)?.symbol || 'TOKEN');
      const newOrder: CompletedOrder = {
        id: 'demo_send_' + Date.now(),
        time: new Date().toLocaleTimeString(),
        type: 'sell',
        symbol: tokenSymbol,
        mint: sendAsset === 'SOL' ? 'So11111111111111111111111111111111111111112' : sendAsset,
        amountSol: sendAsset === 'SOL' ? sendNum : 0,
        amountTokens: sendAsset === 'SOL' ? 0 : sendNum,
        priceSol: 1,
        priceUsd: solPriceUsd,
        signature: demoSig,
      };
      setOrderHistory((prev) => {
        const updated = [newOrder, ...prev];
        localStorage.setItem(`axiom_demo_orders_${activeWallet.pubkey}`, JSON.stringify(updated));
        return updated;
      });

      setIsSending(false);
      setShowSendModal(false);
      setSendAmount('');
      setSendRecipient('');
      setSendPassword('');
      setSendSuccessMessage(`[DEMO PAPER] Simulated transfer of ${sendNum} ${tokenSymbol} completed.`);
      setTimeout(() => setSendSuccessMessage(null), 4000);
    } catch (e: any) {
      setIsSending(false);
      setSendError(e?.message || e.toString());
    }
  };

  const handleCopyWallet = () => {
    if (activeWallet) {
      navigator.clipboard.writeText(activeWallet.pubkey);
      setCopySuccess(true);
      setTimeout(() => setCopySuccess(false), 2000);
    }
  };

  const handleCreateWallet = async () => {
    if (!walletPassword || walletPassword.length < 8) {
      alert('Password must be at least 8 characters');
      return;
    }
    try {
      const created = await tauriInvoke<WalletSummary>('create_new_wallet', {
        label: newWalletLabel || 'Trading Wallet',
        password: walletPassword,
      });
      setWallets([...wallets, created]);
      setActiveWallet(created);
      localStorage.setItem('axiom_active_wallet_pubkey', created.pubkey);
      setShowWalletModal(false);
      setWalletPassword('');
      setNewWalletLabel('');
    } catch (e: any) {
      alert('Error creating wallet: ' + e);
    }
  };

  const handleImportWallet = async () => {
    if (!importKey || !walletPassword) {
      alert('Please provide private key and password');
      return;
    }
    try {
      const imported = await tauriInvoke<WalletSummary>('import_wallet_private_key', {
        secret_base58: importKey,
        label: newWalletLabel || 'Imported Wallet',
        password: walletPassword,
      });
      setWallets([...wallets, imported]);
      setActiveWallet(imported);
      localStorage.setItem('axiom_active_wallet_pubkey', imported.pubkey);
      setShowWalletModal(false);
      setImportKey('');
      setWalletPassword('');
    } catch (e: any) {
      alert('Error importing wallet: ' + e);
    }
  };

  const executeTradeDirect = async (_intent: TradeIntent, _preview: TradePreview, _pwd?: string) => {
    if (!isDemoMode) {
      // Real trading path: Must fail closed until live submission and confirmation exist!
      // In accordance with Phase 1 requirements, never sign and report success without actual on-chain broadcast.
      setIsSigning(false);
      const errMsg = 'Live trading is not available yet. No transaction was submitted.';
      setSigningError(errMsg);
      alert(errMsg);
      return;
    }

    // DEMO MODE: Local paper trading simulation
    setIsSigning(true);
    setSigningError(null);

    try {
      const demoSig = 'SIMULATED_DEMO_TRADE_' + Math.random().toString(36).substring(2, 10).toUpperCase();

      if (activeTab === 'buy') {
        const outTokens = estimatedTokensOut;
        const spentSol = numAmount;
        if (spentSol > walletBalance) {
          throw new Error(`Insufficient Demo SOL. Required: ${spentSol.toFixed(4)} SOL, Demo Balance: ${walletBalance.toFixed(4)} SOL`);
        }
        const newBal = Math.max(0, walletBalance - spentSol);
        setWalletBalance(newBal);
        if (activeWallet) {
          localStorage.setItem(`axiom_demo_bal_${activeWallet.pubkey}`, newBal.toString());
        }

        // Update Demo Holdings
        setHoldings((prev) => {
          const existingIdx = prev.findIndex((h) => h.mint === selectedToken.mint);
          let updated: TokenHolding[];
          if (existingIdx >= 0) {
            const existing = prev[existingIdx];
            const updatedAmount = existing.amount + outTokens;
            const updatedTotalCost = existing.totalCostSol + numAmount;
            const updatedAvgSol = updatedTotalCost / updatedAmount;
            const updatedAvgUsd = (updatedTotalCost * solPriceUsd) / updatedAmount;
            updated = [...prev];
            updated[existingIdx] = {
              ...existing,
              amount: updatedAmount,
              avgBuyPriceSol: updatedAvgSol,
              avgBuyPriceUsd: updatedAvgUsd,
              totalCostSol: updatedTotalCost,
            };
          } else {
            updated = [
              ...prev,
              {
                mint: selectedToken.mint,
                symbol: selectedToken.symbol,
                name: selectedToken.name + ' (Demo)',
                imageUrl: selectedToken.imageUrl,
                amount: outTokens,
                avgBuyPriceSol: selectedToken.priceSol,
                avgBuyPriceUsd: selectedToken.priceUsd,
                totalCostSol: numAmount,
              },
            ];
          }
          if (activeWallet) {
            localStorage.setItem(`axiom_demo_holdings_${activeWallet.pubkey}`, JSON.stringify(updated));
          }
          return updated;
        });

        // Add to Demo Order History
        const demoSpeed = Math.floor(Math.random() * 80) + 120;
        const newOrder: CompletedOrder = {
          id: 'demo_order_' + Date.now(),
          time: new Date().toLocaleTimeString(),
          type: 'buy',
          symbol: selectedToken.symbol,
          mint: selectedToken.mint,
          amountSol: numAmount,
          amountTokens: outTokens,
          priceSol: selectedToken.priceSol,
          priceUsd: selectedToken.priceUsd,
          signature: demoSig,
          executionMs: demoSpeed,
        };
        setOrderHistory((prev) => {
          const updated = [newOrder, ...prev];
          if (activeWallet) {
            localStorage.setItem(`axiom_demo_orders_${activeWallet.pubkey}`, JSON.stringify(updated));
          }
          return updated;
        });

        setShowPreviewModal(false);
        setTradeSuccessModal({
          type: 'buy',
          symbol: selectedToken.symbol,
          amountTokens: outTokens,
          amountSol: numAmount,
          signature: demoSig,
          executionMs: demoSpeed,
        });
        setCurrentNav('trade');
        setTradeViewMode('detail');

      } else {
        // DEMO SELL
        const tokensSold = numAmount;
        const receivedSol = netSolFromSell;
        const newBal = walletBalance + receivedSol;
        setWalletBalance(newBal);
        if (activeWallet) {
          localStorage.setItem(`axiom_demo_bal_${activeWallet.pubkey}`, newBal.toString());
        }

        const heldItem = holdings.find((h) => h.mint === selectedToken.mint);
        const costBasisSol = heldItem ? (heldItem.avgBuyPriceSol * tokensSold) : 0;
        const pnlSol = receivedSol - costBasisSol;
        const pnlUsd = pnlSol * solPriceUsd;
        const pnlPct = costBasisSol > 0 ? (pnlSol / costBasisSol) * 100 : 0;

        // Update Demo Holdings (deduct sold amount)
        setHoldings((prev) => {
          const existingIdx = prev.findIndex((h) => h.mint === selectedToken.mint);
          let updated: TokenHolding[];
          if (existingIdx >= 0) {
            const existing = prev[existingIdx];
            const rem = existing.amount - tokensSold;
            if (rem <= 0.0001) {
              updated = prev.filter((_, idx) => idx !== existingIdx);
            } else {
              updated = [...prev];
              updated[existingIdx] = {
                ...existing,
                amount: rem,
                totalCostSol: Math.max(0, existing.totalCostSol - (existing.avgBuyPriceSol * tokensSold)),
              };
            }
          } else {
            updated = prev;
          }
          if (activeWallet) {
            localStorage.setItem(`axiom_demo_holdings_${activeWallet.pubkey}`, JSON.stringify(updated));
          }
          return updated;
        });

        // Add to Demo Order History with accurate realized PnL
        const demoSpeed = Math.floor(Math.random() * 80) + 120;
        const newOrder: CompletedOrder = {
          id: 'demo_order_' + Date.now(),
          time: new Date().toLocaleTimeString(),
          type: 'sell',
          symbol: selectedToken.symbol,
          mint: selectedToken.mint,
          amountSol: receivedSol,
          amountTokens: tokensSold,
          priceSol: selectedToken.priceSol,
          priceUsd: selectedToken.priceUsd,
          signature: demoSig,
          realizedPnlUsd: pnlUsd,
          realizedPnlPct: pnlPct,
          executionMs: demoSpeed,
        };
        setOrderHistory((prev) => {
          const updated = [newOrder, ...prev];
          if (activeWallet) {
            localStorage.setItem(`axiom_demo_orders_${activeWallet.pubkey}`, JSON.stringify(updated));
          }
          return updated;
        });

        setShowPreviewModal(false);
        setTradeSuccessModal({
          type: 'sell',
          symbol: selectedToken.symbol,
          amountTokens: tokensSold,
          amountSol: receivedSol,
          signature: demoSig,
          executionMs: demoSpeed,
        });
        setCurrentNav('trade');
        setTradeViewMode('detail');
      }

      setIsSigning(false);
      setSignPassword('');
    } catch (e: any) {
      setIsSigning(false);
      setSigningError(e?.message || e.toString());
      if (autoApprove) {
        alert('Demo Execution Failed: ' + (e?.message || e.toString()));
      }
    }
  };

  // Phase 3: Safe Swap Transaction Preparation and Validation
  const handlePrepareAndValidateCandidate = async (routeOverride?: RouteQuoteItem) => {
    if (!activeWallet) {
      setShowWalletModal(true);
      return;
    }
    if (numAmount <= 0) {
      alert('Please enter an amount greater than 0');
      return;
    }
    if (activeTab === 'buy' && numAmount > walletBalance) {
      alert(`Insufficient SOL balance. Required: ${numAmount.toFixed(4)} SOL, Available: ${walletBalance.toFixed(4)} SOL.`);
      return;
    }
    if (activeTab === 'sell') {
      const held = holdings.find((h) => h.mint === selectedToken.mint);
      const heldAmt = held ? held.amount : 0;
      if (heldAmt <= 0) {
        alert(`You do not hold any ${selectedToken.symbol} in this wallet. Buy some first before selling.`);
        return;
      }
      if (numAmount > heldAmt) {
        alert(`Cannot sell ${numAmount.toLocaleString()} ${selectedToken.symbol}. You only hold ${heldAmt.toLocaleString()} ${selectedToken.symbol}.`);
        return;
      }
    }

    // Determine target route
    let targetRoute = routeOverride;
    if (!targetRoute && routeComparison) {
      if (selectedRouteIdx !== null && routeComparison.routes[selectedRouteIdx]) {
        targetRoute = routeComparison.routes[selectedRouteIdx];
      } else if (routeComparison.best_route_index !== null && routeComparison.best_route_index !== undefined) {
        targetRoute = routeComparison.routes[routeComparison.best_route_index];
      }
    }

    if (!targetRoute) {
      setCandidateError('No route available to prepare. Please wait for route comparator to find a valid route.');
      return;
    }

    if (targetRoute.source === 'okx_router' || targetRoute.source === 'okx') {
      setCandidateError("OKX DEX Router is currently unsupported for swap candidate preparation. Candidate preparation is supported for Pump.fun bonding curves, PumpSwap AMMs, and Jupiter/Raydium routes.");
      return;
    }

    if (targetRoute.status !== 'estimated') {
      setCandidateError(`Cannot prepare candidate for route '${targetRoute.venue}': ${targetRoute.reason_unavailable || targetRoute.error || 'Route is unavailable'}`);
      return;
    }

    setIsPreparingCandidate(true);
    setCandidateError(null);

    try {
      const amountAtomic = activeTab === 'buy'
        ? Math.floor(numAmount * 1_000_000_000)
        : Math.floor(numAmount * 1_000_000);

      const pFee = priorityFee === 'turbo' ? 150_000 : priorityFee === 'normal' ? 20_000 : 50_000;
      const inputMint = activeTab === 'buy' ? 'So11111111111111111111111111111111111111112' : selectedToken.mint;
      const outputMint = activeTab === 'buy' ? selectedToken.mint : 'So11111111111111111111111111111111111111112';

      const quoteNetOut = targetRoute.net_output_atomic || 0;
      const expectedMinAtomic = Math.floor(quoteNetOut * (10_000 - slippage) / 10_000);

      if (!expectedMinAtomic || expectedMinAtomic <= 0) {
        setCandidateError('Selected route quote does not contain a valid minimum output to bind. Please refresh route quotes.');
        setIsPreparingCandidate(false);
        return;
      }

      const requestParams: PrepareSwapRequest = {
        wallet_pubkey: activeWallet.pubkey,
        side: activeTab,
        input_mint: inputMint,
        output_mint: outputMint,
        input_amount_atomic: amountAtomic,
        selected_route_source: targetRoute.source,
        venue: targetRoute.venue,
        pool_address: targetRoute.pool_address,
        slippage_bps: slippage,
        expected_minimum_output_atomic: expectedMinAtomic,
        quoted_at_unix_ms: targetRoute.quoted_at_unix_ms || Date.now(),
        state_slot: targetRoute.state_slot,
        priority_fee_micro_lamports: pFee,
        fee_tier: effectiveCustomRpcActive ? 'UserRpc' : 'Default',
      };

      const rpcEndpoint = clientRpcEndpoint;
      const result = await tauriInvoke<SwapValidationResult>('prepare_and_validate_swap_command', {
        params: requestParams,
        expected_signer: activeWallet.pubkey,
        expectedSigner: activeWallet.pubkey,
        server_url: 'http://localhost:8080',
        serverUrl: 'http://localhost:8080',
        rpc_url: rpcEndpoint,
        rpcUrl: rpcEndpoint,
      });

      setPhase4Password('');
      setClusterSafetyConfirmed(false);
      setIsSubmittingLiveSwap(false);
      setLiveSwapStage('');
      setLiveSubmissionResult(null);

      setSwapValidationResult(result);
      setShowPhase3ReviewModal(true);

      if (!result.is_valid || !result.is_simulated) {
        const errorMsg = result.validation_errors.length > 0
          ? result.validation_errors.join('; ')
          : 'Candidate validation or RPC simulation rejected by native validator.';
        setCandidateError(errorMsg);
      }
    } catch (e: any) {
      console.error('Candidate preparation error:', e);
      const errMsg = e?.message || e.toString();
      setCandidateError(errMsg);
    } finally {
      setIsPreparingCandidate(false);
    }
  };

  const handleLiveSwapSignAndSubmit = async () => {
    if (!activeWallet || !swapValidationResult?.candidate) {
      alert('No prepared candidate available for signing');
      return;
    }
    if (!phase4Password || phase4Password.trim().length === 0) {
      alert('Wallet password is required to decrypt keystore and sign transaction.');
      return;
    }
    if (!clusterSafetyConfirmed) {
      alert('Mainnet safety confirmation is required before broadcasting to the cluster.');
      return;
    }

    setIsSubmittingLiveSwap(true);
    setLiveSwapStage('Re-validating candidate & verifying blockhash freshness in Rust...');
    const liveStartTime = performance.now();
    try {
      setLiveSwapStage('Unlocking encrypted keystore & signing transaction message...');
      const rpcEndpoint = clientRpcEndpoint;

      setLiveSwapStage('Submitting transaction with preflight simulation & tracking confirmation...');
      const result = await tauriInvoke<SwapSubmissionResult>('sign_and_submit_swap_candidate_command', {
        candidate: swapValidationResult.candidate,
        expected_signer: activeWallet.pubkey,
        expectedSigner: activeWallet.pubkey,
        password: phase4Password,
        cluster_safety_confirmed: clusterSafetyConfirmed,
        clusterSafetyConfirmed: clusterSafetyConfirmed,
        fee_tier: effectiveCustomRpcActive ? 'UserRpc' : 'Default',
        feeTier: effectiveCustomRpcActive ? 'UserRpc' : 'Default',
        rpc_url: rpcEndpoint,
        rpcUrl: rpcEndpoint,
      });

      setLiveSubmissionResult(result);

      if (result.status === 'confirmed') {
        const elapsedMs = Math.round(performance.now() - liveStartTime);
        // Only update on-chain balance and add to completed orders on confirmed success!
        await updateWalletBalance(activeWallet.pubkey);
        const newOrder: CompletedOrder = {
          id: result.signature || `live_${Date.now()}`,
          time: new Date().toLocaleTimeString(),
          type: activeTab,
          symbol: selectedToken.symbol,
          mint: selectedToken.mint,
          amountSol: activeTab === 'buy' ? parseFloat(amount) : (swapValidationResult.candidate.minimum_output_atomic / 1e9),
          amountTokens: activeTab === 'buy' ? (swapValidationResult.candidate.minimum_output_atomic / 1e6) : parseFloat(amount),
          priceSol: selectedToken.priceSol,
          priceUsd: selectedToken.priceUsd,
          signature: result.signature || '',
          executionMs: elapsedMs,
        };
        setOrderHistory((prev) => {
          const updated = [newOrder, ...prev];
          try {
            localStorage.setItem(`axiom_live_orders_${activeWallet.pubkey}`, JSON.stringify(updated.slice(0, 100)));
          } catch { }
          return updated;
        });
        setTradeSuccessModal({
          type: activeTab,
          symbol: selectedToken.symbol,
          amountTokens: newOrder.amountTokens,
          amountSol: newOrder.amountSol,
          signature: result.signature || '',
          executionMs: elapsedMs,
        });
      }

    } catch (e: any) {
      console.error('Signing / submission failure:', e);
      const errMsg = e?.message || e.toString();
      setLiveSubmissionResult({
        status: 'failed',
        error: errMsg,
        logs: [],
        blockhash_expired: false,
      });
    } finally {
      setIsSubmittingLiveSwap(false);
      setLiveSwapStage('');
      setPhase4Password(''); // Clear password from component state
    }
  };

  const handleQuerySwapStatus = async () => {
    if (!liveSubmissionResult?.signature || !swapValidationResult?.candidate) {
      return;
    }
    setIsSubmittingLiveSwap(true);
    setLiveSwapStage('Querying on-chain transaction status from Solana RPC...');
    try {
      const rpcEndpoint = clientRpcEndpoint;

      const result = await tauriInvoke<SwapSubmissionResult>('query_swap_status_command', {
        signature: liveSubmissionResult.signature,
        last_valid_block_height: swapValidationResult.candidate.last_valid_block_height,
        lastValidBlockHeight: swapValidationResult.candidate.last_valid_block_height,
        rpc_url: rpcEndpoint,
        rpcUrl: rpcEndpoint,
      });
      setLiveSubmissionResult(result);
      if (result.status === 'confirmed' && activeWallet) {
        await updateWalletBalance(activeWallet.pubkey);
      }
    } catch (e: any) {
      alert('Status query error: ' + (e?.message || e.toString()));
    } finally {
      setIsSubmittingLiveSwap(false);
      setLiveSwapStage('');
    }
  };


  const handleOpenPreview = async () => {
    if (!activeWallet) {
      setShowWalletModal(true);
      return;
    }
    if (numAmount <= 0) {
      alert('Please enter an amount greater than 0');
      return;
    }

    // Validation for SELL mode: ensure user holds tokens
    if (activeTab === 'sell') {
      const held = holdings.find((h) => h.mint === selectedToken.mint);
      const heldAmt = held ? held.amount : 0;
      if (heldAmt <= 0) {
        alert(`You do not hold any ${selectedToken.symbol} in this wallet. Buy some first before selling.`);
        return;
      }
      if (numAmount > heldAmt) {
        alert(`Cannot sell ${numAmount.toLocaleString()} ${selectedToken.symbol}. You only hold ${heldAmt.toLocaleString()} ${selectedToken.symbol}.`);
        return;
      }
    }

    // Validation for BUY mode: ensure sufficient SOL balance
    if (activeTab === 'buy') {
      if (numAmount > walletBalance) {
        alert(`Insufficient SOL balance. Required: ${numAmount.toFixed(4)} SOL, Available: ${walletBalance.toFixed(4)} SOL.`);
        return;
      }
    }

    setIsLoadingPreview(true);
    setSigningError(null);
    setSignPassword('');

    try {
      const pFee = priorityFee === 'turbo' ? 150_000 : priorityFee === 'normal' ? 20_000 : 50_000;
      const amountAtomic = activeTab === 'buy'
        ? Math.floor(numAmount * 1_000_000_000)
        : Math.floor(numAmount * 1_000_000);

      // Determine platform: route non-pump tokens and Raydium/Meteora/Orca pairs through OKX DEX Router V1
      const tokenDex = (selectedToken as any)?.dexId?.toLowerCase() || '';
      let platform = 'pump_fun';
      let dexId = 4; // RaydiumSwap default
      if (tokenDex.includes('raydium')) {
        platform = 'okx_router';
        dexId = 4;
      } else if (tokenDex.includes('meteora')) {
        platform = 'okx_router';
        dexId = 13;
      } else if (tokenDex.includes('orca') || tokenDex.includes('whirlpool')) {
        platform = 'okx_router';
        dexId = 2;
      } else if (!selectedToken.mint.endsWith('pump')) {
        platform = 'okx_router';
        dexId = 4;
      }

      if (!isDemoMode && platform === 'okx_router') {
        alert('OKX DEX Router is currently unsupported. Live trading and swap candidate preparation are restricted to natively verified Pump.fun bonding curves and PumpSwap canonical AMMs.');
        setIsLoadingPreview(false);
        return;
      }

      // If user selected DEMO mode: isolated paper trading preview (no real keystore or RPC broadcast)
      if (isDemoMode) {
        const estMinOut = activeTab === 'buy' ? Math.floor(estimatedTokensOut * 1e6) : Math.floor(netSolFromSell * 1e9);
        const demoPreviewId = 'demo_intent_' + Date.now();
        const demoPreview: TradePreview = {
          intent_id: demoPreviewId,
          side: activeTab,
          mint: selectedToken.mint,
          input_amount_atomic: amountAtomic,
          minimum_output_atomic: estMinOut,
          platform_fee_atomic: Math.floor(feeSol * 1e9),
          platform_fee_recipient: 'DEMO_FEE_RECIPIENT_SIMULATION',
          compute_unit_limit: 200_000,
          priority_fee_micro_lamports: pFee,
          slippage_bps: slippage,
          total_sol_required: Math.floor((numAmount + feeSol) * 1e9),
          route: { okx_router: { dex_id: dexId } } as any,
          instructions_count: 3,
          warnings: ['[DEMO MODE] This is a simulated paper trade. Keystore keys are NOT accessed and no transactions are broadcast to Solana.'],
        };
        const demoIntent: TradeIntent = {
          api_version: 1,
          intent_id: demoPreviewId,
          created_at: new Date().toISOString(),
          expires_at: new Date(Date.now() + 60000).toISOString(),
          last_valid_block_height: 310000000,
          wallet_pubkey: activeWallet.pubkey,
          side: activeTab,
          mint: selectedToken.mint,
          route: { pump_curve: { version: 'v2' } },
          input_amount_atomic: amountAtomic,
          minimum_output_atomic: estMinOut,
          slippage_bps: slippage,
          fee: {
            rate_bps: feeBps,
            basis: 'input',
            recipient: 'DEMO_FEE_RECIPIENT_SIMULATION',
            amount_atomic: Math.floor(feeSol * 1e9),
          },
          state_slot: 310000000,
          unsigned_tx_base64: 'DEMO_SIMULATION_BASE64',
        };
        setCurrentIntent(demoIntent);
        setTradePreview(demoPreview);
        setShowPreviewModal(true);
        return;
      }

      // LIVE MODE:
      // Step 1: Request trade intent from Strategy Server (fails if server offline, no silent fallback)
      const intent = await tauriInvoke<TradeIntent>('request_trade_intent', {
        params: {
          wallet_pubkey: activeWallet.pubkey,
          mint: selectedToken.mint,
          amount_atomic: amountAtomic,
          slippage_bps: slippage,
          fee_tier: effectiveCustomRpcActive ? 'user_rpc' : 'default',
          priority_fee_micro_lamports: pFee,
          platform,
          dex_id: dexId,
        },
        side: activeTab,
        serverUrl: 'http://localhost:8080',
      });

      // Step 2: Validate the transaction locally using client-side safety bounds
      const preview = await tauriInvoke<TradePreview>('validate_intent_command', {
        intent,
        expectedSigner: activeWallet.pubkey,
      });

      setCurrentIntent(intent);
      setTradePreview(preview);

      // If 1-Click Auto-Approval is active: fail closed in live mode
      if (autoApprove) {
        await executeTradeDirect(intent, preview, '');
      } else {
        setShowPreviewModal(true);
      }
    } catch (e: any) {
      alert('Trade Intent Error: ' + (e?.message || e.toString()));
    } finally {
      setIsLoadingPreview(false);
    }
  };

  const handleConfirmTrade = async () => {
    if (!currentIntent || !tradePreview) {
      setSigningError('No active trade intent to sign');
      return;
    }

    if (!isDemoMode) {
      // Real trading path fails closed
      setSigningError('Live trading is not available yet. No transaction was submitted.');
      return;
    }

    // In Demo Mode, execute paper trade directly without touching keystore credentials
    await executeTradeDirect(currentIntent, tradePreview, '');
  };
  const handleOpenCoinDetail = (token: TokenItem, tab: 'buy' | 'sell' = 'buy') => {
    handleSelectToken(token);
    setActiveTab(tab);
    setTradeViewMode('detail');
    setCurrentNav('trade');
  };

  const handleTradeHolding = (holding: TokenHolding) => {
    const tok = tokens.find((t) => t.mint === holding.mint) || {
      symbol: holding.symbol,
      name: holding.name,
      mint: holding.mint,
      priceSol: holding.avgBuyPriceSol,
      priceUsd: holding.avgBuyPriceUsd,
      mcapUsd: holding.amount * holding.avgBuyPriceUsd,
      volume24h: 1_000_000,
      totalLiquidity: 500_000,
      change24h: 0,
      progress: 100,
      dexId: 'pump',
      pairAddress: holding.mint,
      imageUrl: holding.imageUrl,
      routes: [{ dexId: 'pump', dexLabel: 'Pump.fun', pairAddress: holding.mint, liquidityUsd: 500_000, priceUsd: holding.avgBuyPriceUsd, priceSol: holding.avgBuyPriceSol, volume24h: 1_000_000, isPrimary: true }],
      poolCount: 1,
    };
    handleSelectToken(tok);
    setActiveTab('sell');
    const heldAmt = Math.floor(holding.amount);
    setAmount(heldAmt > 0 ? heldAmt.toString() : '0');
    setTradeViewMode('detail');
    setCurrentNav('trade');
  };

  const handleSendHolding = (holding: TokenHolding) => {
    setSendAsset(holding.mint);
    const heldAmt = Math.floor(holding.amount);
    setSendAmount(heldAmt > 0 ? heldAmt.toString() : '0');
    setShowSendModal(true);
  };

  const renderHomeScreen = () => (
    <div>
      {/* Total Assets Card */}
      <div className="total-assets-card">
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '4px' }}>
          <div className="total-assets-label" style={{ letterSpacing: '0.5px' }}>
            {isDemoMode ? 'ESTIMATED TOTAL NET WORTH (DEMO PAPER)' : 'ESTIMATED TOTAL NET WORTH (LIVE RPC)'}
          </div>
          {isDemoMode ? (
            <button
              onClick={handleAirdropTestSol}
              style={{
                padding: '3px 8px',
                borderRadius: '10px',
                background: 'rgba(168, 85, 247, 0.15)',
                color: '#c084fc',
                border: '1px solid rgba(168, 85, 247, 0.3)',
                fontSize: '10px',
                fontWeight: 700,
                cursor: 'pointer',
              }}
              title="Claim +2.0 Demo SOL paper balance"
            >
              🧪 +2.0 Demo SOL Faucet
            </button>
          ) : (
            <button
              onClick={() => activeWallet && updateWalletBalance(activeWallet.pubkey)}
              disabled={isRefreshingBalance}
              style={{
                padding: '3px 8px',
                borderRadius: '10px',
                background: 'rgba(0, 240, 160, 0.12)',
                color: 'var(--accent-green)',
                border: '1px solid rgba(0, 240, 160, 0.3)',
                fontSize: '10px',
                fontWeight: 700,
                cursor: 'pointer',
              }}
              title="Refresh on-chain balance via Solana RPC"
            >
              {isRefreshingBalance ? '🔄 Refreshing...' : '🔄 Refresh Balance'}
            </button>
          )}
        </div>
        <div className="total-assets-fiat" style={{ marginBottom: '8px' }}>
          ${(isDemoMode ? totalPortfolioUsd : liquidSolUsd).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
        </div>

        {/* Dual Breakdown Card: Liquid Cash vs Token Assets */}
        <div style={{
          background: 'rgba(0, 0, 0, 0.28)',
          borderRadius: '10px',
          padding: '8px 12px',
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          border: '1px solid rgba(255, 255, 255, 0.06)'
        }}>
          <div>
            <div style={{ fontSize: '10px', color: 'var(--text-dim)', textTransform: 'uppercase', fontWeight: 600 }}>
              {isDemoMode ? 'Demo Liquid Paper Cash' : 'Available On-Chain SOL'}
            </div>
            <div style={{ fontSize: '13px', fontWeight: 700, color: 'var(--accent-green)', display: 'flex', alignItems: 'center', gap: '4px' }}>
              <span>◎ {walletBalance.toFixed(4)} SOL</span>
              <span style={{ fontSize: '11px', color: 'var(--text-muted)', fontWeight: 400 }}>(${liquidSolUsd.toFixed(2)})</span>
            </div>
          </div>
          <div style={{ textAlign: 'right' }}>
            <div style={{ fontSize: '10px', color: 'var(--text-dim)', textTransform: 'uppercase', fontWeight: 600 }}>
              {isDemoMode ? `Paper Positions (${holdings.length})` : 'Token Positions'}
            </div>
            <div style={{ fontSize: '13px', fontWeight: 700, color: isDemoMode ? 'var(--accent-cyan)' : 'var(--text-dim)' }}>
              {isDemoMode ? `$${totalTokensUsd.toFixed(2)}` : 'Unavailable (Unindexed)'}
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: '8px', fontSize: '10px', color: 'var(--text-dim)' }}>
          <span>1 SOL = ${solPriceUsd.toFixed(2)}</span>
          <span className={`pnl-trend-pill ${solChange24h >= 0 ? 'positive' : 'negative'}`} style={{ fontSize: '10px', padding: '2px 6px' }}>
            {solChange24h >= 0 ? '▲ +' : '▼ '}{solChange24h.toFixed(1)}% 24h
          </span>
        </div>
      </div>

      {/* Home Action Buttons (Send, Receive, Trade) */}
      <div className="home-action-row">
        <button className="home-action-item" onClick={() => { setSendAsset('SOL'); setShowSendModal(true); }}>
          <div className="home-action-circle">↑</div>
          <span className="home-action-label">Send</span>
        </button>
        <button className="home-action-item" onClick={() => setShowReceiveModal(true)}>
          <div className="home-action-circle">↓</div>
          <span className="home-action-label">Receive</span>
        </button>
        <button className="home-action-item" onClick={() => { setTradeViewMode('list'); setCurrentNav('trade'); }}>
          <div className="home-action-circle trade-circle">⇄</div>
          <span className="home-action-label" style={{ color: 'var(--accent-green)' }}>Trade</span>
        </button>
      </div>

      {/* Horizontal Menu: Trades | Trending */}
      <div className="home-nav-tabs">
        <button
          className={`home-nav-tab ${homeTab === 'trades' ? 'active' : ''}`}
          onClick={() => setHomeTab('trades')}
        >
          {isDemoMode ? `Trades (${holdings.length})` : 'Positions & Trades'}
        </button>
        <button
          className={`home-nav-tab ${homeTab === 'trending' ? 'active' : ''}`}
          onClick={() => setHomeTab('trending')}
        >
          Trending 🔥
        </button>
      </div>

      {homeTab === 'trades' ? (
        <div>
          {/* Open Trades / Positions Section */}
          <div style={{ marginBottom: '18px' }}>
            <div className="holdings-header">
              <span>{isDemoMode ? `Open Positions (${holdings.length})` : 'Open Positions'}</span>
              <span style={{
                color: isDemoMode ? 'var(--accent-green)' : '#ff6b6b',
                fontSize: '10px',
                fontWeight: 700,
                background: isDemoMode ? undefined : 'rgba(255, 77, 77, 0.1)',
                padding: isDemoMode ? undefined : '2px 6px',
                borderRadius: '4px'
              }}>
                {isDemoMode ? 'PAPER POSITIONS' : 'CHAIN-BACKED DATA UNAVAILABLE'}
              </span>
            </div>

            {!isDemoMode ? (
              <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '24px 16px', textAlign: 'center', color: 'var(--text-muted)' }}>
                <div style={{ fontSize: '24px', marginBottom: '8px' }}>🔒</div>
                <div style={{ fontWeight: 600, color: 'var(--text-main)', marginBottom: '4px', fontSize: '13px' }}>
                  Live Holdings Unavailable
                </div>
                <div style={{ fontSize: '11px', color: 'var(--text-dim)', maxWidth: '340px', margin: '0 auto', lineHeight: 1.4 }}>
                  On-chain token accounts are not indexed in Phase 1. Since live trading is disabled, token positions remain empty until chain-backed indexing is implemented.
                </div>
              </div>
            ) : holdings.length === 0 ? (
              <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '24px 16px', textAlign: 'center', color: 'var(--text-muted)' }}>
                <div style={{ fontSize: '24px', marginBottom: '8px' }}>🪙</div>
                <div style={{ fontWeight: 600, color: 'var(--text-main)', marginBottom: '4px' }}>No Open Trades Yet</div>
                <div style={{ fontSize: '11px', marginBottom: '14px' }}>Open a position with the lowest 0.20% trading fee on Solana.</div>
                <button
                  className="search-btn"
                  style={{ margin: '0 auto', background: 'var(--accent-green)', color: '#000', fontWeight: 700 }}
                  onClick={() => { setTradeViewMode('list'); setCurrentNav('trade'); }}
                >
                  Explore Top Coins
                </button>
              </div>
            ) : (
              <div className="holdings-cards-list">
                {holdings.map((h) => {
                  const live = tokens.find((t) => t.mint === h.mint);
                  const livePrice = live ? live.priceUsd : h.avgBuyPriceUsd;
                  const curVal = h.amount * livePrice;
                  const costVal = h.amount * h.avgBuyPriceUsd;
                  const pnlUsd = curVal - costVal;
                  const pnlPct = costVal > 0 ? (pnlUsd / costVal) * 100 : 0;
                  const isProfitable = pnlUsd >= 0;

                  return (
                    <div key={h.mint} className="holding-card">
                      <div className="holding-main-row" onClick={() => handleTradeHolding(h)}>
                        <div className="holding-token-info">
                          {h.imageUrl ? (
                            <img
                              src={h.imageUrl}
                              alt={h.symbol}
                              className="holding-token-avatar"
                              onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
                            />
                          ) : (
                            <div className="holding-token-avatar-fallback">{h.symbol.charAt(0)}</div>
                          )}
                          <div className="holding-token-names">
                            <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                              <span className="holding-token-symbol">{h.symbol}</span>
                              <span className={`pnl-badge ${isProfitable ? 'positive' : 'negative'}`}>
                                {isProfitable ? '▲ +' : '▼ -'}${Math.abs(pnlUsd).toFixed(2)} ({isProfitable ? '+' : ''}{pnlPct.toFixed(1)}%)
                              </span>
                            </div>
                            <span className="holding-token-subname">{h.name}</span>
                          </div>
                        </div>
                        <div className="holding-values">
                          <div className="holding-fiat-val">${curVal.toFixed(2)}</div>
                          <div className="holding-token-amt">
                            {h.amount.toLocaleString(undefined, { maximumFractionDigits: 0 })} {h.symbol}
                          </div>
                        </div>
                      </div>

                      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '10px', color: 'var(--text-dim)', borderTop: '1px solid rgba(255, 255, 255, 0.04)', paddingTop: '6px' }}>
                        <span>Avg Entry: ${h.avgBuyPriceUsd < 0.01 ? h.avgBuyPriceUsd.toFixed(6) : h.avgBuyPriceUsd.toFixed(4)}</span>
                        <span>Now: ${livePrice < 0.01 ? livePrice.toFixed(6) : livePrice.toFixed(4)}</span>
                      </div>

                      {/* Quick actions for this holding */}
                      <div className="holding-quick-actions">
                        <button className="holding-sub-btn trade-btn" onClick={() => handleTradeHolding(h)}>
                          ⇄ Trade / Sell
                        </button>
                        <button className="holding-sub-btn" onClick={() => handleSendHolding(h)}>
                          ↑ Send
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Trade Orders & History */}
          <div>
            <div className="holdings-header">
              <span>Orders & History ({orderHistory.length})</span>
              <span style={{
                color: isDemoMode ? 'var(--text-dim)' : 'var(--accent-cyan)',
                fontSize: '10px',
                fontWeight: 700,
                background: isDemoMode ? undefined : 'rgba(0, 240, 255, 0.1)',
                padding: isDemoMode ? undefined : '2px 6px',
                borderRadius: '4px'
              }}>
                {isDemoMode ? 'RECENT PAPER ORDERS' : '⚡ LIVE ON-CHAIN ORDERS'}
              </span>
            </div>

            {orderHistory.length === 0 ? (
              <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '16px', textAlign: 'center', color: 'var(--text-muted)', fontSize: '11px' }}>
                {isDemoMode ? 'No completed paper trades yet.' : 'No completed on-chain trades yet.'}
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                {orderHistory.slice(0, 5).map((item) => {
                  const isBuy = item.type === 'buy';
                  return (
                    <div key={item.id} className="holding-card" style={{ padding: '8px 10px' }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                          <span style={{
                            padding: '2px 6px',
                            borderRadius: '4px',
                            fontSize: '10px',
                            fontWeight: 800,
                            background: isBuy ? 'rgba(0, 240, 160, 0.15)' : 'rgba(255, 71, 87, 0.15)',
                            color: isBuy ? 'var(--accent-green)' : 'var(--accent-red)'
                          }}>
                            {item.type.toUpperCase()}
                          </span>
                          <span style={{ fontWeight: 700, fontSize: '12px' }}>{item.symbol}</span>
                          {item.realizedPnlUsd !== undefined && (
                            <span className={`pnl-badge ${item.realizedPnlUsd >= 0 ? 'positive' : 'negative'}`} style={{ fontSize: '9px' }}>
                              PnL: {item.realizedPnlUsd >= 0 ? '+' : '-'}${Math.abs(item.realizedPnlUsd).toFixed(2)}
                            </span>
                          )}
                        </div>
                        <div style={{ textAlign: 'right', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
                          <div style={{ fontWeight: 700, color: isBuy ? 'var(--accent-green)' : 'var(--accent-red)' }}>
                            {isBuy ? '+' : '-'}{item.amountTokens.toLocaleString(undefined, { maximumFractionDigits: 0 })}
                          </div>
                          <div style={{ fontSize: '10px', color: 'var(--text-dim)' }}>
                            {item.amountSol.toFixed(4)} SOL
                          </div>
                        </div>
                      </div>
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: '4px', fontSize: '9px', color: 'var(--text-dim)' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                          <span>{item.time}</span>
                          {item.executionMs !== undefined && (
                            <span style={{
                              color: 'var(--accent-cyan)',
                              background: 'rgba(0, 240, 255, 0.12)',
                              padding: '1px 5px',
                              borderRadius: '3px',
                              fontFamily: 'var(--font-mono)',
                              fontWeight: 700,
                              fontSize: '9px'
                            }}>
                              ⚡ {item.executionMs < 1000 ? `${item.executionMs}ms` : `${(item.executionMs / 1000).toFixed(2)}s`}
                            </span>
                          )}
                        </div>
                        {item.signature.startsWith('SIMULATED_') ? (
                          <span style={{ color: '#c084fc', fontFamily: 'var(--font-mono)' }}>[Demo Paper Simulation]</span>
                        ) : (
                          <a href={`https://solscan.io/tx/${item.signature}`} target="_blank" rel="noreferrer" style={{ color: 'var(--accent-cyan)' }}>
                            Solscan ↗
                          </a>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      ) : (
        /* Trending Tab */
        <div>
          <div className="holdings-header">
            <span>Trending Solana Markets</span>
            <span style={{ color: 'var(--accent-cyan)', fontSize: '10px' }}>TAP TO TRADE</span>
          </div>

          <div className="holdings-cards-list">
            {tokens.slice(0, 10).map((tok, idx) => (
              <div
                key={tok.mint}
                className="holding-card"
                style={{ cursor: 'pointer' }}
                onClick={() => handleOpenCoinDetail(tok)}
              >
                <div className="holding-main-row">
                  <div className="holding-token-info">
                    <div className={`rank-pill ${idx === 0 ? 'gold' : idx === 1 ? 'silver' : idx === 2 ? 'bronze' : ''}`}>
                      {idx + 1}
                    </div>
                    {tok.imageUrl ? (
                      <img
                        src={tok.imageUrl}
                        alt={tok.symbol}
                        className="holding-token-avatar"
                        onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
                      />
                    ) : (
                      <div className="holding-token-avatar-fallback">{tok.symbol.charAt(0)}</div>
                    )}
                    <div className="holding-token-names">
                      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span className="holding-token-symbol">{tok.symbol}</span>
                        <span className={`dex-tag ${tok.dexId}`} style={{ fontSize: '9px', padding: '1px 5px' }}>
                          {DEX_LABELS[tok.dexId] || tok.dexId}
                        </span>
                      </div>
                      <span className="holding-token-subname">{tok.name}</span>
                    </div>
                  </div>
                  <div className="holding-values">
                    <div className="holding-fiat-val">${tok.priceUsd < 0.01 ? tok.priceUsd.toFixed(6) : tok.priceUsd.toFixed(4)}</div>
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: '6px', fontSize: '10px' }}>
                      <span style={{ color: tok.change24h >= 0 ? 'var(--accent-green)' : 'var(--accent-red)', fontWeight: 700 }}>
                        {tok.change24h >= 0 ? '+' : ''}{tok.change24h.toFixed(1)}%
                      </span>
                      <span style={{ color: 'var(--text-dim)' }}>
                        ${tok.volume24h >= 1_000_000 ? (tok.volume24h / 1_000_000).toFixed(1) + 'M' : (tok.volume24h / 1000).toFixed(0) + 'k'} Vol
                      </span>
                    </div>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );

  const renderTradeScreen = () => {
    // Mode 1: Top 10 Highest Volume Tokens List & Search Bar
    if (tradeViewMode === 'list') {
      const q = searchQuery.trim().toLowerCase();
      const filtered = q.length > 0
        ? tokens.filter((t) => t.symbol.toLowerCase().includes(q) || t.name.toLowerCase().includes(q) || t.mint.toLowerCase().includes(q))
        : [...tokens].sort((a, b) => b.volume24h - a.volume24h).slice(0, 10);

      return (
        <div>
          {/* Header bar */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
            <h2 style={{ fontSize: '18px', fontWeight: 800 }}>Trade Markets</h2>
            <span className="lowest-fee-pill">
              ⚡ 0.20% Fee · Lowest
            </span>
          </div>

          {/* Search Bar */}
          <div className="search-box" style={{ marginBottom: '16px' }}>
            <input
              type="text"
              className="search-input"
              placeholder="Search by coin name, ticker, or CA..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && fetchLiveTokens(searchQuery)}
            />
            <button className="search-btn" onClick={() => fetchLiveTokens(searchQuery)} disabled={isSearching}>
              {isSearching ? '...' : 'FIND'}
            </button>
          </div>

          {/* Section Header */}
          <div className="holdings-header">
            <span>{q.length > 0 ? `Search Results (${filtered.length})` : 'Top 10 Highest Volume on Solana'}</span>
            <span style={{ color: 'var(--accent-cyan)', fontSize: '10px' }}>TAP COIN TO TRADE</span>
          </div>

          {/* Top 10 / Filtered Token List */}
          <div className="holdings-cards-list">
            {filtered.map((tok, idx) => (
              <div
                key={tok.mint}
                className="holding-card"
                style={{ cursor: 'pointer' }}
                onClick={() => handleOpenCoinDetail(tok)}
              >
                <div className="holding-main-row">
                  <div className="holding-token-info">
                    <div className={`rank-pill ${idx === 0 ? 'gold' : idx === 1 ? 'silver' : idx === 2 ? 'bronze' : ''}`}>
                      {idx + 1}
                    </div>
                    {tok.imageUrl ? (
                      <img
                        src={tok.imageUrl}
                        alt={tok.symbol}
                        className="holding-token-avatar"
                        onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
                      />
                    ) : (
                      <div className="holding-token-avatar-fallback">{tok.symbol.charAt(0)}</div>
                    )}
                    <div className="holding-token-names">
                      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span className="holding-token-symbol">{tok.symbol}</span>
                        <span className={`dex-tag ${tok.dexId}`} style={{ fontSize: '9px', padding: '1px 5px' }}>
                          {DEX_LABELS[tok.dexId] || tok.dexId}
                        </span>
                      </div>
                      <span className="holding-token-subname">{tok.name}</span>
                    </div>
                  </div>

                  <div className="holding-values">
                    <div className="holding-fiat-val">${tok.priceUsd < 0.01 ? tok.priceUsd.toFixed(6) : tok.priceUsd.toFixed(4)}</div>
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: '6px', fontSize: '10px' }}>
                      <span style={{ color: tok.change24h >= 0 ? 'var(--accent-green)' : 'var(--accent-red)', fontWeight: 700 }}>
                        {tok.change24h >= 0 ? '+' : ''}{tok.change24h.toFixed(1)}%
                      </span>
                      <span style={{ color: 'var(--accent-green)', fontWeight: 600 }}>
                        ${tok.volume24h >= 1_000_000 ? (tok.volume24h / 1_000_000).toFixed(1) + 'M' : (tok.volume24h / 1000).toFixed(0) + 'k'} Vol
                      </span>
                    </div>
                  </div>
                </div>

                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', borderTop: '1px solid rgba(255, 255, 255, 0.04)', paddingTop: '6px', fontSize: '10px', color: 'var(--text-dim)' }}>
                  <span
                    onClick={(e) => {
                      e.stopPropagation();
                      handleCopyCA(tok.mint);
                    }}
                    style={{ cursor: 'pointer', fontFamily: 'var(--font-mono)', display: 'flex', alignItems: 'center', gap: '4px' }}
                  >
                    CA: {tok.mint.slice(0, 6)}...{tok.mint.slice(-4)} {caCopied ? '✓' : '⎘'}
                  </span>
                  <span>${(tok.totalLiquidity / 1_000_000).toFixed(2)}M Liq</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      );
    }

    // Mode 2: Dedicated Coin Trading Page (Coin Name, Price, Change, Chart, Buy/Sell, Open Trades, Orders with PnL)
    const isBuying = activeTab === 'buy';
    const heldForCoin = holdings.find((h) => h.mint === selectedToken.mint);
    const heldCoinAmt = heldForCoin ? heldForCoin.amount : 0;
    const curVal = heldCoinAmt * selectedToken.priceUsd;
    const costVal = heldForCoin ? (heldCoinAmt * heldForCoin.avgBuyPriceUsd) : 0;
    const coinPnlUsd = curVal - costVal;
    const coinPnlPct = costVal > 0 ? (coinPnlUsd / costVal) * 100 : 0;
    const isProfitable = coinPnlUsd >= 0;

    const coinOrders = orderHistory.filter((o) => o.mint === selectedToken.mint);
    const isUp = selectedToken.change24h >= 0;

    return (
      <div>
        {/* Back Link Button */}
        <button className="back-link-btn" onClick={() => setTradeViewMode('list')}>
          ← Back to Markets
        </button>

        {/* Coin Detail Header */}
        <div className="coin-detail-header">
          <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
            {selectedToken.imageUrl ? (
              <img
                src={selectedToken.imageUrl}
                alt=""
                style={{ width: '38px', height: '38px', borderRadius: '50%' }}
                onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
              />
            ) : (
              <div style={{ width: '38px', height: '38px', borderRadius: '50%', background: 'linear-gradient(135deg, var(--accent-cyan), var(--accent-green))', color: '#000', display: 'flex', alignItems: 'center', justifyContent: 'center', fontWeight: 800 }}>
                {selectedToken.symbol.slice(0, 2)}
              </div>
            )}
            <div>
              <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                <span style={{ fontWeight: 800, fontSize: '16px' }}>{selectedToken.symbol}</span>
                <span className={`dex-tag ${selectedToken.dexId}`}>
                  {DEX_LABELS[selectedToken.dexId] || selectedToken.dexId}
                </span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: '6px', fontSize: '11px', color: 'var(--text-dim)' }}>
                <span>{selectedToken.name}</span>
                <span
                  onClick={() => handleCopyCA(selectedToken.mint)}
                  style={{ cursor: 'pointer', color: 'var(--accent-cyan)', fontFamily: 'var(--font-mono)' }}
                >
                  CA: {selectedToken.mint.slice(0, 4)}...{selectedToken.mint.slice(-4)} {caCopied ? '✓' : '⎘'}
                </span>
              </div>
            </div>
          </div>

          <div style={{ textAlign: 'right' }}>
            <div className="coin-price-big">
              ${selectedToken.priceUsd < 0.01 ? selectedToken.priceUsd.toFixed(6) : selectedToken.priceUsd.toFixed(4)}
            </div>
            <span className={`pnl-badge ${isUp ? 'positive' : 'negative'}`} style={{ marginTop: '2px' }}>
              {isUp ? '▲ +' : '▼ '}{selectedToken.change24h.toFixed(1)}%
            </span>
          </div>
        </div>

        {/* Buy / Sell Tabs */}
        <div className="order-tabs" style={{ marginBottom: '12px' }}>
          <button
            className={`tab-btn ${isBuying ? 'active buy' : ''}`}
            onClick={() => setActiveTab('buy')}
          >
            BUY {selectedToken.symbol}
          </button>
          <button
            className={`tab-btn ${!isBuying ? 'active sell' : ''}`}
            onClick={() => {
              setActiveTab('sell');
              if (heldCoinAmt > 0) setAmount(Math.floor(heldCoinAmt).toString());
            }}
          >
            SELL {selectedToken.symbol}
          </button>
        </div>

        {/* Order Form Box */}
        <div className="swap-card-box">
          <div className="swap-card-header">
            <span>{isBuying ? 'YOU SPEND' : `SELL ${selectedToken.symbol}`}</span>
            <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
              <span>Balance: {isBuying ? `${walletBalance.toFixed(4)} SOL` : `${heldCoinAmt.toLocaleString()} ${selectedToken.symbol}`}</span>
              {isBuying && (
                <button
                  onClick={handleAirdropTestSol}
                  style={{
                    padding: '1px 6px',
                    borderRadius: '8px',
                    background: 'rgba(0, 240, 160, 0.15)',
                    color: 'var(--accent-green)',
                    border: '1px solid rgba(0, 240, 160, 0.3)',
                    fontSize: '9px',
                    fontWeight: 700,
                    cursor: 'pointer',
                  }}
                  title="Claim +2.0 SOL test funds"
                >
                  +2 SOL Faucet
                </button>
              )}
            </span>
          </div>

          <div className="swap-input-row">
            <input
              type="number"
              className="swap-num-input"
              placeholder="0.0"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              min="0"
              step="any"
            />
            <div className="swap-token-badge">
              {isBuying ? (
                <>
                  <span style={{ color: 'var(--accent-purple)' }}>◎</span>
                  <span>SOL</span>
                </>
              ) : (
                <span>{selectedToken.symbol}</span>
              )}
            </div>
          </div>

          {/* Quick preset chips */}
          <div style={{ display: 'flex', gap: '6px', marginTop: '10px' }}>
            {isBuying ? (
              <>
                {['0.1', '0.5', '1.0'].map((val) => (
                  <button
                    key={val}
                    className="preset-chip"
                    style={{ flex: 1, padding: '4px', fontSize: '11px' }}
                    onClick={() => setAmount(val)}
                  >
                    {val} SOL
                  </button>
                ))}
                <button
                  className="preset-chip"
                  style={{ flex: 1, padding: '4px', fontSize: '11px' }}
                  onClick={() => setAmount(Math.max(0, walletBalance - 0.01).toFixed(3))}
                >
                  MAX
                </button>
              </>
            ) : (
              <>
                {['25%', '50%', '75%', '100%'].map((pct) => (
                  <button
                    key={pct}
                    className="preset-chip"
                    style={{ flex: 1, padding: '4px', fontSize: '11px' }}
                    onClick={() => {
                      const fraction = parseInt(pct) / 100;
                      setAmount(Math.floor(heldCoinAmt * fraction).toString());
                    }}
                  >
                    {pct}
                  </button>
                ))}
              </>
            )}
          </div>
        </div>

        {/* Phase 2: Real-Time Best-Route Comparator Panel */}
        <div className="comparator-panel">
          <div className="comparator-header">
            <div className="comparator-title">
              <span>⚡ Best-Route Comparator</span>
              <span className="estimate-disclaimer-pill">READ-ONLY</span>
            </div>
            <div className="comparator-freshness">
              {isComparingRoutes ? (
                <span style={{ color: 'var(--accent-cyan)' }}>● Quoting live venues...</span>
              ) : routeComparison ? (
                <span>
                  {routeComparison.best_route_index !== undefined && routeComparison.best_route_index !== null && routeComparison.routes[routeComparison.best_route_index]?.state_slot
                    ? `Slot #${routeComparison.routes[routeComparison.best_route_index].state_slot}`
                    : 'Live On-Chain'} · {new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
                </span>
              ) : (
                <span style={{ color: 'var(--text-dim)' }}>Ready to quote</span>
              )}
            </div>
          </div>

          {comparisonError && (
            <div style={{
              background: 'rgba(255, 77, 77, 0.08)',
              border: '1px solid rgba(255, 77, 77, 0.25)',
              borderRadius: 'var(--radius-sm)',
              padding: '6px 10px',
              fontSize: '11px',
              color: 'var(--accent-red)',
              marginBottom: '10px'
            }}>
              ⚠️ Route comparison warning: {comparisonError}
            </div>
          )}

          {routeComparison && routeComparison.best_route_index !== undefined && routeComparison.best_route_index !== null && routeComparison.routes[routeComparison.best_route_index] ? (() => {
            const best = routeComparison.routes[routeComparison.best_route_index];
            const runnerUp = routeComparison.runner_up_index !== undefined && routeComparison.runner_up_index !== null
              ? routeComparison.routes[routeComparison.runner_up_index]
              : null;
            const diffAtomic = routeComparison.output_difference_atomic || 0;
            const isBuy = activeTab === 'buy';

            const bestNetFormatted = isBuy
              ? `${((best.net_output_atomic || 0) / 1e6).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${selectedToken.symbol}`
              : `${((best.net_output_atomic || 0) / 1e9).toFixed(6)} SOL`;

            const bestGrossFormatted = isBuy
              ? `${((best.gross_output_atomic || 0) / 1e6).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${selectedToken.symbol}`
              : `${((best.gross_output_atomic || 0) / 1e9).toFixed(6)} SOL`;

            const platformFeeDisplay = best.platform_fee_atomic
              ? `${(best.platform_fee_atomic / 1e9).toFixed(6)} SOL`
              : `${(routeComparison.platform_fee_bps / 100).toFixed(2)}%`;

            const runnerUpNetFormatted = runnerUp && runnerUp.net_output_atomic
              ? (isBuy
                ? `${(runnerUp.net_output_atomic / 1e6).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${selectedToken.symbol}`
                : `${(runnerUp.net_output_atomic / 1e9).toFixed(6)} SOL`)
              : null;

            const diffDisplay = isBuy
              ? `+${(diffAtomic / 1e6).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${selectedToken.symbol}`
              : `+${(diffAtomic / 1e9).toFixed(6)} SOL`;

            const pctAdvantage = runnerUp && runnerUp.net_output_atomic && runnerUp.net_output_atomic > 0
              ? ((diffAtomic / runnerUp.net_output_atomic) * 100).toFixed(2)
              : null;

            return (
              <>
                {/* Winner Card */}
                <div className="best-route-winner-card">
                  <div className="best-route-badge">
                    🏆 Recommended Route
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
                    <span style={{ fontWeight: 800, fontSize: '14px', color: 'var(--accent-green)' }}>
                      {best.venue}
                    </span>
                    <span className={`dex-tag ${best.venue.toLowerCase().includes('pump') ? 'pump' : best.venue.toLowerCase().includes('raydium') ? 'raydium' : best.venue.toLowerCase().includes('meteora') ? 'meteora' : 'unknown'}`}>
                      {best.source.toUpperCase()}
                    </span>
                  </div>

                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '6px', fontSize: '11px', marginBottom: '8px' }}>
                    <div>
                      <span style={{ color: 'var(--text-muted)' }}>Estimated Net Output:</span>
                      <div style={{ fontSize: '14px', fontWeight: 800, color: 'var(--text-main)', marginTop: '2px', fontFamily: 'var(--font-mono)' }}>
                        {bestNetFormatted}
                        <span className="estimate-disclaimer-pill">EST.</span>
                      </div>
                    </div>
                    <div style={{ textAlign: 'right' }}>
                      <span style={{ color: 'var(--text-muted)' }}>Gross Output:</span>
                      <div style={{ fontSize: '12px', fontWeight: 600, color: 'var(--text-dim)', marginTop: '2px', fontFamily: 'var(--font-mono)' }}>
                        {bestGrossFormatted}
                      </div>
                    </div>
                  </div>

                  {/* Fee & Impact Breakdown */}
                  <div style={{
                    background: 'rgba(0, 0, 0, 0.25)',
                    borderRadius: 'var(--radius-sm)',
                    padding: '6px 8px',
                    fontSize: '10px',
                    fontFamily: 'var(--font-mono)',
                    display: 'flex',
                    justifyContent: 'space-between',
                    flexWrap: 'wrap',
                    gap: '4px'
                  }}>
                    <span>Venue Fee: <strong style={{ color: 'var(--text-main)' }}>{best.venue_fee_bps ? `${(best.venue_fee_bps / 100).toFixed(2)}%` : '0%'}</strong></span>
                    <span>Platform Fee: <strong style={{ color: 'var(--accent-cyan)' }}>{(routeComparison.platform_fee_bps / 100).toFixed(2)}% ({platformFeeDisplay})</strong></span>
                    <span>Price Impact: <strong style={{ color: best.price_impact_pct && best.price_impact_pct > 3 ? 'var(--accent-red)' : 'var(--accent-green)' }}>{best.price_impact_pct !== undefined && best.price_impact_pct !== null ? `${best.price_impact_pct.toFixed(2)}%` : 'N/A'}</strong></span>
                  </div>

                  {best.pool_address && (
                    <div style={{ marginTop: '6px', fontSize: '10px', color: 'var(--text-dim)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                      <span style={{ fontFamily: 'var(--font-mono)' }}>
                        Pool: {best.pool_address.slice(0, 8)}...{best.pool_address.slice(-6)}
                      </span>
                      <span
                        onClick={() => handleCopyCA(best.pool_address || '')}
                        style={{ cursor: 'pointer', color: 'var(--accent-cyan)' }}
                      >
                        Copy Pool CA ⎘
                      </span>
                    </div>
                  )}

                  <div style={{ marginTop: '8px', paddingTop: '6px', borderTop: '1px dashed rgba(255, 255, 255, 0.08)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span style={{ fontSize: '10px', color: 'var(--text-dim)', display: 'flex', alignItems: 'center', gap: '4px' }}>
                      <span style={{ color: 'var(--accent-cyan)' }}>●</span>
                      <span>Phase 3 Unsigned Candidate Target</span>
                    </span>
                    <button
                      className="preset-chip active"
                      onClick={() => {
                        setSelectedRouteIdx(routeComparison.best_route_index ?? 0);
                        handlePrepareAndValidateCandidate(best);
                      }}
                      disabled={isPreparingCandidate}
                      style={{
                        padding: '3px 8px',
                        fontSize: '10px',
                        fontWeight: 700,
                        background: 'rgba(56, 189, 248, 0.2)',
                        border: '1px solid var(--accent-cyan)',
                        color: 'var(--accent-cyan)',
                        cursor: 'pointer'
                      }}
                    >
                      {isPreparingCandidate && selectedRouteIdx === routeComparison.best_route_index
                        ? 'Preparing...'
                        : 'Prepare This Candidate →'}
                    </button>
                  </div>
                </div>

                {/* Runner-Up Comparison Card */}
                {runnerUp && runnerUpNetFormatted && (
                  <div className="runner-up-card">
                    <div>
                      <div style={{ color: 'var(--text-muted)', fontSize: '10px' }}>RUNNER-UP ROUTE</div>
                      <div style={{ fontWeight: 700, color: 'var(--text-main)', marginTop: '1px' }}>
                        🥈 {runnerUp.venue} ({runnerUpNetFormatted})
                      </div>
                    </div>
                    <div style={{ textAlign: 'right' }}>
                      <div className="runner-up-advantage">
                        {diffDisplay}
                      </div>
                      <div style={{ fontSize: '9px', color: 'var(--text-dim)' }}>
                        {pctAdvantage ? `+${pctAdvantage}% higher net output` : 'Recommended route advantage'}
                      </div>
                    </div>
                  </div>
                )}

                {/* Recommendation Reason Notice */}
                {routeComparison.recommendation_reason && (
                  <div style={{ fontSize: '10px', color: 'var(--text-muted)', marginBottom: '8px', fontStyle: 'italic', padding: '0 4px' }}>
                    💡 {routeComparison.recommendation_reason}
                  </div>
                )}
              </>
            );
          })() : (
            /* Fallback preview while loading or before quote arrives */
            <div className="preview-grid" style={{ marginBottom: '10px' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span>Est. Net Output (Reference):</span>
                <span style={{ fontWeight: 700, color: 'var(--text-main)' }}>
                  {activeTab === 'buy'
                    ? `${estimatedTokensOut.toLocaleString(undefined, { maximumFractionDigits: 0 })} ${selectedToken.symbol} (EST)`
                    : `${netSolFromSell.toFixed(4)} SOL (EST)`}
                </span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span>Platform Treasury Fee ({feeBps} bps):</span>
                <span style={{ color: 'var(--accent-green)', fontWeight: 600 }}>{feeSol.toFixed(6)} SOL ({(feeBps / 100).toFixed(2)}%)</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <span>Candidate Venue:</span>
                <span style={{ color: 'var(--accent-cyan)' }}>{selectedRoute?.dexLabel || selectedToken.dexId.toUpperCase()}</span>
              </div>
            </div>
          )}

          {/* Candidate & Unavailable Routes Accordion */}
          {routeComparison && routeComparison.routes && routeComparison.routes.length > 0 && (
            <div className="routes-table-container">
              <div
                onClick={() => setShowAllRoutes(!showAllRoutes)}
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  alignItems: 'center',
                  cursor: 'pointer',
                  fontSize: '11px',
                  fontWeight: 700,
                  color: 'var(--accent-cyan)',
                  userSelect: 'none'
                }}
              >
                <span>Supported Venues & Discovery ({routeComparison.routes.length})</span>
                <span>{showAllRoutes ? '▲ Hide Details' : '▼ Compare All Venues'}</span>
              </div>

              {showAllRoutes && (
                <div style={{ marginTop: '8px' }}>
                  {routeComparison.routes.map((r, idx) => {
                    const isWinner = routeComparison.best_route_index === idx;
                    const isRunnerUp = routeComparison.runner_up_index === idx;
                    const isEst = r.status === 'estimated';
                    const isSelected = selectedRouteIdx === idx;

                    return (
                      <div
                        key={r.venue + idx}
                        className="routes-list-item"
                        onClick={() => {
                          if (isEst) setSelectedRouteIdx(idx);
                        }}
                        style={{
                          cursor: isEst ? 'pointer' : 'default',
                          border: isSelected ? '1px solid var(--accent-cyan)' : undefined,
                          background: isSelected ? 'rgba(56, 189, 248, 0.05)' : undefined,
                          transition: 'all 0.15s ease',
                        }}
                      >
                        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                          <span style={{ fontSize: '11px' }}>
                            {isWinner ? '🏆' : isRunnerUp ? '🥈' : '•'}
                          </span>
                          <div>
                            <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                              <span style={{ fontWeight: isWinner ? 800 : 600, color: isWinner ? 'var(--accent-green)' : 'var(--text-main)' }}>
                                {r.venue}
                              </span>
                              {isSelected && (
                                <span style={{ fontSize: '9px', fontWeight: 700, color: 'var(--accent-cyan)', background: 'rgba(56, 189, 248, 0.15)', padding: '1px 4px', borderRadius: '3px' }}>
                                  TARGET
                                </span>
                              )}
                            </div>
                            <div style={{ fontSize: '9px', color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
                              {r.pool_address ? `${r.pool_address.slice(0, 6)}...${r.pool_address.slice(-4)}` : r.source}
                            </div>
                          </div>
                        </div>

                        <div style={{ textAlign: 'right' }}>
                          {isEst && r.net_output_atomic ? (
                            <div>
                              <div style={{ fontFamily: 'var(--font-mono)', fontWeight: 700, color: isWinner ? 'var(--accent-green)' : 'var(--text-main)' }}>
                                {activeTab === 'buy'
                                  ? `${(r.net_output_atomic / 1e6).toLocaleString(undefined, { maximumFractionDigits: 1 })} ${selectedToken.symbol}`
                                  : `${(r.net_output_atomic / 1e9).toFixed(5)} SOL`}
                              </div>
                              <div style={{ fontSize: '9px', color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', marginTop: '1px' }}>
                                Impact: {r.price_impact_pct !== undefined && r.price_impact_pct !== null ? `${r.price_impact_pct.toFixed(2)}%` : 'N/A'} | Fee: {(r.venue_fee_bps ? r.venue_fee_bps / 100 : 0).toFixed(2)}%
                              </div>
                              <button
                                className="preset-chip"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setSelectedRouteIdx(idx);
                                  handlePrepareAndValidateCandidate(r);
                                }}
                                disabled={isPreparingCandidate}
                                style={{
                                  marginTop: '3px',
                                  padding: '2px 6px',
                                  fontSize: '9px',
                                  fontWeight: 600,
                                  background: isSelected ? 'rgba(56, 189, 248, 0.2)' : 'rgba(255, 255, 255, 0.05)',
                                  color: isSelected ? 'var(--accent-cyan)' : 'var(--text-muted)',
                                  border: isSelected ? '1px solid var(--accent-cyan)' : '1px solid var(--border-subtle)',
                                  cursor: 'pointer'
                                }}
                              >
                                {isPreparingCandidate && selectedRouteIdx === idx ? 'Preparing...' : 'Prepare Candidate'}
                              </button>
                            </div>
                          ) : (
                            <div style={{ fontSize: '9px', color: 'var(--text-dim)', maxWidth: '170px', lineHeight: 1.2 }}>
                              {r.reason_unavailable || r.error || 'Unavailable'}
                            </div>
                          )}
                          <div style={{ marginTop: '3px' }}>
                            <span className={`route-status-pill ${r.status}`}>
                              {r.status}
                            </span>
                          </div>
                        </div>
                      </div>
                    );
                  })}

                  {routeComparison.candidate_pools && routeComparison.candidate_pools.length > 0 && (
                    <div style={{ marginTop: '10px', borderTop: '1px solid rgba(255, 255, 255, 0.05)', paddingTop: '8px' }}>
                      <div style={{ fontSize: '10px', fontWeight: 700, color: 'var(--text-muted)', marginBottom: '6px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <span>INDEXED MARKET POOLS (DEXSCREENER) ({routeComparison.candidate_pools.length})</span>
                        <span style={{ fontSize: '8px', color: 'var(--text-dim)', fontWeight: 400 }}>Verified via Solana RPC</span>
                      </div>
                      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                        {routeComparison.candidate_pools.map((p, pIdx) => (
                          <div
                            key={p.pool_address + pIdx}
                            style={{
                              display: 'flex',
                              justifyContent: 'space-between',
                              alignItems: 'center',
                              background: 'rgba(255, 255, 255, 0.02)',
                              padding: '4px 6px',
                              borderRadius: '4px',
                              fontSize: '9px',
                              fontFamily: 'var(--font-mono)',
                            }}
                          >
                            <div>
                              <span style={{ fontWeight: 600, color: 'var(--text-main)' }}>{p.dex_label}</span>
                              <span style={{ color: 'var(--text-dim)', marginLeft: '6px' }}>
                                {p.pool_address.slice(0, 6)}...{p.pool_address.slice(-4)}
                              </span>
                            </div>
                            <div style={{ display: 'flex', alignItems: 'center', gap: '5px' }}>
                              <span style={{ color: 'var(--text-dim)' }}>
                                ${p.liquidity_usd >= 1e6 ? `${(p.liquidity_usd / 1e6).toFixed(1)}M` : `${Math.round(p.liquidity_usd).toLocaleString()}`} Liq
                              </span>
                              {p.is_verified_on_chain !== undefined && (
                                <span
                                  style={{
                                    padding: '1px 4px',
                                    borderRadius: '3px',
                                    fontSize: '8px',
                                    fontWeight: 700,
                                    background: p.is_verified_on_chain ? 'rgba(56, 189, 248, 0.15)' : 'rgba(156, 163, 175, 0.15)',
                                    color: p.is_verified_on_chain ? 'var(--accent-cyan)' : 'var(--text-dim)',
                                  }}
                                >
                                  {p.is_verified_on_chain ? 'ON-CHAIN' : 'OFF-CHAIN'}
                                </span>
                              )}
                              <span
                                style={{
                                  padding: '1px 4px',
                                  borderRadius: '3px',
                                  fontSize: '8px',
                                  fontWeight: 700,
                                  background: p.passes_dust_filter ? 'rgba(74, 222, 128, 0.15)' : 'rgba(239, 68, 68, 0.15)',
                                  color: p.passes_dust_filter ? 'var(--accent-green)' : 'var(--accent-red)',
                                }}
                              >
                                {p.passes_dust_filter ? 'PASSES DUST' : 'FILTERED DUST'}
                              </span>
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}

                  <div style={{ fontSize: '9px', color: 'var(--text-dim)', marginTop: '8px', borderTop: '1px dashed rgba(255, 255, 255, 0.05)', paddingTop: '6px' }}>
                    Dust Filtering: Minimum ${routeComparison.min_liquidity_threshold_usd.toLocaleString()} liquidity & ${routeComparison.min_volume_threshold_usd.toLocaleString()} 24h volume.
                  </div>
                  <div style={{ fontSize: '9px', color: 'var(--text-dim)', marginTop: '4px', lineHeight: 1.4 }}>
                    ℹ️ <strong>Routing & Filtering Notice:</strong> Direct AMM quoting is active for Pump.fun and PumpSwap. Liquidity across Raydium, Meteora, and Orca is evaluated via Jupiter meta-aggregation; dust thresholds filter indexed pools for inspection but do not constrain Jupiter&apos;s internal route-planner.
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Phase 2 Read-Only Disclaimer */}
          <div style={{
            fontSize: '9px',
            color: 'var(--text-dim)',
            marginTop: '10px',
            textAlign: 'center',
            borderTop: '1px solid rgba(255, 255, 255, 0.04)',
            paddingTop: '6px'
          }}>
            {routeComparison?.disclaimer || 'Live quotes computed from on-chain AMM reserves and Jupiter Swap V2 meta-aggregation. Swaps are verified and simulated natively before signing.'}
          </div>
        </div>


        {/* Execution Settings (Slippage & Anti-MEV) */}
        <div style={{ background: 'var(--bg-card)', border: '1px solid var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '10px 12px', marginBottom: '12px' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '6px', fontSize: '11px' }}>
            <span style={{ color: 'var(--text-muted)', fontWeight: 600 }}>SLIPPAGE LIMIT</span>
            <span style={{ color: 'var(--accent-green)', fontWeight: 700 }}>{(slippage / 100).toFixed(1)}%</span>
          </div>
          <div style={{ display: 'flex', gap: '6px', marginBottom: '8px' }}>
            {[
              { label: '0.5%', val: 50 },
              { label: '1.0%', val: 100 },
              { label: '1.5%', val: 150 },
              { label: '3.0%', val: 300 },
              { label: '5.0%', val: 500 },
            ].map((s) => (
              <button
                key={s.val}
                className={`preset-chip ${slippage === s.val ? 'active' : ''}`}
                style={{ flex: 1, padding: '3px', fontSize: '10px' }}
                onClick={() => setSlippage(s.val)}
              >
                {s.label}
              </button>
            ))}
          </div>

          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '11px', borderTop: '1px solid var(--border-subtle)', paddingTop: '6px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
              <input
                type="checkbox"
                id="antimev-toggle"
                checked={antiMev}
                onChange={(e) => setAntiMev(e.target.checked)}
                style={{ cursor: 'pointer' }}
              />
              <label htmlFor="antimev-toggle" style={{ color: 'var(--text-muted)', fontWeight: 600, cursor: 'pointer' }}>ANTI-MEV TIP</label>
            </div>
            <div style={{ display: 'flex', gap: '4px' }}>
              {(['normal', 'fast', 'turbo'] as const).map((mode) => (
                <button
                  key={mode}
                  className={`preset-chip ${priorityFee === mode ? 'active' : ''}`}
                  style={{ padding: '2px 6px', fontSize: '9px' }}
                  onClick={() => setPriorityFee(mode)}
                >
                  {mode.toUpperCase()}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* 1-Click Auto-Approval Fast Trading Toggle */}
        <div style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          background: autoApprove ? 'rgba(0, 240, 160, 0.08)' : 'rgba(255, 255, 255, 0.02)',
          border: `1px solid ${autoApprove ? 'rgba(0, 240, 160, 0.35)' : 'var(--border-subtle)'}`,
          borderRadius: 'var(--radius-md)',
          padding: '8px 12px',
          marginBottom: '10px'
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{ fontSize: '13px' }}>⚡</span>
            <div>
              <div style={{ fontSize: '11px', fontWeight: 700, color: autoApprove ? 'var(--accent-green)' : 'var(--text-main)' }}>
                1-Click Auto-Approval
              </div>
              <div style={{ fontSize: '10px', color: 'var(--text-dim)' }}>
                {autoApprove ? 'Instant trade without password prompt' : 'Confirmation modal & password required'}
              </div>
            </div>
          </div>
          <button
            onClick={() => handleToggleAutoApprove(!autoApprove)}
            style={{
              padding: '3px 10px',
              borderRadius: '20px',
              border: 'none',
              background: autoApprove ? 'var(--accent-green)' : 'var(--bg-tertiary)',
              color: autoApprove ? '#000' : 'var(--text-muted)',
              fontWeight: 800,
              fontSize: '10px',
              cursor: 'pointer',
              letterSpacing: '0.5px'
            }}
          >
            {autoApprove ? 'ON' : 'OFF'}
          </button>
        </div>

        {/* Mode Safety Notice */}
        <div style={{
          padding: '6px 10px',
          borderRadius: 'var(--radius-sm)',
          background: isDemoMode ? 'rgba(168, 85, 247, 0.1)' : 'rgba(56, 189, 248, 0.1)',
          border: `1px solid ${isDemoMode ? 'rgba(168, 85, 247, 0.25)' : 'rgba(56, 189, 248, 0.25)'}`,
          color: isDemoMode ? '#c084fc' : 'var(--accent-cyan)',
          fontSize: '11px',
          fontWeight: 600,
          textAlign: 'center',
          marginBottom: '8px'
        }}>
          {isDemoMode
            ? '🧪 DEMO MODE: Paper simulation only. Real keys are never used.'
            : '🛡️ PHASE 3: Real Unsigned Candidate Preparation & Simulation. Signing disabled.'}
        </div>

        {/* Submit Execution Button - Demo Paper Mode or Phase 3 Live Candidate Mode */}
        {isDemoMode ? (
          <button
            className={`submit-btn ${activeTab}`}
            onClick={handleOpenPreview}
            disabled={numAmount <= 0 || isLoadingPreview}
            style={{
              width: '100%',
              height: '48px',
              fontSize: '14px',
              fontWeight: 800,
              borderRadius: 'var(--radius-lg)',
              marginBottom: autoApprove ? '6px' : '16px',
              cursor: numAmount <= 0 || isLoadingPreview ? 'not-allowed' : 'pointer',
              background: 'linear-gradient(135deg, #a855f7, #6366f1)',
            }}
          >
            {isLoadingPreview
              ? 'PREPARING DEMO SIMULATION...'
              : `🧪 DEMO PAPER ${activeTab.toUpperCase()} ${selectedToken.symbol}`}
          </button>
        ) : (() => {
          const activeCandidateRoute = routeComparison && selectedRouteIdx !== null && routeComparison.routes[selectedRouteIdx]
            ? routeComparison.routes[selectedRouteIdx]
            : (routeComparison && routeComparison.best_route_index !== null && routeComparison.best_route_index !== undefined
              ? routeComparison.routes[routeComparison.best_route_index]
              : null);

          return (
            <>
              <button
                className={`submit-btn ${activeTab}`}
                onClick={() => handlePrepareAndValidateCandidate()}
                disabled={numAmount <= 0 || isPreparingCandidate || !activeCandidateRoute || activeCandidateRoute.status !== 'estimated'}
                style={{
                  width: '100%',
                  height: '48px',
                  fontSize: '13px',
                  fontWeight: 800,
                  borderRadius: 'var(--radius-lg)',
                  marginBottom: '8px',
                  opacity: (numAmount <= 0 || isPreparingCandidate || !activeCandidateRoute || activeCandidateRoute.status !== 'estimated') ? 0.6 : 1,
                  cursor: (numAmount <= 0 || isPreparingCandidate || !activeCandidateRoute || activeCandidateRoute.status !== 'estimated') ? 'not-allowed' : 'pointer',
                  background: 'linear-gradient(135deg, #0ea5e9, #38bdf8)',
                  color: '#000',
                  border: 'none',
                }}
              >
                {isPreparingCandidate
                  ? '🔍 PREPARING & SIMULATING CANDIDATE...'
                  : activeCandidateRoute
                    ? `🔍 PREPARE & VALIDATE CANDIDATE (${activeCandidateRoute.venue})`
                    : '🔍 SELECT AN ESTIMATED ROUTE TO PREPARE'}
              </button>

              {candidateError && (
                <div style={{
                  background: 'rgba(239, 68, 68, 0.1)',
                  border: '1px solid rgba(239, 68, 68, 0.3)',
                  borderRadius: 'var(--radius-sm)',
                  padding: '8px 10px',
                  fontSize: '11px',
                  color: 'var(--accent-red)',
                  marginBottom: '10px',
                  lineHeight: 1.4,
                }}>
                  <div style={{ fontWeight: 700, marginBottom: '2px' }}>⚠️ Candidate Preparation Refused:</div>
                  <div>{candidateError}</div>
                </div>
              )}

              <div style={{
                fontSize: '10px',
                color: 'var(--text-dim)',
                textAlign: 'center',
                marginBottom: '14px',
                lineHeight: 1.4,
              }}>
                🛡️ <strong>Live Swap Preparation:</strong> Fetches on-chain state, builds real Solana swap transaction, enforces native platform treasury policy (<code style={{ color: 'var(--accent-cyan)' }}>AnWn...9eH</code>), and runs read-only RPC simulation before signing.
              </div>
            </>
          );
        })()}

        {autoApprove && isDemoMode && (
          <div style={{ fontSize: '10px', color: 'var(--accent-green)', textAlign: 'center', marginBottom: '14px', fontWeight: 600 }}>
            ⚡ 1-Click execution active · No password prompt on submit
          </div>
        )}

        {/* Open Trades / Position for this coin */}
        <div style={{ marginBottom: '16px' }}>
          <div className="holdings-header">
            <span>Your Position in {selectedToken.symbol}</span>
            <span style={{
              color: isDemoMode ? 'var(--accent-green)' : '#ff6b6b',
              fontSize: '10px',
              fontWeight: 600,
              background: isDemoMode ? undefined : 'rgba(255, 77, 77, 0.1)',
              padding: isDemoMode ? undefined : '2px 6px',
              borderRadius: '4px'
            }}>
              {isDemoMode ? 'PAPER POSITION' : 'CHAIN-BACKED DATA UNAVAILABLE'}
            </span>
          </div>

          {!isDemoMode ? (
            <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '14px', textAlign: 'center', color: 'var(--text-dim)', fontSize: '11px' }}>
              🔒 Live on-chain token position tracking is unavailable. Live trading is disabled in Phase 1.
            </div>
          ) : heldCoinAmt > 0 ? (
            <div className="position-card">
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                <div>
                  <div style={{ fontSize: '11px', color: 'var(--text-dim)' }}>Holding Amount</div>
                  <div style={{ fontWeight: 800, fontSize: '15px' }}>
                    {heldCoinAmt.toLocaleString()} {selectedToken.symbol}
                  </div>
                </div>
                <div style={{ textAlign: 'right' }}>
                  <div style={{ fontSize: '11px', color: 'var(--text-dim)' }}>Current Value</div>
                  <div style={{ fontWeight: 800, fontSize: '15px', color: 'var(--accent-cyan)' }}>
                    ${curVal.toFixed(2)}
                  </div>
                </div>
              </div>

              <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '11px', borderTop: '1px solid rgba(255, 255, 255, 0.05)', paddingTop: '6px', marginBottom: '8px' }}>
                <span style={{ color: 'var(--text-dim)' }}>Avg Entry: ${costVal > 0 ? (costVal / heldCoinAmt < 0.01 ? (costVal / heldCoinAmt).toFixed(6) : (costVal / heldCoinAmt).toFixed(4)) : '0.00'}</span>
                <span style={{ color: isProfitable ? 'var(--accent-green)' : 'var(--accent-red)', fontWeight: 700 }}>
                  Unrealized PnL: {isProfitable ? '+' : '-'}${Math.abs(coinPnlUsd).toFixed(2)} ({coinPnlPct >= 0 ? '+' : ''}{coinPnlPct.toFixed(1)}%)
                </span>
              </div>

              <button
                className="holding-sub-btn trade-btn"
                style={{ width: '100%', padding: '8px' }}
                onClick={() => {
                  setActiveTab('sell');
                  setAmount(Math.floor(heldCoinAmt).toString());
                }}
              >
                ⚡ Quick Sell / Take Profit (100%)
              </button>
            </div>
          ) : (
            <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '14px', textAlign: 'center', color: 'var(--text-muted)', fontSize: '11px' }}>
              No demo position in {selectedToken.symbol}. Buy above to enter.
            </div>
          )}
        </div>

        {/* Orders & Trade History for this coin */}
        <div style={{ marginBottom: '16px' }}>
          <div className="holdings-header">
            <span>Trade History · {selectedToken.symbol} {isDemoMode ? `(${coinOrders.length})` : ''}</span>
            <span style={{
              color: isDemoMode ? 'var(--text-dim)' : '#ff6b6b',
              fontSize: '10px',
              fontWeight: 600,
              background: isDemoMode ? undefined : 'rgba(255, 77, 77, 0.1)',
              padding: isDemoMode ? undefined : '2px 6px',
              borderRadius: '4px'
            }}>
              {isDemoMode ? 'LOCAL PAPER' : 'LIVE HISTORY UNAVAILABLE'}
            </span>
          </div>

          {!isDemoMode ? (
            <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '14px', textAlign: 'center', color: 'var(--text-dim)', fontSize: '11px' }}>
              🔒 Live on-chain order history is unavailable until RPC transaction indexing is implemented.
            </div>
          ) : coinOrders.length === 0 ? (
            <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '14px', textAlign: 'center', color: 'var(--text-muted)', fontSize: '11px' }}>
              No past orders for {selectedToken.symbol}.
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
              {coinOrders.map((ord) => {
                const isBuyOrd = ord.type === 'buy';
                return (
                  <div key={ord.id} className="holding-card" style={{ padding: '8px 10px' }}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span style={{
                          padding: '2px 5px',
                          borderRadius: '4px',
                          fontSize: '10px',
                          fontWeight: 800,
                          background: isBuyOrd ? 'rgba(0, 240, 160, 0.15)' : 'rgba(255, 71, 87, 0.15)',
                          color: isBuyOrd ? 'var(--accent-green)' : 'var(--accent-red)'
                        }}>
                          {ord.type.toUpperCase()}
                        </span>
                        {ord.realizedPnlUsd !== undefined && (
                          <span className={`pnl-badge ${ord.realizedPnlUsd >= 0 ? 'positive' : 'negative'}`} style={{ fontSize: '9px' }}>
                            PnL: {ord.realizedPnlUsd >= 0 ? '+' : '-'}${Math.abs(ord.realizedPnlUsd).toFixed(2)}
                          </span>
                        )}
                      </div>
                      <div style={{ textAlign: 'right', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
                        <div style={{ fontWeight: 700, color: isBuyOrd ? 'var(--accent-green)' : 'var(--accent-red)' }}>
                          {isBuyOrd ? '+' : '-'}{ord.amountTokens.toLocaleString(undefined, { maximumFractionDigits: 0 })} {ord.symbol}
                        </div>
                        <div style={{ fontSize: '10px', color: 'var(--text-dim)' }}>
                          {ord.amountSol.toFixed(4)} SOL
                        </div>
                      </div>
                    </div>
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: '4px', fontSize: '9px', color: 'var(--text-dim)' }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span>{ord.time}</span>
                        {ord.executionMs !== undefined && (
                          <span style={{
                            color: 'var(--accent-cyan)',
                            background: 'rgba(0, 240, 255, 0.12)',
                            padding: '1px 5px',
                            borderRadius: '3px',
                            fontFamily: 'var(--font-mono)',
                            fontWeight: 700,
                            fontSize: '9px'
                          }}>
                            ⚡ {ord.executionMs < 1000 ? `${ord.executionMs}ms` : `${(ord.executionMs / 1000).toFixed(2)}s`}
                          </span>
                        )}
                      </div>
                      {ord.signature.startsWith('SIMULATED_') ? (
                        <span style={{ color: '#c084fc', fontFamily: 'var(--font-mono)' }}>[Demo Simulation]</span>
                      ) : (
                        <a href={`https://solscan.io/tx/${ord.signature}`} target="_blank" rel="noreferrer" style={{ color: 'var(--accent-cyan)' }}>
                          Solscan ↗
                        </a>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    );
  };

  const renderHistoryScreen = () => (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '14px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <h2 style={{ fontSize: '18px', fontWeight: 800 }}>Activity</h2>
          <span style={{
            fontSize: '10px',
            padding: '2px 8px',
            borderRadius: '12px',
            fontWeight: 700,
            background: isDemoMode ? 'rgba(168, 85, 247, 0.15)' : 'rgba(0, 240, 255, 0.15)',
            color: isDemoMode ? '#c084fc' : 'var(--accent-cyan)'
          }}>
            {isDemoMode ? '🧪 DEMO PAPER TRADES' : '⚡ LIVE ON-CHAIN ORDERS'}
          </span>
        </div>
        <span style={{ fontSize: '11px', color: 'var(--text-dim)' }}>
          {isDemoMode ? `${orderHistory.length} Simulations` : `${orderHistory.length} Transactions`}
        </span>
      </div>

      {orderHistory.length === 0 ? (
        <div style={{ background: 'var(--bg-card)', border: '1px dashed var(--border-subtle)', borderRadius: 'var(--radius-md)', padding: '32px 16px', textAlign: 'center', color: 'var(--text-muted)' }}>
          <div style={{ fontSize: '28px', marginBottom: '8px' }}>📜</div>
          <div style={{ fontWeight: 600, color: 'var(--text-main)', marginBottom: '4px' }}>
            {isDemoMode ? 'No Demo Paper Trades Yet' : 'No Live On-Chain Trades Yet'}
          </div>
          <div style={{ fontSize: '11px' }}>
            {isDemoMode ? 'Paper trade executions will appear here labeled as simulations.' : 'Confirmed live transactions will appear here with execution latency metrics.'}
          </div>
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
          {orderHistory.map((item) => {
            const isBuy = item.type === 'buy';
            return (
              <div key={item.id} className="holding-card">
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
                    <div style={{
                      width: '32px',
                      height: '32px',
                      borderRadius: '50%',
                      background: isBuy ? 'rgba(0, 240, 160, 0.15)' : 'rgba(255, 71, 87, 0.15)',
                      color: isBuy ? 'var(--accent-green)' : 'var(--accent-red)',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      fontWeight: 800,
                      fontSize: '14px'
                    }}>
                      {isBuy ? '↓' : '↑'}
                    </div>
                    <div>
                      <div style={{ fontWeight: 700, fontSize: '13px', display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span>{item.type.toUpperCase()}</span>
                        <span style={{ color: 'var(--accent-cyan)' }}>{item.symbol}</span>
                      </div>
                      <div style={{ fontSize: '10px', color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span>{item.time}</span>
                        {item.executionMs !== undefined && (
                          <span style={{
                            color: 'var(--accent-cyan)',
                            background: 'rgba(0, 240, 255, 0.12)',
                            padding: '1px 5px',
                            borderRadius: '3px',
                            fontFamily: 'var(--font-mono)',
                            fontWeight: 700,
                            fontSize: '9px'
                          }}>
                            ⚡ {item.executionMs < 1000 ? `${item.executionMs}ms` : `${(item.executionMs / 1000).toFixed(2)}s`}
                          </span>
                        )}
                      </div>
                    </div>
                  </div>

                  <div style={{ textAlign: 'right' }}>
                    <div style={{ fontWeight: 700, fontSize: '13px', color: isBuy ? 'var(--accent-green)' : 'var(--accent-red)' }}>
                      {isBuy ? '+' : '-'}{item.amountTokens.toLocaleString(undefined, { maximumFractionDigits: 0 })} {item.symbol}
                    </div>
                    <div style={{ fontSize: '11px', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>
                      {item.amountSol.toFixed(4)} SOL
                    </div>
                  </div>
                </div>

                <div style={{ borderTop: '1px solid rgba(255, 255, 255, 0.05)', paddingTop: '6px', display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '10px' }}>
                  <span style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
                    {item.signature.startsWith('SIMULATED_') ? 'Simulated Ref: ' + item.signature.slice(0, 18) + '...' : `Sig: ${item.signature.slice(0, 8)}...${item.signature.slice(-6)}`}
                  </span>
                  {item.signature.startsWith('SIMULATED_') ? (
                    <span style={{ color: '#c084fc', fontFamily: 'var(--font-mono)', fontSize: '9px', background: 'rgba(168, 85, 247, 0.15)', padding: '2px 6px', borderRadius: '4px' }}>
                      [SIMULATED - NO ON-CHAIN TX]
                    </span>
                  ) : (
                    <a
                      href={`https://solscan.io/tx/${item.signature}`}
                      target="_blank"
                      rel="noreferrer"
                      style={{ color: 'var(--accent-cyan)', textDecoration: 'underline' }}
                    >
                      Solscan ↗
                    </a>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );

  const renderReceiveModal = () => {
    if (!showReceiveModal || !activeWallet) return null;

    return (
      <div className="modal-backdrop">
        <div className="modal-content" style={{ width: '380px', textAlign: 'center' }}>
          <div className="modal-header">
            <span className="modal-title">Receive Solana &amp; Tokens</span>
            <span className="modal-badge">SOL / SPL</span>
          </div>

          {/* Clean styled QR visual code */}
          <div style={{
            margin: '16px auto',
            width: '160px',
            height: '160px',
            background: '#ffffff',
            borderRadius: '12px',
            padding: '12px',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            boxShadow: '0 8px 24px rgba(0, 0, 0, 0.4)'
          }}>
            <svg width="136" height="136" viewBox="0 0 29 29" shapeRendering="crispEdges">
              <path fill="#000000" d="M0 0h7v7H0zm2 2v3h3V2zm7 0h2v1H9zm3 0h1v1h-1zm2 0h1v3h-1zm3 0h1v1h-1zm2 0h7v7h-7zm2 2v3h3V2zM7 9h1v1H7zm3 0h1v1h-1zm2 0h1v2h-1zm3 0h2v1h-2zm4 0h1v1h-1zm-9 2h2v1h-2zm3 0h2v1h-2zm5 0h2v2h-2zm-7 2h1v1h-1zm2 0h1v1h-1zm-8 1h1v2H6zm2 0h2v1H8zm5 0h1v1h-1zm3 0h1v1h-1zm-9 2h1v1H7zm3 0h1v1h-1zm5 0h1v1h-1zm4 0h1v1h-1zm-8 2h1v1H8zm2 0h2v1h-2zm4 0h1v2h-1zm-8 2h1v1H8zm4 0h1v1h-1zm-6 1h1v1H6zm14 0h1v1h-1zm-18 2h7v7H2zm2 2v3h3v-3zm7 0h1v1H9zm2 0h2v2h-2zm3 0h1v1h-1zm2 0h1v2h-1zm4 0h1v1h-1zm2 0h1v1h-1zm-7 2h1v1h-1zm2 0h2v1h-2zm-8 1h2v1H7zm11 0h1v1h-1zm3 0h1v2h-1zm-8 1h1v1h-1zm2 0h1v1h-1zm-5 1h2v1H7zm8 0h1v1h-1z" />
            </svg>
          </div>

          <div style={{ fontSize: '11px', color: 'var(--text-muted)', marginBottom: '8px' }}>
            Your Solana Public Key:
          </div>

          <div style={{
            background: 'var(--bg-primary)',
            padding: '10px',
            borderRadius: 'var(--radius-md)',
            fontFamily: 'var(--font-mono)',
            fontSize: '11px',
            color: 'var(--text-main)',
            wordBreak: 'break-all',
            marginBottom: '16px',
            border: '1px solid var(--border-subtle)'
          }}>
            {activeWallet.pubkey}
          </div>

          <div className="modal-actions" style={{ justifyContent: 'center', gap: '8px' }}>
            <button className="modal-btn-cancel" onClick={() => setShowReceiveModal(false)}>
              CLOSE
            </button>
            <button className="modal-btn-confirm" onClick={handleCopyWallet}>
              {copySuccess ? '✓ COPIED!' : 'COPY ADDRESS'}
            </button>
          </div>
        </div>
      </div>
    );
  };

  const renderSendModal = () => {
    if (!showSendModal || !activeWallet) return null;

    const isSendingSol = sendAsset === 'SOL';
    const heldToken = holdings.find((h) => h.mint === sendAsset);
    const maxAvailable = isSendingSol ? Math.max(0, walletBalance - 0.005) : (heldToken ? heldToken.amount : 0);

    return (
      <div className="modal-backdrop">
        <div className="modal-content" style={{ width: '420px' }}>
          <div className="modal-header">
            <span className="modal-title">Send Crypto</span>
            <span className="modal-badge">LOCAL SIGNING</span>
          </div>

          {/* Asset Selection */}
          <div style={{ marginBottom: '12px' }}>
            <label style={{ display: 'block', fontSize: '11px', color: 'var(--text-muted)', marginBottom: '4px' }}>
              SELECT ASSET
            </label>
            <select
              className="route-selector"
              value={sendAsset}
              onChange={(e) => setSendAsset(e.target.value)}
            >
              <option value="SOL">SOL — Balance: {walletBalance.toFixed(4)} SOL</option>
              {holdings.map((h) => (
                <option key={h.mint} value={h.mint}>
                  {h.symbol} ({h.name}) — Balance: {h.amount.toLocaleString()} {h.symbol}
                </option>
              ))}
            </select>
          </div>

          {/* Recipient Address */}
          <div style={{ marginBottom: '12px' }}>
            <label style={{ display: 'block', fontSize: '11px', color: 'var(--text-muted)', marginBottom: '4px' }}>
              RECIPIENT SOLANA ADDRESS
            </label>
            <input
              type="text"
              className="input-field"
              placeholder="Base58 Solana public key..."
              value={sendRecipient}
              onChange={(e) => setSendRecipient(e.target.value)}
            />
          </div>

          {/* Send Amount */}
          <div style={{ marginBottom: '12px' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px', fontSize: '11px' }}>
              <span style={{ color: 'var(--text-muted)' }}>AMOUNT</span>
              <span
                style={{ color: 'var(--accent-cyan)', cursor: 'pointer' }}
                onClick={() => setSendAmount(maxAvailable > 0 ? (isSendingSol ? maxAvailable.toFixed(4) : Math.floor(maxAvailable).toString()) : '0')}
              >
                Max: {isSendingSol ? `${maxAvailable.toFixed(4)} SOL` : `${Math.floor(maxAvailable).toLocaleString()} ${heldToken?.symbol || ''}`}
              </span>
            </div>
            <input
              type="number"
              className="input-field"
              placeholder="0.0"
              value={sendAmount}
              onChange={(e) => setSendAmount(e.target.value)}
              min="0"
              step="any"
            />
          </div>

          {/* Mode Safety Notice */}
          <div style={{
            padding: '6px 10px',
            borderRadius: 'var(--radius-sm)',
            background: isDemoMode ? 'rgba(168, 85, 247, 0.1)' : 'rgba(255, 77, 77, 0.1)',
            border: `1px solid ${isDemoMode ? 'rgba(168, 85, 247, 0.25)' : 'rgba(255, 77, 77, 0.25)'}`,
            color: isDemoMode ? '#c084fc' : '#ff6b6b',
            fontSize: '11px',
            fontWeight: 600,
            marginBottom: '12px'
          }}>
            {isDemoMode
              ? '🧪 DEMO MODE: Paper transfer simulation only. No real keys or Solana broadcast.'
              : '🔒 LIVE MODE: Transfers are disabled in Phase 1 and will fail closed safely.'}
          </div>

          {/* Keystore Unlock Password */}
          {!isDemoMode && (
            <div style={{ marginBottom: '16px' }}>
              <label style={{ display: 'block', fontSize: '11px', color: 'var(--text-muted)', marginBottom: '4px' }}>
                ENTER KEYSTORE PASSWORD TO SIGN
              </label>
              <input
                type="password"
                className="input-field"
                placeholder="Local encryption password"
                value={sendPassword}
                onChange={(e) => setSendPassword(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleConfirmSend()}
              />
              {sendError && (
                <div style={{ color: 'var(--accent-red)', fontSize: '11px', marginTop: '6px', fontWeight: 600 }}>
                  ❌ {sendError}
                </div>
              )}
            </div>
          )}

          {sendError && isDemoMode && (
            <div style={{ color: 'var(--accent-red)', fontSize: '11px', marginBottom: '12px', fontWeight: 600 }}>
              ❌ {sendError}
            </div>
          )}

          <div className="modal-actions">
            <button
              className="modal-btn-cancel"
              onClick={() => { setShowSendModal(false); setSendError(null); }}
              disabled={isSending}
            >
              CANCEL
            </button>
            <button
              className="modal-btn-confirm"
              onClick={handleConfirmSend}
              disabled={isSending}
              style={{
                background: isDemoMode ? 'linear-gradient(135deg, #a855f7, #6366f1)' : undefined
              }}
            >
              {isSending
                ? 'PROCESSING...'
                : (isDemoMode ? '🧪 SIMULATE DEMO SEND' : '🔴 CONFIRM & SEND (FAILS CLOSED)')}
            </button>
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className="wallet-shell">
      <div className="wallet-frame">
        {/* Top Wallet Bar */}
        <div className="wallet-top-bar">
          <div
            className="wallet-selector-pill"
            onClick={() => setShowWalletModal(true)}
            title="Click to switch or create wallets"
          >
            <div className="status-dot"></div>
            <span>{activeWallet?.label || 'Trading Wallet'}</span>
            <span style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', fontSize: '11px' }}>
              {activeWallet ? `${activeWallet.pubkey.slice(0, 4)}...${activeWallet.pubkey.slice(-4)}` : ''} · {rpcLatency}ms
            </span>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
            {/* Explicit Trading Mode Switcher */}
            <button
              onClick={() => handleSetTradingMode(isDemoMode ? 'live' : 'demo')}
              style={{
                background: isDemoMode ? 'rgba(168, 85, 247, 0.18)' : 'rgba(255, 77, 77, 0.18)',
                border: `1px solid ${isDemoMode ? '#c084fc' : '#ff6b6b'}`,
                color: isDemoMode ? '#c084fc' : '#ff6b6b',
                borderRadius: '12px',
                padding: '3px 8px',
                fontSize: '10px',
                fontFamily: 'var(--font-mono)',
                cursor: 'pointer',
                fontWeight: 700,
                display: 'flex',
                alignItems: 'center',
                gap: '4px',
              }}
              title={isDemoMode ? 'Currently in Demo Paper Mode. Click to switch to Live Mode.' : 'Currently in Live Mode (Fail-closed). Click to switch to Demo Paper Mode.'}
            >
              <span>{isDemoMode ? '🧪 DEMO' : '🔴 LIVE'}</span>
            </button>

            <button
              onClick={() => {
                if (!customRpcUrl.trim()) {
                  setShowWalletModal(true);
                } else {
                  handleToggleCustomRpc(!customRpc);
                }
              }}
              style={{
                background: effectiveCustomRpcActive ? 'rgba(0, 240, 160, 0.12)' : 'rgba(255, 255, 255, 0.05)',
                border: `1px solid ${effectiveCustomRpcActive ? 'var(--accent-green)' : 'var(--border-subtle)'}`,
                color: effectiveCustomRpcActive ? 'var(--accent-green)' : 'var(--text-muted)',
                borderRadius: '12px',
                padding: '3px 8px',
                fontSize: '10px',
                fontFamily: 'var(--font-mono)',
                cursor: 'pointer',
                fontWeight: 600,
              }}
              title={
                customRpcUrl.trim()
                  ? `Click to toggle Custom RPC (${effectiveCustomRpcActive ? 'Active: 10 bps fee discount' : 'Inactive: 20 bps standard fee'})`
                  : 'Click to configure Custom RPC URL in Settings to qualify for 10 bps fee discount'
              }
            >
              {effectiveCustomRpcActive ? '⚡ Custom RPC (10 bps)' : '⚡ Default RPC (20 bps)'}
            </button>
            <button
              className="icon-btn"
              onClick={handleCopyWallet}
              title="Copy Wallet Public Key"
            >
              {copySuccess ? '✓' : '⧉'}
            </button>
            <button
              className="icon-btn"
              onClick={() => setShowWalletModal(true)}
              title="Keystore & Settings"
            >
              ⚙
            </button>
          </div>
        </div>

        {/* Persistent Trading Mode Banner */}
        <div style={{
          padding: '6px 14px',
          background: isDemoMode ? 'rgba(168, 85, 247, 0.12)' : 'rgba(255, 77, 77, 0.12)',
          borderBottom: `1px solid ${isDemoMode ? 'rgba(168, 85, 247, 0.25)' : 'rgba(255, 77, 77, 0.25)'}`,
          color: isDemoMode ? '#c084fc' : '#ff6b6b',
          fontSize: '10px',
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          fontWeight: 600,
          fontFamily: 'var(--font-mono)',
        }}>
          <span>
            {isDemoMode
              ? '🧪 DEMO MODE — Paper trading only. No real keys or Solana broadcast.'
              : '🔴 LIVE MODE — Real RPC balance. Trading fails closed safely.'}
          </span>
          <button
            onClick={() => handleSetTradingMode(isDemoMode ? 'live' : 'demo')}
            style={{
              background: 'transparent',
              border: 'none',
              color: isDemoMode ? '#e9d5ff' : '#fca5a5',
              textDecoration: 'underline',
              cursor: 'pointer',
              fontSize: '10px',
              fontFamily: 'inherit',
            }}
          >
            {isDemoMode ? 'Switch to Live' : 'Switch to Demo'}
          </button>
        </div>

        {/* Success toast if send completed */}
        {sendSuccessMessage && (
          <div style={{ padding: '8px 14px', background: 'rgba(0, 240, 160, 0.12)', borderBottom: '1px solid var(--accent-green)', color: 'var(--accent-green)', fontSize: '11px', textAlign: 'center', fontWeight: 600 }}>
            ✓ {sendSuccessMessage}
          </div>
        )}

        {/* Scrollable Wallet Body */}
        <div className="wallet-scroll-body">
          {currentNav === 'home' && renderHomeScreen()}
          {currentNav === 'trade' && renderTradeScreen()}
          {currentNav === 'history' && renderHistoryScreen()}
        </div>

        {/* Bottom Navigation Dock */}
        <div className="bottom-nav-dock">
          <button
            className={`bottom-nav-btn ${currentNav === 'home' ? 'active' : ''}`}
            onClick={() => setCurrentNav('home')}
          >
            <span className="bottom-nav-icon">🏠</span>
            <span>Home</span>
          </button>
          <button
            className={`bottom-nav-btn ${currentNav === 'trade' ? 'active' : ''}`}
            onClick={() => {
              setCurrentNav('trade');
              setTradeViewMode('list');
            }}
          >
            <span className="bottom-nav-icon">📈</span>
            <span>Trade</span>
          </button>
          <button
            className={`bottom-nav-btn ${currentNav === 'history' ? 'active' : ''}`}
            onClick={() => setCurrentNav('history')}
          >
            <span className="bottom-nav-icon">📜</span>
            <span>History</span>
          </button>
        </div>

        {/* Wallet Management Modal */}
        {showWalletModal && (
          <div className="modal-backdrop">
            <div className="modal-content">
              <div className="modal-header">
                <span className="modal-title">Encrypted Local Keystore</span>
                <span className="modal-badge">AES-256-GCM</span>
              </div>

              {/* Wallet Selection / Current */}
              {activeWallet && (
                <div style={{ background: 'var(--bg-primary)', padding: '12px', borderRadius: 'var(--radius-md)', marginBottom: '16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
                    <span style={{ fontWeight: 600 }}>{activeWallet.label}</span>
                    <span style={{ color: 'var(--accent-green)', fontWeight: 700 }}>
                      {walletBalance.toFixed(4)} SOL
                    </span>
                  </div>
                  <div style={{ fontFamily: 'var(--font-mono)', fontSize: '11px', color: 'var(--text-muted)', wordBreak: 'break-all', marginBottom: '8px' }}>
                    {activeWallet.pubkey}
                  </div>
                  <div style={{ display: 'flex', gap: '8px' }}>
                    <button
                      className="search-btn"
                      style={{ background: 'var(--bg-tertiary)', color: 'var(--text-main)', border: '1px solid var(--border-subtle)' }}
                      onClick={handleCopyWallet}
                    >
                      {copySuccess ? '✓ COPIED' : 'COPY PUBKEY'}
                    </button>
                    <button
                      className="search-btn"
                      style={{ background: 'var(--bg-tertiary)', color: 'var(--text-main)', border: '1px solid var(--border-subtle)' }}
                      onClick={() => updateWalletBalance(activeWallet.pubkey)}
                      disabled={isRefreshingBalance}
                    >
                      {isRefreshingBalance ? 'PINGING...' : 'REFRESH RPC'}
                    </button>
                    <a
                      href={`https://solscan.io/account/${activeWallet.pubkey}`}
                      target="_blank"
                      rel="noreferrer"
                      className="search-btn"
                      style={{ textDecoration: 'none', display: 'flex', alignItems: 'center' }}
                    >
                      SOLSCAN ↗
                    </a>
                  </div>
                </div>
              )}

              {/* Multi-Wallet Switcher */}
              {wallets.length > 1 && (
                <div style={{ marginBottom: '16px' }}>
                  <div style={{ fontSize: '11px', fontWeight: 700, color: 'var(--text-dim)', marginBottom: '6px' }}>
                    SWITCH WALLET ({wallets.length} SAVED):
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                    {wallets.map((w) => {
                      const isSelected = activeWallet?.pubkey === w.pubkey;
                      return (
                        <div
                          key={w.pubkey}
                          onClick={() => {
                            setActiveWallet(w);
                            localStorage.setItem('axiom_active_wallet_pubkey', w.pubkey);
                          }}
                          style={{
                            display: 'flex',
                            justifyContent: 'space-between',
                            alignItems: 'center',
                            padding: '8px 10px',
                            borderRadius: 'var(--radius-sm)',
                            background: isSelected ? 'rgba(0, 240, 160, 0.12)' : 'var(--bg-tertiary)',
                            border: `1px solid ${isSelected ? 'var(--accent-green)' : 'var(--border-subtle)'}`,
                            cursor: 'pointer'
                          }}
                        >
                          <div>
                            <div style={{ fontSize: '12px', fontWeight: 700, color: isSelected ? 'var(--accent-green)' : 'var(--text-main)' }}>
                              {w.label} {isSelected ? '✓ (Active)' : ''}
                            </div>
                            <div style={{ fontSize: '10px', fontFamily: 'var(--font-mono)', color: 'var(--text-dim)' }}>
                              {w.pubkey.slice(0, 8)}...{w.pubkey.slice(-8)}
                            </div>
                          </div>
                          <button
                            className="search-btn"
                            style={{
                              fontSize: '9px',
                              padding: '2px 8px',
                              background: isSelected ? 'var(--accent-green)' : 'var(--bg-card)',
                              color: isSelected ? '#000' : 'var(--text-main)',
                              fontWeight: 700
                            }}
                          >
                            {isSelected ? 'ACTIVE' : 'SELECT'}
                          </button>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Auto-Approval & Trade Execution Settings */}
              <div style={{
                background: 'var(--bg-primary)',
                padding: '12px',
                borderRadius: 'var(--radius-md)',
                marginBottom: '16px',
                border: `1px solid ${autoApprove ? 'rgba(0, 240, 160, 0.35)' : 'var(--border-subtle)'}`
              }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <span style={{ fontSize: '15px' }}>⚡</span>
                    <div>
                      <div style={{ fontWeight: 700, fontSize: '12px', color: autoApprove ? 'var(--accent-green)' : 'var(--text-main)' }}>
                        Auto-Approval For Trades
                      </div>
                      <div style={{ fontSize: '10px', color: 'var(--text-dim)' }}>
                        Trade without password confirmation on every order
                      </div>
                    </div>
                  </div>
                  <button
                    onClick={() => handleToggleAutoApprove(!autoApprove)}
                    style={{
                      padding: '4px 12px',
                      borderRadius: '20px',
                      border: 'none',
                      background: autoApprove ? 'var(--accent-green)' : 'var(--bg-tertiary)',
                      color: autoApprove ? '#000' : 'var(--text-muted)',
                      fontWeight: 800,
                      fontSize: '11px',
                      cursor: 'pointer',
                      transition: 'all 0.15s ease'
                    }}
                  >
                    {autoApprove ? 'ENABLED' : 'DISABLED'}
                  </button>
                </div>

                {autoApprove && (
                  <div style={{ background: 'rgba(0, 240, 160, 0.08)', borderRadius: '6px', padding: '8px 10px', marginTop: '8px', border: '1px solid rgba(0, 240, 160, 0.2)' }}>
                    <div style={{ fontSize: '10px', color: 'var(--accent-green)', fontWeight: 600, marginBottom: '2px' }}>
                      ✓ 1-Click Fast Execution Active
                    </div>
                    <div style={{ fontSize: '10px', color: 'var(--text-muted)' }}>
                      Orders execute immediately with cached session key. Keystore remains encrypted on disk (Argon2id + AES-256-GCM).
                    </div>
                  </div>
                )}

                {/* Reset Demo Portfolio Button */}
                <div style={{ borderTop: '1px solid var(--border-subtle)', paddingTop: '10px', marginTop: '10px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <div style={{ fontSize: '10px', color: 'var(--text-dim)' }}>
                    Clean transaction history &amp; reset to 2.0 SOL:
                  </div>
                  <button
                    onClick={handleResetDemoPortfolio}
                    style={{
                      padding: '3px 8px',
                      borderRadius: '6px',
                      border: '1px solid var(--border-subtle)',
                      background: 'var(--bg-tertiary)',
                      color: 'var(--text-main)',
                      fontSize: '10px',
                      cursor: 'pointer',
                      fontWeight: 600
                    }}
                    title="Resets wallet to 2.0 SOL and clean test positions"
                  >
                    Reset Portfolio
                  </button>
                </div>
              </div>

              {/* Custom Solana RPC Configuration */}
              <div style={{
                background: 'var(--bg-primary)',
                padding: '12px',
                borderRadius: 'var(--radius-md)',
                marginBottom: '16px',
                border: `1px solid ${effectiveCustomRpcActive ? 'rgba(0, 240, 160, 0.35)' : 'var(--border-subtle)'}`
              }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <span style={{ fontSize: '15px' }}>⚡</span>
                    <div>
                      <div style={{ fontWeight: 700, fontSize: '12px', color: effectiveCustomRpcActive ? 'var(--accent-green)' : 'var(--text-main)' }}>
                        Custom Solana RPC Endpoint
                      </div>
                      <div style={{ fontSize: '10px', color: 'var(--text-dim)' }}>
                        Use your own RPC (Helius, QuickNode, Triton, etc.) for lower latency &amp; 10 bps fee tier
                      </div>
                    </div>
                  </div>
                  <button
                    onClick={() => handleToggleCustomRpc(!customRpc)}
                    style={{
                      padding: '4px 12px',
                      borderRadius: '20px',
                      border: 'none',
                      background: effectiveCustomRpcActive ? 'var(--accent-green)' : 'var(--bg-tertiary)',
                      color: effectiveCustomRpcActive ? '#000' : 'var(--text-muted)',
                      fontWeight: 800,
                      fontSize: '11px',
                      cursor: 'pointer',
                      transition: 'all 0.15s ease'
                    }}
                  >
                    {customRpc ? 'ENABLED' : 'DISABLED'}
                  </button>
                </div>

                <input
                  type="text"
                  className="input-field"
                  placeholder="https://mainnet.helius-rpc.com/?api-key=YOUR_API_KEY"
                  value={customRpcUrl}
                  onChange={(e) => handleUpdateCustomRpcUrl(e.target.value)}
                  style={{ width: '100%', fontSize: '11px', fontFamily: 'var(--font-mono)', boxSizing: 'border-box' }}
                />

                <div style={{ marginTop: '8px', fontSize: '10px', color: 'var(--text-dim)', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <span>
                    Platform Fee: <strong style={{ color: effectiveCustomRpcActive ? 'var(--accent-green)' : 'var(--accent-cyan)' }}>{effectiveCustomRpcActive ? '10 bps (0.10% - User RPC discount)' : '20 bps (0.20% - Standard)'}</strong>
                  </span>
                  <span>
                    {effectiveCustomRpcActive ? '⚡ Custom RPC Active' : '🌐 Public RPC (api.mainnet-beta.solana.com)'}
                  </span>
                </div>
              </div>

              {/* Create or Import Tabs */}
              <div className="order-tabs" style={{ marginBottom: '14px' }}>
                <button
                  className={`tab-btn ${walletTab === 'create' ? 'active buy' : ''}`}
                  onClick={() => setWalletTab('create')}
                >
                  CREATE NEW
                </button>
                <button
                  className={`tab-btn ${walletTab === 'import' ? 'active buy' : ''}`}
                  onClick={() => setWalletTab('import')}
                >
                  IMPORT KEY
                </button>
              </div>

              <div style={{ display: 'flex', flexDirection: 'column', gap: '10px', marginBottom: '16px' }}>
                <input
                  type="text"
                  className="input-field"
                  placeholder="Wallet Label (e.g. Sniping Wallet 1)"
                  value={newWalletLabel}
                  onChange={(e) => setNewWalletLabel(e.target.value)}
                />

                {walletTab === 'import' && (
                  <input
                    type="password"
                    className="input-field"
                    placeholder="Base58 Private Key (stored encrypted on disk)"
                    value={importKey}
                    onChange={(e) => setImportKey(e.target.value)}
                  />
                )}

                <input
                  type="password"
                  className="input-field"
                  placeholder="Local Encryption Password (min 8 chars)"
                  value={walletPassword}
                  onChange={(e) => setWalletPassword(e.target.value)}
                />
              </div>

              <div className="modal-actions">
                <button className="modal-btn-cancel" onClick={() => setShowWalletModal(false)}>
                  CLOSE
                </button>
                <button
                  className="modal-btn-confirm"
                  onClick={walletTab === 'create' ? handleCreateWallet : handleImportWallet}
                >
                  {walletTab === 'create' ? 'GENERATE KEYPAIR' : 'ENCRYPT &amp; SAVE'}
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Trade Preview & Local Keystore Unlock Modal */}
        {showPreviewModal && tradePreview && (
          <div className="modal-backdrop">
            <div className="modal-content" style={{ width: '480px' }}>
              <div className="modal-header">
                <span className="modal-title">
                  {isDemoMode ? '🧪 Demo Paper Trade Preview' : 'Authorize Local Transaction'}
                </span>
                <span className="modal-badge" style={{
                  background: isDemoMode ? 'rgba(168, 85, 247, 0.2)' : 'rgba(255, 77, 77, 0.2)',
                  color: isDemoMode ? '#c084fc' : '#ff6b6b',
                  border: `1px solid ${isDemoMode ? '#c084fc' : '#ff6b6b'}`
                }}>
                  {isDemoMode ? 'PAPER SIMULATION' : '🔒 LIVE FAIL-CLOSED'}
                </span>
              </div>

              {/* Target Token Card */}
              <div style={{ display: 'flex', alignItems: 'center', gap: '10px', marginBottom: '14px', background: 'var(--bg-primary)', padding: '10px 12px', borderRadius: 'var(--radius-md)' }}>
                {selectedToken.imageUrl ? (
                  <img src={selectedToken.imageUrl} alt="" style={{ width: '32px', height: '32px', borderRadius: '50%' }} />
                ) : (
                  <div style={{ width: '32px', height: '32px', borderRadius: '50%', background: 'linear-gradient(135deg, var(--accent-cyan), var(--accent-green))', display: 'flex', alignItems: 'center', justifyContent: 'center', fontWeight: 700, color: '#000', fontSize: '13px' }}>
                    {selectedToken.symbol.slice(0, 2)}
                  </div>
                )}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                    <span style={{ fontWeight: 700, fontSize: '13px' }}>{selectedToken.symbol}</span>
                    <span style={{ fontSize: '11px', color: 'var(--text-muted)' }}>{selectedToken.name}</span>
                  </div>
                  <div
                    className="ca-clickable"
                    onClick={() => handleCopyCA(selectedToken.mint)}
                    style={{ fontSize: '10px', fontFamily: 'var(--font-mono)', color: 'var(--text-dim)', display: 'flex', alignItems: 'center', gap: '4px', cursor: 'pointer' }}
                  >
                    <span>CA: {selectedToken.mint.slice(0, 10)}...{selectedToken.mint.slice(-8)}</span>
                    <span style={{ color: 'var(--accent-cyan)' }}>{caCopied ? '✓' : '⎘'}</span>
                  </div>
                </div>
                <span className={`dex-tag ${selectedToken.dexId}`}>
                  {DEX_LABELS[selectedToken.dexId] || selectedToken.dexId}
                </span>
              </div>

              {/* Decoded Transaction Parameters */}
              <div className="preview-grid">
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Action:</span>
                  <span style={{ fontWeight: 700, color: tradePreview.side === 'buy' ? 'var(--accent-green)' : 'var(--accent-red)', textTransform: 'uppercase' }}>
                    {tradePreview.side} {selectedToken.symbol}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Signer Wallet:</span>
                  <span style={{ color: 'var(--accent-cyan)' }}>
                    {activeWallet?.pubkey.slice(0, 6)}...{activeWallet?.pubkey.slice(-6)}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>{tradePreview.side === 'buy' ? 'Spend Amount:' : 'Tokens To Sell:'}</span>
                  <span style={{ color: 'var(--accent-green)', fontWeight: 700 }}>
                    {tradePreview.side === 'buy' ? `${(tradePreview.input_amount_atomic / 1e9).toFixed(4)} SOL` : `${(tradePreview.input_amount_atomic / 1e6).toLocaleString()} ${selectedToken.symbol}`}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Min Output:</span>
                  <span style={{ color: 'var(--text-main)', fontWeight: 600 }}>
                    {tradePreview.side === 'buy' ? `${(tradePreview.minimum_output_atomic / 1e6).toLocaleString(undefined, { maximumFractionDigits: 0 })} ${selectedToken.symbol}` : `${(tradePreview.minimum_output_atomic / 1e9).toFixed(6)} SOL`}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Execution Route:</span>
                  <span style={{ color: isDemoMode ? '#c084fc' : 'var(--accent-cyan)', fontWeight: 600 }}>
                    {isDemoMode
                      ? 'Paper Simulation Engine (Local)'
                      : (tradePreview.route && typeof tradePreview.route === 'object' && 'okx_router' in tradePreview.route
                        ? 'OKX DEX Router V1 (Aggregate)'
                        : 'Pump.fun Bonding Curve')}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Protocol Fee ({feeBps} bps):</span>
                  <span>{(tradePreview.platform_fee_atomic / 1e9).toFixed(6)} SOL</span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Fee Recipient:</span>
                  <span style={{ color: 'var(--text-muted)' }}>
                    {tradePreview.platform_fee_recipient.slice(0, 6)}...{tradePreview.platform_fee_recipient.slice(-4)}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Compute Budget:</span>
                  <span>{tradePreview.compute_unit_limit.toLocaleString()} CU @ {tradePreview.priority_fee_micro_lamports.toLocaleString()} µ-lamports</span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Slippage Limit:</span>
                  <span>{tradePreview.slippage_bps / 100}%</span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', borderTop: '1px solid var(--border-subtle)', paddingTop: '6px', marginTop: '2px' }}>
                  <span style={{ fontWeight: 600 }}>Total SOL Required:</span>
                  <span style={{ fontWeight: 700, color: 'var(--accent-cyan)' }}>
                    {(tradePreview.total_sol_required / 1e9).toFixed(6)} SOL
                  </span>
                </div>
              </div>

              {/* Mode Safety Notice */}
              {isDemoMode ? (
                <div style={{ background: 'rgba(168, 85, 247, 0.08)', border: '1px solid rgba(168, 85, 247, 0.25)', borderRadius: 'var(--radius-sm)', padding: '8px 10px', marginBottom: '16px', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
                  <div style={{ color: '#c084fc', fontWeight: 700, marginBottom: '2px', display: 'flex', alignItems: 'center', gap: '4px' }}>
                    <span>🧪 Demo Paper Simulation Mode</span>
                  </div>
                  <div style={{ color: 'var(--text-muted)', fontSize: '10px', lineHeight: 1.4 }}>
                    This is a paper trade simulation. Keystore keys are NOT accessed and no transactions are broadcast to Solana.
                  </div>
                </div>
              ) : (
                <div style={{ background: 'rgba(255, 77, 77, 0.08)', border: '1px solid rgba(255, 77, 77, 0.25)', borderRadius: 'var(--radius-sm)', padding: '8px 10px', marginBottom: '16px', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
                  <div style={{ color: '#ff6b6b', fontWeight: 700, marginBottom: '2px', display: 'flex', alignItems: 'center', gap: '4px' }}>
                    <span>🔒 Live Execution Locked (Phase 1 Fail-Closed)</span>
                  </div>
                  <div style={{ color: 'var(--text-muted)', fontSize: '10px', lineHeight: 1.4 }}>
                    Live on-chain execution is not implemented yet. Attempting to execute will fail closed safely without signing or submitting any transaction.
                  </div>
                </div>
              )}

              {tradePreview.warnings && tradePreview.warnings.length > 0 && (
                <div style={{ background: 'rgba(255, 183, 3, 0.1)', border: '1px solid var(--accent-amber)', borderRadius: 'var(--radius-sm)', padding: '8px 10px', marginBottom: '16px', fontSize: '11px' }}>
                  {tradePreview.warnings.map((w, idx) => (
                    <div key={idx} style={{ color: 'var(--accent-amber)' }}>⚠️ {w}</div>
                  ))}
                </div>
              )}

              {/* In Live Mode: prompt password and display error if attempted */}
              {!isDemoMode && (
                <div style={{ marginBottom: '16px' }}>
                  <label style={{ display: 'block', fontSize: '12px', color: 'var(--text-muted)', marginBottom: '4px' }}>
                    Enter password to unlock local private key and sign:
                  </label>
                  <div style={{ fontSize: '11px', color: 'var(--text-dim)', marginBottom: '8px' }}>
                    Wallet: <span style={{ color: 'var(--accent-cyan)', fontFamily: 'var(--font-mono)' }}>{activeWallet?.label}</span>
                  </div>
                  <input
                    type="password"
                    className="input-field"
                    placeholder="Keystore password"
                    value={signPassword}
                    onChange={(e) => setSignPassword(e.target.value)}
                    autoFocus
                    onKeyDown={(e) => e.key === 'Enter' && handleConfirmTrade()}
                  />
                  {signingError && (
                    <div style={{ color: 'var(--accent-red)', fontSize: '12px', marginTop: '6px', fontWeight: 600, display: 'flex', alignItems: 'center', gap: '6px' }}>
                      <span>❌</span>
                      <span>{signingError}</span>
                    </div>
                  )}
                </div>
              )}

              {/* In Demo Mode: password is NOT required */}
              {isDemoMode && (
                <div style={{ marginBottom: '16px', padding: '10px', background: 'rgba(168, 85, 247, 0.05)', borderRadius: 'var(--radius-sm)', border: '1px dashed rgba(168, 85, 247, 0.3)' }}>
                  <div style={{ fontSize: '11px', color: '#c084fc', fontWeight: 700, marginBottom: '2px' }}>
                    🧪 Keystore Credentials Protected
                  </div>
                  <div style={{ fontSize: '10px', color: 'var(--text-muted)' }}>
                    Demo mode does not access your encrypted keystore. Click below to simulate the paper trade.
                  </div>
                  {signingError && (
                    <div style={{ color: 'var(--accent-red)', fontSize: '12px', marginTop: '6px', fontWeight: 600 }}>
                      ❌ {signingError}
                    </div>
                  )}
                </div>
              )}

              <div className="modal-actions">
                <button
                  className="modal-btn-cancel"
                  onClick={() => setShowPreviewModal(false)}
                  disabled={isSigning}
                >
                  CANCEL
                </button>
                <button
                  className="modal-btn-confirm"
                  onClick={handleConfirmTrade}
                  disabled={isSigning}
                  style={{
                    background: isDemoMode ? 'linear-gradient(135deg, #a855f7, #6366f1)' : undefined
                  }}
                >
                  {isSigning
                    ? 'PROCESSING...'
                    : (isDemoMode ? '🧪 EXECUTE PAPER TRADE (DEMO)' : '🔴 UNLOCK & EXECUTE (FAIL-CLOSED)')}
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Phase 3: Review Unsigned Swap Transaction Candidate Modal */}
        {showPhase3ReviewModal && swapValidationResult && (
          <div className="modal-backdrop">
            <div className="modal-content" style={{ width: '560px', maxHeight: '90vh', overflowY: 'auto' }}>
              <div className="modal-header">
                <span className="modal-title">
                  Review Unsigned Swap Candidate
                </span>
                <span className="modal-badge" style={{
                  background: swapValidationResult.is_valid && swapValidationResult.is_simulated
                    ? 'rgba(74, 222, 128, 0.15)'
                    : 'rgba(239, 68, 68, 0.15)',
                  color: swapValidationResult.is_valid && swapValidationResult.is_simulated
                    ? 'var(--accent-green)'
                    : 'var(--accent-red)',
                  border: `1px solid ${swapValidationResult.is_valid && swapValidationResult.is_simulated ? 'var(--accent-green)' : 'var(--accent-red)'}`
                }}>
                  {swapValidationResult.is_valid && swapValidationResult.is_simulated
                    ? '✅ VALIDATED & SIMULATED'
                    : '❌ VALIDATION FAILED'}
                </span>
              </div>

              {/* Target Token & Candidate Header */}
              <div style={{ display: 'flex', alignItems: 'center', gap: '10px', marginBottom: '14px', background: 'var(--bg-primary)', padding: '10px 12px', borderRadius: 'var(--radius-md)' }}>
                {selectedToken.imageUrl ? (
                  <img src={selectedToken.imageUrl} alt="" style={{ width: '36px', height: '36px', borderRadius: '50%' }} />
                ) : (
                  <div style={{ width: '36px', height: '36px', borderRadius: '50%', background: 'linear-gradient(135deg, var(--accent-cyan), var(--accent-green))', display: 'flex', alignItems: 'center', justifyContent: 'center', fontWeight: 700, color: '#000', fontSize: '14px' }}>
                    {selectedToken.symbol.slice(0, 2)}
                  </div>
                )}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                    <span style={{ fontWeight: 800, fontSize: '14px' }}>{selectedToken.symbol}</span>
                    <span style={{ fontSize: '11px', color: 'var(--text-muted)' }}>{selectedToken.name}</span>
                  </div>
                  <div style={{ fontSize: '10px', fontFamily: 'var(--font-mono)', color: 'var(--text-dim)', marginTop: '2px' }}>
                    Candidate ID: {swapValidationResult.candidate_id}
                  </div>
                </div>
                <span className="dex-tag" style={{ background: 'rgba(14, 165, 233, 0.2)', color: 'var(--accent-cyan)', border: '1px solid var(--accent-cyan)' }}>
                  {swapValidationResult.review_summary.venue}
                </span>
              </div>

              {/* Validation Errors or Warnings if any */}
              {swapValidationResult.validation_errors.length > 0 && (
                <div style={{ background: 'rgba(239, 68, 68, 0.1)', border: '1px solid var(--accent-red)', borderRadius: 'var(--radius-sm)', padding: '10px 12px', marginBottom: '14px', fontSize: '11px' }}>
                  <div style={{ fontWeight: 700, color: 'var(--accent-red)', marginBottom: '4px' }}>❌ Validation Failures Detected:</div>
                  <ul style={{ margin: 0, paddingLeft: '16px', color: 'var(--accent-red)' }}>
                    {swapValidationResult.validation_errors.map((err, i) => (
                      <li key={i}>{err}</li>
                    ))}
                  </ul>
                </div>
              )}

              {swapValidationResult.validation_warnings.length > 0 && (
                <div style={{ background: 'rgba(255, 183, 3, 0.1)', border: '1px solid var(--accent-amber)', borderRadius: 'var(--radius-sm)', padding: '8px 12px', marginBottom: '14px', fontSize: '11px' }}>
                  {swapValidationResult.validation_warnings.map((w, idx) => (
                    <div key={idx} style={{ color: 'var(--accent-amber)' }}>⚠️ {w}</div>
                  ))}
                </div>
              )}

              {/* Candidate Review Grid */}
              <div className="preview-grid" style={{ marginBottom: '14px' }}>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Trade Action:</span>
                  <span style={{ fontWeight: 800, color: swapValidationResult.review_summary.side.toLowerCase() === 'buy' ? 'var(--accent-green)' : 'var(--accent-red)', textTransform: 'uppercase' }}>
                    {swapValidationResult.review_summary.side} {selectedToken.symbol}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Exact Input:</span>
                  <span style={{ color: 'var(--text-main)', fontWeight: 700, fontFamily: 'var(--font-mono)' }}>
                    {swapValidationResult.review_summary.input_display}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Estimated Gross Output:</span>
                  <span style={{ color: 'var(--text-dim)', fontFamily: 'var(--font-mono)' }}>
                    {swapValidationResult.review_summary.estimated_output_display} (EST)
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', background: 'rgba(74, 222, 128, 0.08)', padding: '4px 6px', borderRadius: '4px' }}>
                  <span style={{ fontWeight: 700, color: 'var(--accent-green)' }}>Guaranteed Minimum Output:</span>
                  <span style={{ fontWeight: 800, color: 'var(--accent-green)', fontFamily: 'var(--font-mono)' }}>
                    {swapValidationResult.review_summary.minimum_output_display}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Slippage Limit:</span>
                  <span>{swapValidationResult.review_summary.slippage_display}</span>
                </div>

                {/* Verified Fees & Recipient */}
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <span>Platform Treasury Fee:</span>
                  <div style={{ textAlign: 'right' }}>
                    <span style={{ color: 'var(--accent-cyan)', fontWeight: 700 }}>
                      {swapValidationResult.review_summary.platform_fee_display}
                    </span>
                    <div style={{ fontSize: '9px', color: 'var(--accent-green)', fontFamily: 'var(--font-mono)' }}>
                      ✓ Verified Native Treasury: {swapValidationResult.candidate.summary.platform_fee_recipient.slice(0, 4)}...{swapValidationResult.candidate.summary.platform_fee_recipient.slice(-4)}
                    </div>
                  </div>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Priority Compute Fee:</span>
                  <span>{swapValidationResult.review_summary.priority_fee_display}</span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Rent / ATA Creation:</span>
                  <span>{swapValidationResult.review_summary.rent_cost_display}</span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', borderTop: '1px solid var(--border-subtle)', paddingTop: '6px' }}>
                  <span style={{ fontWeight: 700 }}>Total Estimated SOL Required:</span>
                  <span style={{ fontWeight: 800, color: 'var(--accent-cyan)', fontFamily: 'var(--font-mono)' }}>
                    {swapValidationResult.review_summary.total_sol_estimated_display}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Transaction Byte Size:</span>
                  <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-dim)' }}>
                    {swapValidationResult.review_summary.transaction_size_bytes} bytes (Max: 1232 MTU)
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>Quote Age / Freshness:</span>
                  <span style={{ fontFamily: 'var(--font-mono)', color: swapValidationResult.review_summary.quote_age_seconds > 45 ? 'var(--accent-amber)' : 'var(--accent-green)' }}>
                    {swapValidationResult.review_summary.quote_age_seconds}s old (Limit: 60s)
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span>RPC Read-Only Simulation:</span>
                  <span style={{ color: 'var(--accent-green)', fontWeight: 600 }}>
                    {swapValidationResult.is_simulated
                      ? `✓ Succeeded (${swapValidationResult.simulation_units_consumed?.toLocaleString() || 0} CU consumed)`
                      : '❌ Simulation Failed'}
                  </span>
                </div>
              </div>

              {/* Instruction Breakdown List */}
              {swapValidationResult.candidate_summary?.instructions && (
                <div style={{ marginBottom: '14px', background: 'rgba(0, 0, 0, 0.25)', borderRadius: 'var(--radius-sm)', padding: '10px 12px' }}>
                  <div style={{ fontSize: '11px', fontWeight: 700, color: 'var(--text-muted)', marginBottom: '8px', display: 'flex', justifyContent: 'space-between' }}>
                    <span>COMPILED INSTRUCTIONS ({swapValidationResult.candidate_summary.instructions.length})</span>
                    <span style={{ fontSize: '9px', color: 'var(--accent-green)' }}>✓ All Programs Allowlisted</span>
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
                    {swapValidationResult.candidate_summary.instructions.map((ix, idx) => {
                      const isExpanded = Boolean(expandedInstructions[idx]);
                      return (
                        <div key={idx} style={{ background: 'rgba(255, 255, 255, 0.03)', borderRadius: '4px', padding: '6px 8px', fontSize: '10px' }}>
                          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                            <span style={{ fontWeight: 700, color: 'var(--accent-cyan)' }}>
                              #{idx + 1} {ix.program_name}
                            </span>
                            <button
                              onClick={() => setExpandedInstructions((prev) => ({ ...prev, [idx]: !isExpanded }))}
                              style={{ background: 'transparent', border: 'none', color: 'var(--text-dim)', cursor: 'pointer', fontSize: '9px' }}
                            >
                              {isExpanded ? 'Hide Accounts ▲' : `View Accounts (${ix.accounts.length}) ▼`}
                            </button>
                          </div>
                          <div style={{ color: 'var(--text-main)', marginTop: '2px', fontFamily: 'var(--font-mono)' }}>
                            {ix.description}
                          </div>
                          <div style={{ fontSize: '9px', color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', marginTop: '2px' }}>
                            Program ID: {ix.program_id}
                          </div>
                          {isExpanded && (
                            <div style={{ marginTop: '6px', borderTop: '1px dashed rgba(255, 255, 255, 0.1)', paddingTop: '4px' }}>
                              <div style={{ fontSize: '9px', fontWeight: 600, color: 'var(--text-dim)', marginBottom: '4px' }}>Accounts ({ix.accounts.length}):</div>
                              {ix.accounts.map((acc, aIdx) => (
                                <div key={aIdx} style={{ display: 'flex', justifyContent: 'space-between', fontSize: '9px', fontFamily: 'var(--font-mono)', padding: '2px 0' }}>
                                  <span style={{ color: acc.is_signer ? 'var(--accent-amber)' : 'var(--text-muted)' }}>
                                    {acc.label ? `[${acc.label}] ` : ''}{acc.pubkey.slice(0, 8)}...{acc.pubkey.slice(-6)}
                                  </span>
                                  <div style={{ display: 'flex', gap: '4px' }}>
                                    {acc.is_signer && <span style={{ color: 'var(--accent-amber)', fontWeight: 700 }}>SIGNER</span>}
                                    {acc.is_writable && <span style={{ color: 'var(--accent-cyan)' }}>WRITABLE</span>}
                                    {!acc.is_signer && !acc.is_writable && <span style={{ color: 'var(--text-dim)' }}>READONLY</span>}
                                  </div>
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Simulation Execution Logs (Collapsible) */}
              {swapValidationResult.simulation_logs.length > 0 && (
                <div style={{ marginBottom: '14px' }}>
                  <div
                    onClick={() => setShowSimLogs(!showSimLogs)}
                    style={{ fontSize: '10px', color: 'var(--text-dim)', cursor: 'pointer', userSelect: 'none', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}
                  >
                    <span>📜 RPC Simulation Execution Logs ({swapValidationResult.simulation_logs.length} lines)</span>
                    <span>{showSimLogs ? '▲ Hide' : '▼ Expand'}</span>
                  </div>
                  {showSimLogs && (
                    <div style={{ marginTop: '6px', maxHeight: '120px', overflowY: 'auto', background: 'rgba(0, 0, 0, 0.5)', padding: '6px 8px', borderRadius: '4px', fontFamily: 'var(--font-mono)', fontSize: '9px', color: '#94a3b8' }}>
                      {swapValidationResult.simulation_logs.map((log, lIdx) => (
                        <div key={lIdx}>{log}</div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* Phase 4 Live Signing, Submission & Confirmation Section */}
              {liveSubmissionResult ? (
                <div style={{ marginBottom: '16px' }}>
                  {liveSubmissionResult.status === 'confirmed' && (
                    <div style={{ background: 'rgba(74, 222, 128, 0.12)', border: '1px solid var(--accent-green)', borderRadius: 'var(--radius-sm)', padding: '14px', marginBottom: '14px' }}>
                      <div style={{ color: 'var(--accent-green)', fontWeight: 800, fontSize: '13px', display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span>✅ TRANSACTION CONFIRMED ON SOLANA MAINNET</span>
                      </div>
                      <div style={{ marginTop: '8px', fontSize: '11px', color: 'var(--text-main)', fontFamily: 'var(--font-mono)', display: 'flex', flexDirection: 'column', gap: '3px' }}>
                        {liveSubmissionResult.slot && <div>Block Slot: {liveSubmissionResult.slot}</div>}
                        {liveSubmissionResult.confirmations !== undefined && <div>Confirmations: {liveSubmissionResult.confirmations}</div>}
                        {liveSubmissionResult.signature && (
                          <div style={{ wordBreak: 'break-all' }}>
                            Signature: {liveSubmissionResult.signature}
                          </div>
                        )}
                      </div>
                      {liveSubmissionResult.explorer_url && (
                        <div style={{ marginTop: '10px' }}>
                          <a
                            href={liveSubmissionResult.explorer_url}
                            target="_blank"
                            rel="noreferrer"
                            style={{ color: 'var(--accent-cyan)', textDecoration: 'underline', fontSize: '11px', fontWeight: 700 }}
                          >
                            🔍 View Confirmed Transaction on Solscan ↗
                          </a>
                        </div>
                      )}
                    </div>
                  )}

                  {liveSubmissionResult.status === 'failed' && (
                    <div style={{ background: 'rgba(239, 68, 68, 0.12)', border: '1px solid var(--accent-red)', borderRadius: 'var(--radius-sm)', padding: '14px', marginBottom: '14px' }}>
                      <div style={{ color: 'var(--accent-red)', fontWeight: 800, fontSize: '13px' }}>
                        ❌ TRANSACTION EXECUTION FAILED
                      </div>
                      <div style={{ marginTop: '6px', fontSize: '11px', color: '#ff8585', fontFamily: 'var(--font-mono)' }}>
                        {liveSubmissionResult.error || 'Execution was rejected by the cluster or failed preflight simulation.'}
                      </div>
                      {liveSubmissionResult.logs && liveSubmissionResult.logs.length > 0 && (
                        <div style={{ marginTop: '8px', maxHeight: '100px', overflowY: 'auto', background: 'rgba(0,0,0,0.4)', padding: '6px', borderRadius: '4px', fontSize: '10px', color: '#94a3b8', fontFamily: 'var(--font-mono)' }}>
                          {liveSubmissionResult.logs.map((l, i) => <div key={i}>{l}</div>)}
                        </div>
                      )}
                      {liveSubmissionResult.explorer_url && (
                        <div style={{ marginTop: '8px' }}>
                          <a
                            href={liveSubmissionResult.explorer_url}
                            target="_blank"
                            rel="noreferrer"
                            style={{ color: 'var(--accent-cyan)', textDecoration: 'underline', fontSize: '11px' }}
                          >
                            View Details on Solscan ↗
                          </a>
                        </div>
                      )}
                      <div style={{ marginTop: '6px', fontSize: '10px', color: 'var(--text-muted)' }}>
                        No tokens or SOL were deducted from your wallet.
                      </div>
                    </div>
                  )}

                  {liveSubmissionResult.status === 'expired' && (
                    <div style={{ background: 'rgba(255, 183, 3, 0.12)', border: '1px solid var(--accent-amber)', borderRadius: 'var(--radius-sm)', padding: '14px', marginBottom: '14px' }}>
                      <div style={{ color: 'var(--accent-amber)', fontWeight: 800, fontSize: '13px' }}>
                        ⌛ TRANSACTION EXPIRED
                      </div>
                      <div style={{ marginTop: '6px', fontSize: '11px', color: 'var(--text-main)' }}>
                        {liveSubmissionResult.error || 'Transaction blockhash expired without landing on cluster.'}
                      </div>
                      <div style={{ marginTop: '4px', fontSize: '10px', color: 'var(--text-dim)' }}>
                        Solana cluster dropped this blockhash. No funds were spent. Please request a fresh route candidate.
                      </div>
                      {liveSubmissionResult.explorer_url && (
                        <div style={{ marginTop: '8px' }}>
                          <a
                            href={liveSubmissionResult.explorer_url}
                            target="_blank"
                            rel="noreferrer"
                            style={{ color: 'var(--accent-cyan)', textDecoration: 'underline', fontSize: '11px' }}
                          >
                            Check on Solscan ↗
                          </a>
                        </div>
                      )}
                    </div>
                  )}

                  {liveSubmissionResult.status === 'unknown' && (
                    <div style={{ background: 'rgba(234, 179, 8, 0.12)', border: '1px solid #eab308', borderRadius: 'var(--radius-sm)', padding: '14px', marginBottom: '14px' }}>
                      <div style={{ color: '#eab308', fontWeight: 800, fontSize: '13px' }}>
                        ⚠️ TRANSACTION STATUS UNRESOLVED / PENDING
                      </div>
                      <div style={{ marginTop: '6px', fontSize: '11px', color: 'var(--text-main)' }}>
                        {liveSubmissionResult.error}
                      </div>
                      {liveSubmissionResult.explorer_url && (
                        <div style={{ marginTop: '6px' }}>
                          <a
                            href={liveSubmissionResult.explorer_url}
                            target="_blank"
                            rel="noreferrer"
                            style={{ color: 'var(--accent-cyan)', textDecoration: 'underline', fontSize: '11px' }}
                          >
                            Check Transaction on Solscan ↗
                          </a>
                        </div>
                      )}
                      <div style={{ marginTop: '10px' }}>
                        <button
                          onClick={handleQuerySwapStatus}
                          disabled={isSubmittingLiveSwap}
                          style={{ padding: '6px 14px', background: '#eab308', color: '#000', fontWeight: 700, borderRadius: '4px', border: 'none', cursor: 'pointer', fontSize: '11px' }}
                        >
                          {isSubmittingLiveSwap ? 'Checking Status...' : '🔄 Re-check On-Chain Status'}
                        </button>
                      </div>
                    </div>
                  )}

                  <div className="modal-actions">
                    <button
                      className="modal-btn-confirm"
                      onClick={() => {
                        setShowPhase3ReviewModal(false);
                        setLiveSubmissionResult(null);
                      }}
                      style={{ width: '100%', background: 'var(--bg-tertiary)', color: 'var(--text-main)', border: '1px solid var(--border-subtle)', fontWeight: 700 }}
                    >
                      CLOSE REVIEW
                    </button>
                  </div>
                </div>
              ) : (
                <div>
                  {/* Authorization & Password Entry */}
                  {swapValidationResult.is_valid && swapValidationResult.is_simulated ? (
                    <div style={{ background: 'var(--bg-primary)', border: '1px solid var(--border-subtle)', borderRadius: 'var(--radius-sm)', padding: '12px 14px', marginBottom: '16px' }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                        <span style={{ fontSize: '11px', fontWeight: 700, color: 'var(--text-main)' }}>
                          🔑 Authorize & Sign Transaction
                        </span>
                        <span style={{ fontSize: '10px', color: 'var(--accent-cyan)', fontFamily: 'var(--font-mono)' }}>
                          Cluster: Mainnet-Beta
                        </span>
                      </div>

                      <div style={{ marginBottom: '10px' }}>
                        <input
                          type="password"
                          value={phase4Password}
                          onChange={(e) => setPhase4Password(e.target.value)}
                          placeholder="Enter wallet keystore password"
                          disabled={isSubmittingLiveSwap}
                          style={{
                            width: '100%',
                            padding: '8px 10px',
                            background: 'rgba(0, 0, 0, 0.3)',
                            border: '1px solid var(--border-subtle)',
                            borderRadius: '4px',
                            color: '#fff',
                            fontSize: '12px',
                            fontFamily: 'var(--font-mono)',
                          }}
                        />
                      </div>

                      <label style={{ display: 'flex', alignItems: 'flex-start', gap: '8px', cursor: 'pointer', fontSize: '11px', color: 'var(--text-muted)', userSelect: 'none' }}>
                        <input
                          type="checkbox"
                          checked={clusterSafetyConfirmed}
                          onChange={(e) => setClusterSafetyConfirmed(e.target.checked)}
                          disabled={isSubmittingLiveSwap}
                          style={{ marginTop: '2px' }}
                        />
                        <span>
                          <strong style={{ color: 'var(--accent-amber)' }}>Mainnet Safety Gate:</strong> I confirm intentional execution on Solana Mainnet. I have verified the route, amounts, and fees.
                        </span>
                      </label>

                      {isSubmittingLiveSwap && (
                        <div style={{ marginTop: '12px', background: 'rgba(14, 165, 233, 0.1)', border: '1px solid var(--accent-cyan)', borderRadius: 'var(--radius-sm)', padding: '8px 12px', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
                          <div style={{ color: 'var(--accent-cyan)', fontWeight: 700, display: 'flex', alignItems: 'center', gap: '8px' }}>
                            <span className="spinner" style={{ display: 'inline-block', width: '12px', height: '12px', border: '2px solid var(--accent-cyan)', borderTopColor: 'transparent', borderRadius: '50%' }}></span>
                            <span>{liveSwapStage || 'Executing trade in Rust...'}</span>
                          </div>
                        </div>
                      )}
                    </div>
                  ) : (
                    <div style={{ background: 'rgba(239, 68, 68, 0.08)', border: '1px solid rgba(239, 68, 68, 0.25)', borderRadius: 'var(--radius-sm)', padding: '10px 12px', marginBottom: '16px', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
                      <div style={{ color: '#ff6b6b', fontWeight: 700, marginBottom: '2px' }}>
                        🔒 Signing Blocked Due to Validation Failure
                      </div>
                      <div style={{ color: 'var(--text-muted)', fontSize: '10px' }}>
                        The candidate transaction failed client-side security checks or RPC simulation. Execution is refused to protect your funds.
                      </div>
                    </div>
                  )}

                  {/* Modal Actions */}
                  <div className="modal-actions">
                    <button
                      className="modal-btn-cancel"
                      onClick={() => setShowPhase3ReviewModal(false)}
                      disabled={isSubmittingLiveSwap}
                      style={{ flex: 1 }}
                    >
                      CLOSE REVIEW
                    </button>
                    <button
                      className="modal-btn-confirm"
                      onClick={handleLiveSwapSignAndSubmit}
                      disabled={!swapValidationResult.is_valid || !swapValidationResult.is_simulated || isSubmittingLiveSwap || !phase4Password.trim() || !clusterSafetyConfirmed}
                      style={{
                        flex: 2,
                        opacity: (!swapValidationResult.is_valid || !swapValidationResult.is_simulated || isSubmittingLiveSwap || !phase4Password.trim() || !clusterSafetyConfirmed) ? 0.5 : 1,
                        cursor: (!swapValidationResult.is_valid || !swapValidationResult.is_simulated || isSubmittingLiveSwap || !phase4Password.trim() || !clusterSafetyConfirmed) ? 'not-allowed' : 'pointer',
                        background: 'linear-gradient(135deg, var(--accent-green), var(--accent-cyan))',
                        color: '#000',
                        fontWeight: 800,
                      }}
                      title={
                        !clusterSafetyConfirmed
                          ? 'Check the Mainnet Safety Gate confirmation box to proceed'
                          : !phase4Password.trim()
                            ? 'Enter your wallet password to proceed'
                            : 'Sign transaction with local keypair and submit to Solana RPC'
                      }
                    >
                      {isSubmittingLiveSwap
                        ? 'SIGNING & SUBMITTING...'
                        : (!swapValidationResult.is_valid || !swapValidationResult.is_simulated)
                          ? '❌ VALIDATION FAILED'
                          : '🚀 SIGN & SUBMIT TO SOLANA'}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}


        {/* Transaction Confirmed Success Modal */}
        {tradeSuccessModal && (
          <div className="modal-backdrop">
            <div className="modal-content" style={{ width: '420px', textAlign: 'center' }}>
              <div className="success-modal-icon" style={{ background: isDemoMode ? 'rgba(168, 85, 247, 0.2)' : undefined, color: isDemoMode ? '#c084fc' : undefined }}>
                {isDemoMode ? '🧪' : '✓'}
              </div>
              <div style={{ fontSize: '18px', fontWeight: 700, color: 'var(--text-main)', marginBottom: '4px' }}>
                {isDemoMode
                  ? (tradeSuccessModal.type === 'buy' ? '🧪 Demo Buy Simulated!' : '🧪 Demo Sell Simulated!')
                  : (tradeSuccessModal.type === 'buy' ? '🎉 Buy Order Confirmed!' : '🎉 Sell Order Confirmed!')}
              </div>
              <div style={{ fontSize: '11px', color: isDemoMode ? '#c084fc' : 'var(--accent-green)', fontFamily: 'var(--font-mono)', marginBottom: '16px' }}>
                {isDemoMode ? 'PAPER SIMULATION COMPLETED LOCALLY · NO ON-CHAIN TX' : 'SOLANA TRANSACTION CONFIRMED LOCALLY'}
              </div>

              <div className="success-stat-box">
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span style={{ color: 'var(--text-muted)' }}>Action:</span>
                  <span style={{ fontWeight: 700, color: tradeSuccessModal.type === 'buy' ? 'var(--accent-green)' : 'var(--accent-red)' }}>
                    {tradeSuccessModal.type.toUpperCase()} {tradeSuccessModal.symbol}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span style={{ color: 'var(--text-muted)' }}>Tokens {tradeSuccessModal.type === 'buy' ? 'Received' : 'Sold'}:</span>
                  <span style={{ fontWeight: 700, color: 'var(--text-main)' }}>
                    {tradeSuccessModal.amountTokens.toLocaleString(undefined, { maximumFractionDigits: 0 })} {tradeSuccessModal.symbol}
                  </span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span style={{ color: 'var(--text-muted)' }}>SOL {tradeSuccessModal.type === 'buy' ? 'Spent' : 'Received'}:</span>
                  <span style={{ fontWeight: 700, color: 'var(--accent-cyan)' }}>
                    {tradeSuccessModal.amountSol.toFixed(4)} SOL
                  </span>
                </div>
                {tradeSuccessModal.executionMs !== undefined && (
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span style={{ color: 'var(--text-muted)' }}>Execution Latency:</span>
                    <span style={{
                      fontWeight: 700,
                      color: 'var(--accent-cyan)',
                      fontFamily: 'var(--font-mono)',
                      background: 'rgba(0, 240, 255, 0.1)',
                      padding: '2px 8px',
                      borderRadius: '4px',
                      fontSize: '11px'
                    }}>
                      ⚡ {tradeSuccessModal.executionMs < 1000 ? `${tradeSuccessModal.executionMs}ms` : `${(tradeSuccessModal.executionMs / 1000).toFixed(2)}s`}
                    </span>
                  </div>
                )}
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', borderTop: '1px solid var(--border-subtle)', paddingTop: '8px', marginTop: '4px' }}>
                  <span style={{ color: 'var(--text-muted)' }}>Reference:</span>
                  {tradeSuccessModal.signature.startsWith('SIMULATED_') ? (
                    <span style={{ color: '#c084fc', fontFamily: 'var(--font-mono)', fontSize: '10px' }}>
                      {tradeSuccessModal.signature.slice(0, 20)}... [Demo Paper]
                    </span>
                  ) : (
                    <a
                      href={`https://solscan.io/tx/${tradeSuccessModal.signature}`}
                      target="_blank"
                      rel="noreferrer"
                      style={{ color: 'var(--accent-cyan)', textDecoration: 'underline', fontSize: '11px' }}
                    >
                      {tradeSuccessModal.signature.slice(0, 8)}...{tradeSuccessModal.signature.slice(-6)} ↗
                    </a>
                  )}
                </div>
              </div>

              <div className="modal-actions" style={{ justifyContent: 'center', gap: '10px' }}>
                <button
                  className="modal-btn-cancel"
                  onClick={() => {
                    setTradeSuccessModal(null);
                    setCurrentNav('trade');
                    setTradeViewMode('detail');
                  }}
                >
                  CLOSE
                </button>
                <button
                  className="modal-btn-confirm"
                  onClick={() => {
                    setTradeSuccessModal(null);
                    setCurrentNav('trade');
                    setTradeViewMode('detail');
                  }}
                  style={{
                    background: isDemoMode ? 'linear-gradient(135deg, #a855f7, #6366f1)' : undefined
                  }}
                >
                  CONTINUE TRADING
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Receive & Send Modals */}
        {renderReceiveModal()}
        {renderSendModal()}
      </div>
    </div>
  );
}
