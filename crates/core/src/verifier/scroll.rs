use crate::{
    BlockWitness,
    error::StatelessValidationError,
    verifier::{VerifyResult, run},
};
use reth_primitives_traits::RecoveredBlock;
use sbv_primitives::{
    Address, B256, U256,
    chainspec::ChainSpec,
    hardforks::DogeosHardforks,
    types::{
        consensus::BlockHeader,
        reth::{evm::execute::ProviderError, primitives::Block},
    },
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
pub(super) fn l2_message_queue_info(
    chain_spec: &ChainSpec,
    block: &RecoveredBlock<Block>,
    state: &SparseState,
) -> Result<(B256, u64), ProviderError> {
    // storage access **MUST** load account first, see: [`SparseState::storage`]
    let _account = state.account(L2_MESSAGE_QUEUE)?.ok_or_else(|| {
        ProviderError::other(io::Error::new(
            io::ErrorKind::NotFound,
            format!("L2MessageQueue contract not found at {L2_MESSAGE_QUEUE}"),
        ))
    })?;
    let withdraw_root = state.storage(L2_MESSAGE_QUEUE, WITHDRAW_TRIE_ROOT_SLOT)?;

    // The `nextMessageIndex` slot proof is only guaranteed to be part of the committed witness once
    // the Tsuki hardfork is active. Before Tsuki it is not guaranteed and may be absent, so reading
    // it could fail; we return the sentinel `0` instead. NOTE: `0` here is a sentinel, not
    // necessarily the real on-chain `nextMessageIndex` for a pre-Tsuki block. See
    // `dogeos/changes/next-message-index.md`.
    let next_message_index: u64 = if chain_spec.is_tsuki_active_at_timestamp(block.timestamp()) {
        next_message_index_from_value(state.storage(L2_MESSAGE_QUEUE, NEXT_MESSAGE_INDEX_SLOT)?)?
    } else {
        0
    };
    Ok((withdraw_root.into(), next_message_index))
}

/// Converts a raw `nextMessageIndex` storage value into a `u64`, returning an error instead of
/// panicking when the value does not fit.
///
/// Scroll's `L2MessageQueue.nextMessageIndex` is a monotonic counter that fits comfortably in a
/// `u64`, but the witness data is untrusted, so an out-of-range value must surface as an error
/// rather than aborting the verifier via a panicking `U256::to()`.
fn next_message_index_from_value(next_message_index: U256) -> Result<u64, ProviderError> {
    u64::try_from(next_message_index).map_err(|_| {
        ProviderError::other(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("nextMessageIndex does not fit into u64: {next_message_index}"),
        ))
    })
}

#[cfg(test)]
#[cfg(feature = "scroll-compress-info")]
mod tests {
    use super::*;
    use sbv_primitives::{
        chainspec::{Chain, build_chain_spec_force_hardfork},
        hardforks::Hardfork,
    };

    #[rstest::rstest]
    fn test_feynman(
        #[files("../../testdata/scroll/feynman/*.json")]
        #[mode = str]
        witness_json: &str,
    ) {
        let witness: BlockWitness = serde_json::from_str(witness_json).unwrap();
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Feynman);
        let result = run_host(&[witness], chain_spec).unwrap();
        assert_eq!(result.next_message_index, 0);
    }

    /// A pre-backfill Feynman witness must verify without reading the absent slot-1 proof.
    ///
    /// Keep this fixture in its original pre-backfill form. If the pre-Tsuki gate is removed,
    /// verification fails with `MPT: Unresolved node access` instead of returning the sentinel `0`.
    #[test]
    fn test_next_message_index_pre_tsuki_missing_slot1_proof_is_sentinel() {
        let witness: BlockWitness = serde_json::from_str(include_str!(
            "../../../../testdata/scroll/feynman/534352-19604670.json"
        ))
        .unwrap();
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Feynman);

        let result = run_host(&[witness], chain_spec).unwrap();

        assert_eq!(result.next_message_index, 0);
    }

    /// Real DogeOS witnesses from a Tsuki materializer run must replay as a contiguous chunk.
    ///
    /// These blocks contain no L1 messages, so the queue index remains zero. The forced Tsuki
    /// chain spec is intentional: the local fixture predates the public Chikyū activation time,
    /// but was produced by a Tsuki-enabled development network.
    #[test]
    fn test_next_message_index_tsuki_empty_queue_witnesses() {
        let witnesses = [
            include_str!(
                "../../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"
            ),
            include_str!(
                "../../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"
            ),
            include_str!(
                "../../../../testdata/dogeos/next-message-index/tsuki-empty-queue/13.json"
            ),
        ]
        .map(|json| serde_json::from_str::<BlockWitness>(json).unwrap());

        assert!(
            witnesses
                .iter()
                .all(|witness| witness.chain_id == 6_281_971)
        );
        assert_eq!(
            witnesses
                .iter()
                .map(|witness| witness.header.number)
                .collect::<Vec<_>>(),
            [11, 12, 13]
        );

        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(6_281_971), Hardfork::Tsuki);
        let result = run_host(&witnesses, chain_spec).unwrap();

        assert_eq!(result.next_message_index, 0);
    }

    /// Genuine Tsuki-enabled DogeOS local-node transition fixture.
    ///
    /// Block 19's first transaction calls `withdrawToL1(address)`, enqueueing the first L2-to-L1
    /// withdrawal message and changing the authenticated queue slot from `0` in its parent state
    /// to `1` in its final state. The forced Tsuki chain spec is intentional: this development
    /// fixture predates a public Chikyū activation and the registered Chikyū spec does not activate
    /// Tsuki.
    #[test]
    fn test_next_message_index_post_tsuki_transition() {
        const EXPECTED_NEXT_MESSAGE_INDEX: u64 = 1;
        const EXPECTED_BLOCK_HASH: B256 = sbv_primitives::b256!(
            "17ce064e49dc6f59353d35487eee042140490b885e73275a69ad23982494ecbc"
        );
        const EXPECTED_PARENT_STATE_ROOT: B256 = sbv_primitives::b256!(
            "80479f9622ccb9d62a40ee02a217494c17413163ab10596ba189ab6be3901c88"
        );
        const EXPECTED_POST_STATE_ROOT: B256 = sbv_primitives::b256!(
            "0824e511633e44abe11ded8c271dbfcecea21263cb37fc84d1835dc379b5724d"
        );

        let witness: BlockWitness = serde_json::from_str(include_str!(
            "../../../../testdata/dogeos/next-message-index/6281971-19.json"
        ))
        .unwrap();
        assert_ne!(EXPECTED_NEXT_MESSAGE_INDEX, 0);
        assert_eq!(witness.chain_id, 6_281_971);
        assert_eq!(witness.header.number, 19);
        assert_eq!(witness.prev_state_root, EXPECTED_PARENT_STATE_ROOT);
        assert_eq!(witness.header.state_root, EXPECTED_POST_STATE_ROOT);

        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Tsuki);
        assert!(chain_spec.is_tsuki_active_at_timestamp(witness.header.timestamp));

        let result = run_host(&[witness], chain_spec).unwrap();
        assert_eq!(result.blocks[0].hash(), EXPECTED_BLOCK_HASH);
        assert_eq!(result.next_message_index, EXPECTED_NEXT_MESSAGE_INDEX);
    }

    #[test]
    fn test_next_message_index_overflow() {
        let err = next_message_index_from_value(U256::from(u64::MAX) + U256::from(1_u8))
            .expect_err("values above u64::MAX must be rejected");

        assert!(
            err.to_string()
                .contains("nextMessageIndex does not fit into u64")
        );
    }
}
