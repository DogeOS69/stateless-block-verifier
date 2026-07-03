//! Errors that can occur during stateless validation.

use sbv_primitives::B256;

/// Errors that can occur during stateless validation.
#[derive(Debug, thiserror::Error)]
pub enum StatelessValidationError {
    /// The block witness is empty.
    #[error("empty witnesses")]
    EmptyWitnesses,

    /// The ancestor chain of the block witness is invalid.
    #[error("invalid ancestor chain")]
    InvalidAncestorChain,

    /// The SparseState could not be created from the witness data.
    #[error("sparse state creation failed")]
    SparseStateCreationFailed,

    /// The block witness failed to execute.
    #[error("stateless block execution failed: {0}")]
    StatelessExecutionFailed(String),

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
