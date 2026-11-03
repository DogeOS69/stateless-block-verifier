//! Genesis headers synced against dogeos-reth `tsuki-v0.12` (`23a1737c`).
//!
//! State roots were computed once from the upstream allocations on the host. Keep these
//! headers in sync when repinning dogeos-reth; the host parity tests compare every field.

use alloy_consensus::Header;
use alloy_primitives::{U256, b256, bytes};

pub(super) fn mainnet() -> Header {
    Header {
        state_root: b256!("b664af5befb987dc28f8698efd26d819c44299c6c7dae42822e9fb4d2fcab0e9"),
        gas_limit: 30_000_000,
        difficulty: U256::from(1),
        base_fee_per_gas: Some(1_000_000_000),
        ..Default::default()
    }
}

pub(super) fn chikyu() -> Header {
    Header {
        state_root: b256!("81c777d8b8d3f71224b8e8d6c38e84efa20ff55bcecfd7ff74a5ce108db5a4f2"),
        gas_limit: 10_000_000,
        difficulty: U256::from(1),
        timestamp: 1_760_027_426,
        extra_data: bytes!(
            "0000000000000000000000000000000000000000000000000000000000000000c91396c888e02be45d4716efded42529668556350000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        ),
        base_fee_per_gas: Some(100_000_000),
        ..Default::default()
    }
}

pub(super) fn dev() -> Header {
    Header {
        state_root: b256!("adde919431c299f64206fb3e78bc542663625322e3ff94d698fa000d68acd3fe"),
        gas_limit: 20_000_000,
        timestamp: 1_687_223_762,
        base_fee_per_gas: Some(0),
        ..Default::default()
    }
}

/// DogeOS DevNet (chain 221122) block 0, read from a DevNet node; it has no published genesis
/// document here.
pub(super) fn devnet() -> Header {
    Header {
        state_root: b256!("a325dcb6788ee5e537bd469ed956d479de42652939983160b0ad502f2a3e4a58"),
        gas_limit: 30_000_000,
        difficulty: U256::from(1),
        base_fee_per_gas: Some(100_000_000),
        ..Default::default()
    }
}
