//! Pump.fun program addresses, fee recipients, and PDA derivation.
//!
//! Rust port of `pumpfun-bonkfun-bot/src/platforms/pumpfun/address_provider.py`.
//! All constants verified against the reference code and pump-fun/pump-public-docs.

use rand::seq::SliceRandom;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;

// ────────────────────────────────────────────────────────────────────────────
// System program IDs
// ────────────────────────────────────────────────────────────────────────────

/// Native SOL mint — `Pubkey::default()` on-chain, but instructions need
/// Wrapped SOL (`So11111111111111111111111111111111111111112`).
pub const WRAPPED_SOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");

pub const SYSTEM_PROGRAM: Pubkey = solana_sdk::system_program::ID;
pub const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
    pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// ────────────────────────────────────────────────────────────────────────────
// Pump.fun program addresses
// ────────────────────────────────────────────────────────────────────────────

/// Main Pump.fun program.
pub const PUMP_PROGRAM: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

/// Global account PDA.
pub const PUMP_GLOBAL: Pubkey = pubkey!("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf");

/// Mint authority PDA — `["mint-authority"]`. Present on create instructions,
/// absent from trade instructions, useful for filtering subscriptions.
pub const PUMP_MINT_AUTHORITY: Pubkey = pubkey!("TSLvdd1pWpHVjahSpsvCXUbgwsL3JAcvokwaKt1eokM");

/// Event authority PDA.
pub const PUMP_EVENT_AUTHORITY: Pubkey = pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");

/// Legacy fee recipient.
pub const PUMP_FEE: Pubkey = pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");

/// Mayhem mode fee recipient.
pub const PUMP_MAYHEM_FEE: Pubkey = pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS");

/// Liquidity migrator.
pub const PUMP_LIQUIDITY_MIGRATOR: Pubkey = pubkey!("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg");

/// Fee program (pfeeUxB...).
pub const PUMP_FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");

// ────────────────────────────────────────────────────────────────────────────
// Fee recipient sets (8 each, as specified in FEE_RECIPIENTS.md)
// ────────────────────────────────────────────────────────────────────────────

/// 8 normal fee recipients — used as `fee_recipient` for non-mayhem coins.
pub const NORMAL_FEE_RECIPIENTS: [Pubkey; 8] = [
    pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV"),
    pubkey!("7VtfL8fvgNfhz17qKRMjzQEXgbdpnHHHQRh54R9jP2RJ"),
    pubkey!("7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX"),
    pubkey!("9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz"),
    pubkey!("AVmoTthdrX6tKt4nDjco2D775W2YK3sDhxPcMmzUAmTY"),
    pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM"),
    pubkey!("FWsW1xNtWscwNmKv6wVsU1iTzRN6wmmk3MjxRP5tT7hz"),
    pubkey!("G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP"),
];

/// 8 reserved fee recipients — used as `fee_recipient` for mayhem coins.
pub const RESERVED_FEE_RECIPIENTS: [Pubkey; 8] = [
    pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS"),
    pubkey!("4budycTjhs9fD6xw62VBducVTNgMgJJ5BgtKq7mAZwn6"),
    pubkey!("8SBKzEQU4nLSzcwF4a74F2iaUDQyTfjGndn6qUWBnrpR"),
    pubkey!("4UQeTP1T39KZ9Sfxzo3WR5skgsaP6NZa87BAkuazLEKH"),
    pubkey!("8sNeir4QsLsJdYpc9RZacohhK1Y5FLU3nC5LXgYB4aa6"),
    pubkey!("Fh9HmeLNUMVCvejxCtCL2DbYaRyBFVJ5xrWkLnMH6fdk"),
    pubkey!("463MEnMeGyJekNZFQSTUABBEbLnvMTALbT6ZmsxAbAdq"),
    pubkey!("6AUH3WEHucYZyC61hqpqYUWVto5qA5hjHuNQ32GNnNxA"),
];

