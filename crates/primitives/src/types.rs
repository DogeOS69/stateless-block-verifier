/// re-export types from alloy_consensus
pub mod consensus {
    pub use alloy_consensus::{
        Block, BlockHeader, Header, SignableTransaction, Transaction, TxEip1559, TxEip2930,
        TxEip4844, TxEip4844Variant, TxEip4844WithSidecar, TxEip7702, TxLegacy, Typed2718,
        transaction::{SignerRecoverable, TxHashRef},
    };

    pub use dogeos_protocol_types::{
        ScrollReceiptEnvelope as ReceiptEnvelope, ScrollTransaction,
        ScrollTxEnvelope as TxEnvelope, ScrollTxType as TxType,
        ScrollTypedTransaction as TypedTransaction, TxL1Message,
    };

    /// Stable serde representations used by persisted block witnesses.
    pub mod serde_bincode_compat {
        pub use alloy_consensus::serde_bincode_compat::Header;
        pub use dogeos_protocol_types::serde_bincode_compat::ScrollTxEnvelope as TxEnvelope;
    }
}
pub use consensus::{Header, TypedTransaction as AlloyTypedTransaction};

/// re-export types from alloy_eips
pub use alloy_eips as eips;

/// re-export types from alloy-evm
#[cfg(feature = "evm-types")]
pub mod evm {
    pub use alloy_evm::{Evm, EvmEnv, precompiles};

    pub use dogeos_reth_evm::{
        DEFAULT_BASE_FEE_OVERHEAD, MAX_L2_BASE_FEE, ReceiptBuilderCtx, ScrollBaseFeeProvider,
        ScrollBlockExecutionCtx, ScrollBlockExecutor, ScrollBlockExecutorFactory,
        ScrollDefaultPrecompilesFactory, ScrollEvmFactory, ScrollReceiptBuilder,
        spec_id_at_timestamp_and_number,
    };

    #[cfg(feature = "scroll-compress-info")]
    pub use dogeos_reth_evm::{compute_compressed_size, compute_compression_ratio};

    pub use dogeos_reth_evm::{ScrollTxCompressionInfo, ScrollTxCompressionInfos};
}

/// Scroll RPC network type
#[cfg(feature = "network-types")]
pub mod network {
    /// Network definition
    pub type Network = dogeos_rpc_types::Scroll;
}
#[cfg(feature = "network-types")]
pub use network::*;

/// re-export types from revm
#[cfg(feature = "revm-types")]
pub mod revm {
    pub use revm::{
        bytecode::Bytecode,
        context::{BlockEnv, CfgEnv},
        database, precompile,
        state::AccountInfo,
    };

    pub use revm_scroll::{
        ScrollSpecId as SpecId, builder::ScrollCfgExt, precompile::ScrollPrecompileProvider,
    };
}

/// re-export types from reth_primitives
pub mod reth {
    /// Re-export the consensus traits and the DogeOS validator.
    #[cfg(feature = "reth-consensus-types")]
    pub mod consensus {
        pub use reth_consensus::*;

        pub use dogeos_reth_consensus::{
            DogeosConsensus, DogeosConsensusError, validate_gas_limit_against_parent,
        };
    }

    /// Re-export types from `reth-primitives-types`
    pub mod primitives {
        pub use reth_primitives_traits::{RecoveredBlock, SealedBlock, SealedHeader};

        pub use dogeos_reth_primitives::{
            DogeosBlock as Block, DogeosBlockBody as BlockBody, DogeosPrimitives as EthPrimitives,
            ScrollReceipt as Receipt, ScrollTransactionSigned as TransactionSigned,
        };

        pub use reth_primitives_traits::transaction::signed::SignedTransaction;
    }

    /// Re-export Reth execution traits and the Scroll receipt builder
    #[cfg(feature = "reth-evm-types")]
    pub mod evm {
        pub use reth_evm::*;

        pub use crate::types::scroll::RethReceiptBuilder;
    }

    #[cfg(feature = "reth-execution-types")]
    pub use reth_execution_types as execution_types;
}

#[cfg(feature = "reth-evm-types")]
mod scroll {
    use alloy_consensus::{Eip658Value, Receipt};
    use alloy_evm::Evm;
    use dogeos_protocol_types::ScrollTransactionReceipt;
    use dogeos_reth_evm::{ReceiptBuilderCtx, ScrollReceiptBuilder};
    use dogeos_reth_primitives::{ScrollReceipt, ScrollTransactionSigned, ScrollTxType};

    /// Compatibility copy of `dogeos_reth_evm::ScrollRethReceiptBuilder`.
    ///
    /// This is not a `no_std` requirement: the zkVM guest is built with Rust's standard library.
    /// The blocker is `reth-primitives-traits` 0.1.1, whose `std` feature unconditionally enables
    /// `quanta` 0.12.6. `quanta` selects its Unix clock backend for every non-Windows, non-Wasm
    /// target, so `target_os = "zkvm"` tries to use libc's unavailable `timespec`,
    /// `clock_gettime`, and `CLOCK_MONOTONIC` symbols.
    ///
    /// Upstream reth-core fixed this by making `quanta` opt-in and falling back to
    /// `std::time::Instant` when it is disabled:
    /// <https://github.com/paradigmxyz/reth-core/commit/4342fddd67d3c2714f1bc4c4ac6725b1923b9a6d>.
    /// Once that fix is available in a compatible pinned `reth-primitives-traits` release, enable
    /// `dogeos-reth-evm/std`, re-export `ScrollRethReceiptBuilder`, and delete this copy together
    /// with the manual EVM configuration in `sbv-core`.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct RethReceiptBuilder;

    impl ScrollReceiptBuilder for RethReceiptBuilder {
        type Transaction = ScrollTransactionSigned;
        type Receipt = ScrollReceipt;

        fn build_receipt<E: Evm>(&self, ctx: ReceiptBuilderCtx<E>) -> Self::Receipt {
            let inner = Receipt {
                status: Eip658Value::Eip658(ctx.result.is_success()),
                cumulative_gas_used: ctx.cumulative_gas_used,
                logs: ctx.result.into_logs(),
            };
            let with_l1_fee = |inner| ScrollTransactionReceipt::new(inner, ctx.l1_fee);

            match ScrollTxType::try_from(ctx.tx_type).expect("unexpected Scroll transaction type") {
                ScrollTxType::Legacy => ScrollReceipt::Legacy(with_l1_fee(inner)),
                ScrollTxType::Eip2930 => ScrollReceipt::Eip2930(with_l1_fee(inner)),
                ScrollTxType::Eip1559 => ScrollReceipt::Eip1559(with_l1_fee(inner)),
                ScrollTxType::Eip7702 => ScrollReceipt::Eip7702(with_l1_fee(inner)),
                ScrollTxType::L1Message => ScrollReceipt::L1Message(inner),
            }
        }
    }
}

/// re-export types from alloy_rpc_types_eth
pub mod rpc {
    pub use alloy_rpc_types_eth::{Header, TransactionTrait};

    pub use alloy_rpc_types_debug::ExecutionWitness;
    pub use dogeos_rpc_types::{
        ScrollRpcTransaction as Transaction, ScrollTransactionReceipt as TransactionReceipt,
        ScrollTransactionRequest as TransactionRequest,
    };

    /// Transaction object used in RPC.
    #[allow(unused_qualifications)]
    pub type RpcTransaction<T = super::consensus::TxEnvelope> = alloy_rpc_types_eth::Transaction<T>;

    /// Block representation for RPC.
    pub type Block = alloy_rpc_types_eth::Block<Transaction>;
}
