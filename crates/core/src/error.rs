//! Errors that can occur during stateless validation.

use sbv_primitives::{B256, types::reth::evm::block::BlockExecutionError};

#[cfg(feature = "scroll")]
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
    #[cfg(feature = "scroll")]
    #[error("failed to read L2 message queue info after block {block_number}: {source}")]
    L2MessageQueueInfoFailed {
        /// The number of the block whose post-state queue values could not be read.
        block_number: u64,
        /// The typed provider/trie error returned by the queue read.
        #[source]
        source: ProviderError,
    },

    /// The post-state root computed from the witness does not match the expected post-state root in the block header.
    #[error("mismatched post-state root: {got}\n {expected}")]
    PostStateRootMismatch {
        /// The computed post-state root
        got: B256,
        /// The expected post-state root; in the block header
        expected: B256,
    },

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

    #[cfg(feature = "scroll")]
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