/// 8 buyback fee recipients — required on every buy/sell instruction.
pub const BUYBACK_FEE_RECIPIENTS: [Pubkey; 8] = [
    pubkey!("5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD"),
    pubkey!("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7"),
    pubkey!("GXPFM2caqTtQYC2cJ5yJRi9VDkpsYZXzYdwYpGnLmtDL"),
    pubkey!("3BpXnfJaUTiwXnJNe7Ej1rcbzqTTQUvLShZaWazebsVR"),
    pubkey!("5cjcW9wExnJJiqgLjq7DEG75Pm6JBgE1hNv4B2vHXUW6"),
    pubkey!("EHAAiTxcdDwQ3U4bU6YcMsQGaekdzLS3B5SmYo46kJtL"),
    pubkey!("5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD"),
    pubkey!("A7hAgCzFw14fejgCp387JUJRMNyz4j89JKnhtKU8piqW"),
];

// ────────────────────────────────────────────────────────────────────────────
// Fee recipient selection
// ────────────────────────────────────────────────────────────────────────────

/// Pick one of the 8 buyback fee recipients at random to spread load.
pub fn pick_buyback_fee_recipient() -> Pubkey {
    let mut rng = rand::thread_rng();
    *BUYBACK_FEE_RECIPIENTS
        .choose(&mut rng)
        .expect("array is non-empty")
}

/// Pick the correct fee recipient based on mayhem mode.
pub fn pick_fee_recipient(is_mayhem_mode: bool) -> Pubkey {
    if is_mayhem_mode {
        let mut rng = rand::thread_rng();
        *RESERVED_FEE_RECIPIENTS
            .choose(&mut rng)
            .expect("array is non-empty")
    } else {
        let mut rng = rand::thread_rng();
        *NORMAL_FEE_RECIPIENTS
            .choose(&mut rng)
            .expect("array is non-empty")
    }
}

// ────────────────────────────────────────────────────────────────────────────
// PDA derivation helpers
// ────────────────────────────────────────────────────────────────────────────

/// Normalise the quote mint: `Pubkey::default()` (SOL on-chain) → Wrapped SOL.
pub fn normalize_quote_mint(quote_mint: &Pubkey) -> Pubkey {
    if *quote_mint == Pubkey::default() {
        WRAPPED_SOL_MINT
    } else {
        *quote_mint
    }
}

/// Check whether a quote mint represents native SOL.
pub fn is_sol_paired(quote_mint: &Pubkey) -> bool {
    *quote_mint == Pubkey::default() || *quote_mint == WRAPPED_SOL_MINT
}

/// Derive bonding curve PDA: `["bonding-curve", mint]`.
pub fn find_bonding_curve(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], &PUMP_PROGRAM)
}

/// Derive bonding curve V2 PDA: `["bonding-curve-v2", mint]`.
pub fn find_bonding_curve_v2(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"bonding-curve-v2", mint.as_ref()], &PUMP_PROGRAM)
}

/// Derive creator vault PDA: `["creator-vault", creator]`.
pub fn find_creator_vault(creator: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"creator-vault", creator.as_ref()], &PUMP_PROGRAM)
}

/// Derive sharing config PDA: `["sharing-config", base_mint]` under the fee program.
pub fn find_sharing_config(base_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"sharing-config", base_mint.as_ref()], &PUMP_FEE_PROGRAM)
}

/// Derive holder-reward creator PDA: `["holder-rewards", mint]`.
pub fn find_holder_reward_creator(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"holder-rewards", mint.as_ref()], &PUMP_PROGRAM)
}

/// Derive global volume accumulator PDA: `["global_volume_accumulator"]`.
pub fn find_global_volume_accumulator() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"global_volume_accumulator"], &PUMP_PROGRAM)
}

/// Derive user volume accumulator PDA: `["user_volume_accumulator", user]`.
pub fn find_user_volume_accumulator(user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"user_volume_accumulator", user.as_ref()], &PUMP_PROGRAM)
}

/// Derive fee config PDA: `["fee_config", PUMP_PROGRAM]` under the fee program.
pub fn find_fee_config() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"fee_config", PUMP_PROGRAM.as_ref()], &PUMP_FEE_PROGRAM)
}

/// Derive the Associated Token Account for `(owner, mint)` under `token_program`.
pub fn derive_ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(owner, mint, token_program)
}

// ────────────────────────────────────────────────────────────────────────────
// V2 account resolution
// ────────────────────────────────────────────────────────────────────────────

