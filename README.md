# Solana Trading Bot & Platform Reference Architecture

This project workspace serves as the foundation and reference base for building a high-performance Solana trading bot and platform.

## Workspace Structure

- `references/`: Complete source code of curated reference repositories.
  - `pumpfun-bonkfun-bot/` (Chainstack Labs - Python): Sniping & trading bot for Pump.fun and Bonk.fun with transaction building and monitoring.
  - `pumpfun-cli/` (Chainstack Labs - Python): CLI tool for Pump.fun operations (bonding curve calculations, buy/sell executions, account decoding).
  - `solana-swap-tutorial/` (builderby - Node.js/JavaScript): Jito MEV bundling, priority fee calculation, and Jupiter API integration for sandwich-resistant swaps.
  - `jupiter-v6-swap-api/` (gungho0619 - TypeScript/Node.js): Jupiter V6 Swap API integration, quote fetching, swap transaction building, and execution.
  - `sol-trade-sdk-golang/` (Agit8037 - Golang): Ultra low-latency Solana DEX trading SDK supporting Raydium, Pump.fun, Orca, Jito bundles, and fast RPC execution in Go.
  - `Solana-Trading-Bot/` (henrytirla - Python): Comprehensive bot covering Pump.fun token tracking, Jito MEV protection bundles, WSOL wrapping, and priority fees.
  - `lightweight-charts/` (TradingView - TypeScript): High-performance financial candlestick charting library for real-time price charts and technical indicators.

---

## Key Architectural Reference Themes

### 1. Jito Bundling & MEV Protection
- **Purpose**: Prevent front-running and sandwich attacks by routing transactions directly to Jito block engines instead of the public mempool.
- **Reference Files**:
  - `references/solana-swap-tutorial/jitoService.js`
  - `references/Solana-Trading-Bot/Jito/`
  - `references/sol-trade-sdk-golang/pkg/jito/`

### 2. Pump.fun Bonding Curve & Fast Swap Logic
- **Purpose**: Decode bonding curve state, calculate exact token out / SOL in prices with slippage, and execute instant buys/sells.
- **Reference Files**:
  - `references/pumpfun-cli/src/` & `idl/`
  - `references/pumpfun-bonkfun-bot/bots/` & `idl/`
  - `references/Solana-Trading-Bot/PumpDotFun/`

### 3. Jupiter V6 DEX Routing & Execution
- **Purpose**: Best-execution aggregated swaps across Raydium, Orca, Meteora, Whirlpools, etc.
- **Reference Files**:
  - `references/jupiter-v6-swap-api/`
  - `references/solana-swap-tutorial/jupiterApi.js`

### 4. High-Throughput Trade Execution (Go Engine)
- **Purpose**: Concurrency, low-latency gRPC/RPC calls, and microsecond-level order generation.
- **Reference Files**:
  - `references/sol-trade-sdk-golang/pkg/`
  - `references/sol-trade-sdk-golang/examples/`

### 5. Real-Time TradingView Charts
- **Purpose**: Live candlestick and volume display for tokens being traded.
- **Reference Files**:
  - `references/lightweight-charts/`
