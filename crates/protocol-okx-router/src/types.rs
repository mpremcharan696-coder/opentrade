//! Types, structures, and DEX enums matching OKX DEX Router V1 Anchor contract.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Supported DEX venues in the OKX Router V1 aggregation network.
#[derive(
    BorshSerialize, BorshDeserialize, Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize,
)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum Dex {
    SplTokenSwap = 0,
    StableSwap = 1,
    Whirlpool = 2,
    MeteoraDynamicpool = 3,
    RaydiumSwap = 4,
    RaydiumStableSwap = 5,
    RaydiumClmmSwap = 6,
    AldrinExchangeV1 = 7,
    AldrinExchangeV2 = 8,
    LifinityV1 = 9,
    LifinityV2 = 10,
    RaydiumClmmSwapV2 = 11,
    FluxBeam = 12,
    MeteoraDlmm = 13,
    RaydiumCpmmSwap = 14,
    OpenBookV2 = 15,
    WhirlpoolV2 = 16,
    Phoenix = 17,
    ObricV2 = 18,
    SanctumAddLiq = 19,
    SanctumRemoveLiq = 20,
    SanctumNonWsolSwap = 21,
    SanctumWsolSwap = 22,
    PumpfunBuy = 23,
    PumpfunSell = 24,
    StabbleSwap = 25,
    SanctumRouter = 26,
    MeteoraVaultDeposit = 27,
    MeteoraVaultWithdraw = 28,
    Saros = 29,
    MeteoraLst = 30,
    Solfi = 31,
    QualiaSwap = 32,
    Zerofi = 33,
    PumpfunammBuy = 34,
    PumpfunammSell = 35,
    Virtuals = 36,
    VertigoBuy = 37,
    VertigoSell = 38,
    PerpetualsAddLiq = 39,
    PerpetualsRemoveLiq = 40,
    PerpetualsSwap = 41,
    RaydiumLaunchpad = 42,
    LetsBonkFun = 43,
    Woofi = 44,
    MeteoraDbc = 45,
    MeteoraDlmmSwap2 = 46,
    MeteoraDAMMV2 = 47,
    Gavel = 48,
    BoopfunBuy = 49,
    BoopfunSell = 50,
    MeteoraDbc2 = 51,
    GooseFX = 52,
    Dooar = 53,
    Numeraire = 54,
    SaberDecimalWrapperDeposit = 55,
    SaberDecimalWrapperWithdraw = 56,
    SarosDlmm = 57,
    OneDexSwap = 58,
    Manifest = 59,
    ByrealClmm = 60,
    PancakeSwapV3Swap = 61,
    PancakeSwapV3SwapV2 = 62,
    Tessera = 63,
}

/// A specific route segment consisting of DEXs and proportional weights.
#[derive(BorshSerialize, BorshDeserialize, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Route {
    pub dexes: Vec<Dex>,
    pub weights: Vec<u8>,
}

/// Core swap arguments passed to OKX Router instruction.
#[derive(BorshSerialize, BorshDeserialize, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SwapArgs {
    pub amount_in: u64,
    pub expect_amount_out: u64,
    pub min_return: u64,
    pub amounts: Vec<u64>,
    pub routes: Vec<Vec<Route>>,
}

/// Anchor 8-byte discriminator for `global:swap_v3`.
pub const SWAP_V3_DISCRIMINATOR: [u8; 8] = [240, 224, 38, 33, 176, 31, 241, 175];

/// Wire instruction payload for calling `swap_v3` on OKX Router V1.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct SwapV3InstructionData {
    pub discriminator: [u8; 8],
    pub args: SwapArgs,
    pub commission_info: u32,
    pub platform_fee_rate: u16,
    pub order_id: u64,
}

impl SwapV3InstructionData {
    pub fn new(args: SwapArgs, platform_fee_rate_bps: u16, order_id: u64) -> Self {
        Self {
            discriminator: SWAP_V3_DISCRIMINATOR,
            args,
            commission_info: 0,
            platform_fee_rate: platform_fee_rate_bps,
            order_id,
        }
    }
}
