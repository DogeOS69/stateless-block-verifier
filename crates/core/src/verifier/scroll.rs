use crate::{
    BlockWitness,
    verifier::{VerifyResult, run},
};
use reth_stateless::{StatelessTrie, validation::StatelessValidationError};
use sbv_primitives::{
    Address, B256, U256, chainspec::ChainSpec, types::reth::evm::execute::ProviderError,
};
use sbv_trie::SparseState;
use std::{io, sync::Arc};

/// L2MessageQueue pre-deployed address.
pub const L2_MESSAGE_QUEUE: Address =
    sbv_primitives::address!("5300000000000000000000000000000000000000");
/// Storage slot of messageRoot in L2MessageQueue.
pub const WITHDRAW_TRIE_ROOT_SLOT: U256 = U256::ZERO;
/// Storage slot of nextMessageIndex in L2MessageQueue (inherited from AppendOnlyMerkleTree).
pub const NEXT_MESSAGE_INDEX_SLOT: U256 = U256::from_limbs([1, 0, 0, 0]);

/// State commit mode for the block witness verification process.
#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize),
    rkyv(derive(Debug, Hash, PartialEq, Eq))
)]
pub enum StateCommitMode {
    /// Commit state by chunk.
    Chunk,
    /// Commit state by block.
    Block,
    /// Use chunk mode first if it fails, fallback to block mode.
    Auto,
}

/// Verify the block witness and return the gas used.
pub fn run_host(
    witnesses: &[BlockWitness],
    chain_spec: Arc<ChainSpec>,
) -> Result<VerifyResult, StatelessValidationError> {
    let compression_infos = witnesses
        .iter()
        .map(|block| block.compression_infos())
        .collect::<Vec<_>>();
    run(witnesses, chain_spec, compression_infos)
}

/// Get the Scroll L2 message queue outputs committed after block execution.
///
/// Note: `withdraw_root` here should not be confused with the withdrawal root of the beacon
/// chain.
pub(super) fn l2_message_queue_info(state: &SparseState) -> Result<(B256, u64), ProviderError> {
    // storage access **MUST** load account first, see: [`SparseState::storage`]
    let _account = state.account(L2_MESSAGE_QUEUE)?.ok_or_else(|| {
        ProviderError::other(io::Error::new(
            io::ErrorKind::NotFound,
            format!("L2MessageQueue contract not found at {L2_MESSAGE_QUEUE}"),
        ))
    })?;
    let withdraw_root = state.storage(L2_MESSAGE_QUEUE, WITHDRAW_TRIE_ROOT_SLOT)?;
    let next_message_index = state.storage(L2_MESSAGE_QUEUE, NEXT_MESSAGE_INDEX_SLOT)?;
    Ok((withdraw_root.into(), next_message_index.to()))
}

#[cfg(test)]
#[cfg(feature = "scroll-compress-info")]
mod tests {
    use super::*;
    use sbv_primitives::{
        chainspec::{Chain, build_chain_spec_force_hardfork, get_chain_spec},
        hardforks::Hardfork,
    };

    #[rstest::rstest]
    fn test_euclid_v2(
        #[files("../../testdata/scroll/euclidv2/*.json")]
        #[mode = str]
        witness_json: &str,
    ) {
        let witness: BlockWitness = serde_json::from_str(witness_json).unwrap();
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::EuclidV2);
        run_host(&[witness], chain_spec).unwrap();
    }

    #[rstest::rstest]
    fn test_feynman(
        #[files("../../testdata/scroll/feynman/*.json")]
        #[mode = str]
        witness_json: &str,
    ) {
        let witness: BlockWitness = serde_json::from_str(witness_json).unwrap();
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Feynman);
        run_host(&[witness], chain_spec).unwrap();
    }

    #[test]
    fn test_next_message_index_feynman_fixture() {
        let witness: BlockWitness = serde_json::from_str(include_str!(
            "../../../../testdata/dogeos/next-message-index/20240125.json"
        ))
        .unwrap();
        let chain_spec = get_chain_spec(Chain::from_id(witness.chain_id)).unwrap();

        let result = run_host(&[witness], chain_spec).unwrap();

        assert_eq!(result.next_message_index, 208530);
    }
}
