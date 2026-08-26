//! Standard block witness verifier implementation.

use crate::{BlockWitness, EvmExecutor, database::WitnessDatabase, witness::BlockWitnessChunkExt};
use itertools::Itertools;
use reth_primitives_traits::RecoveredBlock;
use sbv_primitives::{
    B256, U256,
    chainspec::ChainSpec,
    types::{reth::primitives::Block, rpc::ExecutionWitness},
};
use sbv_trie::{HashedPostState, KeccakKeyHasher, SparseState};
use std::sync::Arc;

#[cfg(feature = "scroll")]
mod scroll;

#[cfg(feature = "scroll")]
pub use scroll::*;

#[cfg(not(feature = "scroll"))]
mod ethereum;
use crate::error::StatelessValidationError;
#[cfg(not(feature = "scroll"))]
pub use ethereum::*;

/// Result of the block witness verification process.
#[derive(Debug)]
pub struct VerifyResult {
    /// Recovered blocks from the witnesses.
    pub blocks: Vec<RecoveredBlock<Block>>,
    /// Pre-state root of the first block.
    pub pre_state_root: B256,
    /// Post-state root after executing the witnesses.
    pub post_state_root: B256,
    /// Gas used during the verification process.
    pub gas_used: u64,

    /// Withdrawal root after executing the witnesses.
    #[cfg(feature = "scroll")]
    pub withdraw_root: B256,
    /// Next L2-to-L1 message index from Scroll's L2MessageQueue after executing the witnesses.
    ///
    /// Only extracted once the Tsuki hardfork is active. Before Tsuki the `nextMessageIndex`
    /// storage proof is not guaranteed to be part of the witness (and may be absent), so this is
    /// the sentinel `0` — **not** the real on-chain value. Consumers that fold this into proof
    /// public inputs must not treat a pre-Tsuki `0` as authoritative. See `l2_message_queue_info`.
    #[cfg(feature = "scroll")]
    pub next_message_index: u64,
}

/// Verify the block witness and return the gas used.
pub fn run(
    witnesses: &[BlockWitness],
    chain_spec: Arc<ChainSpec>,
    #[cfg(feature = "scroll")] compression_infos: Vec<Vec<(U256, usize)>>,
) -> Result<VerifyResult, StatelessValidationError> {
    if witnesses.is_empty() {
        return Err(StatelessValidationError::EmptyWitnesses);
    }
    if !witnesses.has_same_chain_id() {
        return Err(StatelessValidationError::InvalidAncestorChain);
    }
    if !witnesses.has_seq_block_number() {
        return Err(StatelessValidationError::InvalidAncestorChain);
    }
    if !witnesses.has_seq_state_root() {
        return Err(StatelessValidationError::InvalidAncestorChain);
    }

    let pre_state_root = witnesses[0].prev_state_root;
    let post_state_root = witnesses.last().unwrap().header.state_root;

    let execution_witness = ExecutionWitness {
        state: witnesses
            .iter()
            .flat_map(|w| w.states.iter().cloned())
            .collect(),
        codes: witnesses
            .iter()
            .flat_map(|w| w.codes.iter().cloned())
            .collect(),
        ..Default::default()
    };
    let (mut trie, bytecode) =
        SparseState::new(&execution_witness, pre_state_root).map_err(|source| {
            StatelessValidationError::SparseStateCreationFailed {
                pre_state_root,
                source,
            }
        })?;

    let blocks = witnesses
        .iter()
        .map(|w| {
            dev_trace!("{w:#?}");
            w.build_reth_block()
        })
        .collect::<Result<Vec<RecoveredBlock<Block>>, _>>()
        .map_err(|_| StatelessValidationError::SignerRecovery)?;

    if !blocks
        .iter()
        .tuple_windows()
        .all(|(a, b)| a.hash() == b.header().parent_hash)
    {
        return Err(StatelessValidationError::InvalidAncestorChain);
    }

    let mut gas_used = 0;

    #[cfg(not(feature = "scroll"))]
    let compression_infos = std::iter::repeat_n::<Vec<(U256, usize)>>(vec![], blocks.len());

    #[cfg(not(feature = "scroll"))]
    let block_hashes = import_block_hashes(witnesses);
    #[cfg(feature = "scroll")]
    let block_hashes = Default::default();

    for (block, _compression_infos) in blocks.iter().zip_eq(compression_infos) {
        let db = WitnessDatabase::new(&trie, &bytecode, &block_hashes);

        #[cfg(not(feature = "scroll"))]
        let executor = EvmExecutor::new(chain_spec.clone(), db, block);

        #[cfg(feature = "scroll")]
        let executor = EvmExecutor::new(chain_spec.clone(), db, block, Some(_compression_infos));

        let output = executor.execute().map_err(|source| {
            StatelessValidationError::StatelessExecutionFailed {
                block_number: block.number,
                source,
            }
        })?;
        gas_used += output.gas_used;

        // Compute and check the post state root
        let hashed_state =
            HashedPostState::from_bundle_state::<KeccakKeyHasher>(&output.state.state);
        let state_root = trie.calculate_state_root(hashed_state);

        if block.state_root != state_root {
            dev_error!(
                "Block #{} root mismatch: root after in trace = {:x}, root after in reth = {:x}",
                block.number,
                block.state_root,
                post_state_root
            );
            return Err(StatelessValidationError::PostStateRootMismatch {
                got: state_root,
                expected: block.state_root,
            });
        }
    }

    #[cfg(feature = "scroll")]
    let (withdraw_root, next_message_index) = l2_message_queue_info(
        &chain_spec,
        blocks.last().as_ref().expect("witnesses can not be empty"),
        &trie,
    )
    .map_err(|source| map_l2_message_queue_info_error(blocks.last().unwrap().number, source))?;

    Ok(VerifyResult {
        blocks,
        pre_state_root,
        post_state_root,
        gas_used,
        #[cfg(feature = "scroll")]
        withdraw_root,
        #[cfg(feature = "scroll")]
        next_message_index,
    })
}

#[cfg(feature = "scroll")]
fn map_l2_message_queue_info_error(
    block_number: u64,
    source: sbv_primitives::types::reth::evm::execute::ProviderError,
) -> StatelessValidationError {
    StatelessValidationError::L2MessageQueueInfoFailed {
        block_number,
        source,
    }
}

#[cfg(all(test, feature = "scroll"))]
mod tests {
    use super::*;
    use sbv_primitives::types::reth::evm::execute::ProviderError;
    use std::io;

    #[test]
    fn queue_info_errors_are_not_classified_as_execution_failures() {
        let error = map_l2_message_queue_info_error(
            44,
            ProviderError::other(io::Error::other("missing queue proof")),
        );

        assert!(matches!(
            error,
            StatelessValidationError::L2MessageQueueInfoFailed {
                block_number: 44,
                ..
            }
        ));
    }
}