use shared_types::token::TokenInfo;

/// All resolved accounts needed for a buy_v2 or sell_v2 instruction.
///
/// buy_v2 uses all 27 fields; sell_v2 uses all except `global_volume_accumulator`.
#[derive(Debug, Clone)]
pub struct ResolvedV2Accounts {
    pub global: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub base_token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub associated_token_program: Pubkey,
    pub fee_recipient: Pubkey,
    pub associated_quote_fee_recipient: Pubkey,
    pub buyback_fee_recipient: Pubkey,
    pub associated_quote_buyback_fee_recipient: Pubkey,
    pub bonding_curve: Pubkey,
    pub associated_base_bonding_curve: Pubkey,
    pub associated_quote_bonding_curve: Pubkey,
    pub user: Pubkey,
    pub associated_base_user: Pubkey,
    pub associated_quote_user: Pubkey,
    pub creator_vault: Pubkey,
    pub associated_creator_vault: Pubkey,
    pub sharing_config: Pubkey,
    pub global_volume_accumulator: Pubkey,
    pub user_volume_accumulator: Pubkey,
    pub associated_user_volume_accumulator: Pubkey,
    pub fee_config: Pubkey,
    pub fee_program: Pubkey,
    pub system_program: Pubkey,
    pub event_authority: Pubkey,
    pub program: Pubkey,
}

/// Resolve all accounts for a V2 buy/sell instruction.
///
/// Mirrors `PumpFunAddressProvider._get_v2_common_accounts()` from the Python reference.
/// All accounts are mandatory for V2 regardless of coin type.
pub fn resolve_v2_accounts(token_info: &TokenInfo, user: &Pubkey) -> ResolvedV2Accounts {
    let base_mint = token_info.mint;
    let base_token_program = token_info.base_token_program;
    let quote_mint = normalize_quote_mint(&token_info.quote_mint);
    let quote_token_program = token_info.quote_token_program;

    let bonding_curve = token_info
        .bonding_curve
        .unwrap_or_else(|| find_bonding_curve(&base_mint).0);

    let creator = token_info.creator.unwrap_or_default();
    let creator_vault = token_info
        .creator_vault
        .unwrap_or_else(|| find_creator_vault(&creator).0);

    let fee_recipient = pick_fee_recipient(token_info.is_mayhem_mode);
    let buyback_fee_recipient = pick_buyback_fee_recipient();
    let (user_volume_accumulator, _) = find_user_volume_accumulator(user);
    let (global_volume_accumulator, _) = find_global_volume_accumulator();

    ResolvedV2Accounts {
        global: PUMP_GLOBAL,
        base_mint,
        quote_mint,
        base_token_program,
        quote_token_program,
        associated_token_program: ASSOCIATED_TOKEN_PROGRAM,
        fee_recipient,
        associated_quote_fee_recipient: derive_ata(
            &fee_recipient,
            &quote_mint,
            &quote_token_program,
        ),
        buyback_fee_recipient,
        associated_quote_buyback_fee_recipient: derive_ata(
            &buyback_fee_recipient,
            &quote_mint,
            &quote_token_program,
        ),
        bonding_curve,
        associated_base_bonding_curve: derive_ata(&bonding_curve, &base_mint, &base_token_program),
        associated_quote_bonding_curve: derive_ata(
            &bonding_curve,
            &quote_mint,
            &quote_token_program,
        ),
        user: *user,
        associated_base_user: derive_ata(user, &base_mint, &base_token_program),
        associated_quote_user: derive_ata(user, &quote_mint, &quote_token_program),
        creator_vault,
        associated_creator_vault: derive_ata(&creator_vault, &quote_mint, &quote_token_program),
        sharing_config: find_sharing_config(&base_mint).0,
        global_volume_accumulator,
        user_volume_accumulator,
        associated_user_volume_accumulator: derive_ata(
            &user_volume_accumulator,
            &quote_mint,
            &quote_token_program,
        ),
        fee_config: find_fee_config().0,
        fee_program: PUMP_FEE_PROGRAM,
        system_program: SYSTEM_PROGRAM,
        event_authority: PUMP_EVENT_AUTHORITY,
        program: PUMP_PROGRAM,
    }
}
