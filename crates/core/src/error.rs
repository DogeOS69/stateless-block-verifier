//! Errors that can occur during stateless validation.

use sbv_primitives::{B256, types::reth::evm::block::BlockExecutionError};

use sbv_primitives::types::reth::evm::execute::ProviderError;

/// Errors that can occur during stateless validation.
#[derive(Debug, thiserror::Error)]
pub enum StatelessValidationError {
    /// The block witness is empty.
    #[error("empty witnesses")]
    EmptyWitnesses,

    /// The ancestor chain of the block witness is invalid.
    #[error("invalid ancestor chain")]
    InvalidAncestorChain,

    /// The sparse state could not be created from the witness data.
    #[error("failed to create sparse state for pre-state root {pre_state_root}: {source}")]
    SparseStateCreationFailed {
        /// The pre-state root that the witness was expected to reveal.
        pre_state_root: B256,
        /// The exact RLP decoding error returned by sparse-state construction.
        #[source]
        source: alloy_rlp::Error,
    },

    /// The block witness failed to execute.
    #[error("stateless execution failed for block {block_number}: {source}")]
    StatelessExecutionFailed {
        /// The number of the block that failed to execute.
        block_number: u64,
        /// The typed block-execution error.
        #[source]
        source: BlockExecutionError,
    },

    /// Reading Scroll's post-execution L2 message queue outputs failed.
    #[error("failed to read L2 message queue info after block {block_number}: {source}")]
    L2MessageQueueInfoFailed {
        /// The number of the block whose post-state queue values could not be read.
        block_number: u64,
        /// The typed provider/trie error returned by the queue read.
        #[source]
        source: ProviderError,
    },

    /// A chunk that must be anchored to a trusted parent header was verified without one.
    #[error("missing trusted parent header for block {block_number}")]
    MissingParentHeader {
        /// The number of the first block in the chunk.
        block_number: u64,
    },

    /// A block header does not satisfy the DogeOS header rules on its own.
    #[error("invalid header for block {block_number}: {source}")]
    InvalidHeader {
        /// The number of the offending block.
        block_number: u64,
        /// The typed consensus error describing which rule the header breaks.
        #[source]
        source: sbv_primitives::types::reth::consensus::ConsensusError,
    },

    /// A block header does not satisfy the DogeOS header rules that relate it to its parent.
    #[error("invalid header for block {block_number} against its parent: {source}")]
    InvalidHeaderAgainstParent {
        /// The number of the offending block.
        block_number: u64,
        /// The typed consensus error describing which rule the pair breaks.
        #[source]
        source: sbv_primitives::types::reth::consensus::ConsensusError,
    },

    /// The chain's L2SystemConfig contract address is unset, so the base-fee overhead has no
    /// authenticated source.
    #[error(
        "chain {chain_id} has no L2SystemConfig address, so the base-fee overhead for block {block_number} cannot be read"
    )]
    L2SystemConfigUnconfigured {
        /// The number of the block whose base fee could not be derived.
        block_number: u64,
        /// The chain the proof claims to be for.
        chain_id: u64,
    },

    /// The parent header a block's base fee is derived from carries no base fee.
    #[error("the parent of block {block_number} carries no base fee")]
    ParentBaseFeeMissing {
        /// The number of the block whose base fee could not be derived.
        block_number: u64,
    },

    /// The first block does not link to the trusted parent header.
    #[error("parent header mismatch: parent hash {got}, trusted parent {expected}")]
    ParentHashMismatch {
        /// The parent hash the first block's header claims.
        got: B256,
        /// The hash of the trusted parent header.
        expected: B256,
    },

    /// The first block does not directly follow the trusted parent header.
    #[error("parent number mismatch: first block {got}, expected {expected}")]
    ParentNumberMismatch {
        /// The number of the first block.
        got: u64,
        /// The number the first block must have: one above the trusted parent's.
        expected: u64,
    },

    /// The trusted parent header's state root is not the pre-state root the witness reveals.
    #[error("parent state root mismatch: witness pre-state root {got}, trusted parent {expected}")]
    ParentStateRootMismatch {
        /// The pre-state root the first witness declares.
        got: B256,
        /// The state root of the trusted parent header.
        expected: B256,
    },

    /// Reading the L2 base-fee overhead from a block's pre-state failed.
    #[error("failed to read the base-fee overhead for block {block_number}: {source}")]
    BaseFeeOverheadReadFailed {
        /// The number of the block whose pre-state could not be read.
        block_number: u64,
        /// The typed provider/trie error returned by the read.
        #[source]
        source: ProviderError,
    },

    /// A block header's base fee is not the one the protocol rule derives from its parent.
    #[error("base fee mismatch for block {block_number}: header {got}, expected {expected}")]
    BaseFeeMismatch {
        /// The number of the offending block.
        block_number: u64,
        /// The base fee the header claims.
        got: u64,
        /// The base fee the protocol rule derives from the parent header and the pre-state.
        expected: u64,
    },

    /// A block header's `gas_used` is not the gas the block actually spent.
    #[error("gas used mismatch for block {block_number}: header {got}, executed {expected}")]
    GasUsedMismatch {
        /// The number of the offending block.
        block_number: u64,
        /// The gas the header claims.
        got: u64,
        /// The gas the block's execution actually spent.
        expected: u64,
    },

    /// A block's body does not match the roots committed in its header.
    #[error("body validation failed for block {block_number}: {source}")]
    BodyValidationFailed {
        /// The number of the offending block.
        block_number: u64,
        /// The typed consensus error describing which commitment disagrees.
        #[source]
        source: sbv_primitives::types::reth::consensus::ConsensusError,
    },

    /// The post-state root computed from the witness does not match the expected post-state root in the block header.
    #[error("mismatched post-state root: {got}\n {expected}")]
    PostStateRootMismatch {
        /// The computed post-state root
        got: B256,
        /// The expected post-state root; in the block header
        expected: B256,
    },

    /// The number of compression infos for a block does not match its transaction count.
    #[error("invalid compression infos")]
    InvalidCompressionInfos,

    /// Errors related to signature recovery and verification
    #[error("signer recovery failed")]
    SignerRecovery,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{error::Error, io};

    #[test]
    fn execution_error_retains_block_context_and_source_chain() {
        let error = StatelessValidationError::StatelessExecutionFailed {
            block_number: 42,
            source: BlockExecutionError::other(io::Error::other("executor source")),
        };

        assert_eq!(
            error.to_string(),
            "stateless execution failed for block 42: executor source"
        );
        let source = error.source().expect("execution source must be retained");
        assert_eq!(source.to_string(), "executor source");
        assert!(source.downcast_ref::<BlockExecutionError>().is_some());
    }

    #[test]
    fn queue_error_retains_block_context_and_source_chain() {
        let error = StatelessValidationError::L2MessageQueueInfoFailed {
            block_number: 43,
            source: ProviderError::other(io::Error::other("queue source")),
        };

        assert_eq!(
            error.to_string(),
            "failed to read L2 message queue info after block 43: queue source"
        );
        let source = error.source().expect("queue source must be retained");
        assert_eq!(source.to_string(), "queue source");
        assert!(source.downcast_ref::<ProviderError>().is_some());
    }
}
