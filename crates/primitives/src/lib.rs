//! Stateless Block Verifier primitives library.

/// The spec of a DogeOS network
#[cfg(feature = "chainspec")]
pub mod chainspec;

/// DogeOS hardforks and shared fork traits
#[cfg(feature = "hardforks")]
pub mod hardforks {
    pub use reth_ethereum_forks::{Hardfork as HardforkTrait, *};

    pub use dogeos_hardforks::{DogeosHardfork as Hardfork, DogeosHardforks};
}

/// Legacy Types definition leave for backward compatibility
pub mod legacy_types;

/// Types definition
pub mod types;

pub use alloy_primitives::{
    self, Address, B64, B256, BlockHash, BlockNumber, Bloom, Bytes, ChainId, Signature,
    SignatureError, TxHash, U8, U256, address, b256, keccak256,
};
