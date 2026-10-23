//! Scroll block witness verifier implementation.

use crate::{
    BlockWitness, EvmExecutor, database::WitnessDatabase, error::StatelessValidationError,
    witness::BlockWitnessChunkExt,
};
use itertools::Itertools;
use reth_primitives_traits::RecoveredBlock;
use sbv_primitives::{
    Address, B256, U256,
    chainspec::ChainSpec,
    hardforks::DogeosHardforks,
    types::{
        consensus::BlockHeader,
        reth::{evm::execute::ProviderError, primitives::Block},
        rpc::ExecutionWitness,
    },
};
use sbv_trie::{HashedPostState, KeccakKeyHasher, SparseState};
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
pub enum StateCommitMode {
    /// Commit state by chunk.
    Chunk,
    /// Commit state by block.
    Block,
    /// Use chunk mode first if it fails, fallback to block mode.
    Auto,
}

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
    pub withdraw_root: B256,
    /// Next L2-to-L1 message index from Scroll's L2MessageQueue after executing the witnesses.
    ///
    /// Only extracted once the Tsuki hardfork is active. Before Tsuki the `nextMessageIndex`
    /// storage proof is not guaranteed to be part of the witness (and may be absent), so this is
    /// the sentinel `0` — **not** the real on-chain value. Consumers that fold this into proof
    /// public inputs must not treat a pre-Tsuki `0` as authoritative. See `l2_message_queue_info`.
    pub next_message_index: u64,
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

/// Verify the block witness and return the gas used.
pub fn run(
    witnesses: &[BlockWitness],
    chain_spec: Arc<ChainSpec>,
    compression_infos: Vec<Vec<(U256, usize)>>,
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
    if compression_infos.len() != witnesses.len()
        || !witnesses
            .iter()
            .zip(&compression_infos)
            .all(|(witness, infos)| infos.len() == witness.transactions.len())
    {
        return Err(StatelessValidationError::InvalidCompressionInfos);
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

    for (block, compression_infos) in blocks.iter().zip_eq(compression_infos) {
        let db = WitnessDatabase::new(&trie, &bytecode);

        let executor = EvmExecutor::new(chain_spec.clone(), db, block, Some(compression_infos));

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
        withdraw_root,
        next_message_index,
    })
}

fn map_l2_message_queue_info_error(
    block_number: u64,
    source: ProviderError,
) -> StatelessValidationError {
    StatelessValidationError::L2MessageQueueInfoFailed {
        block_number,
        source,
    }
}

/// Get the Scroll L2 message queue outputs committed after block execution.
///
/// Note: `withdraw_root` here should not be confused with the withdrawal root of the beacon
/// chain.
fn l2_message_queue_info(
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
mod tests {
    use super::*;

    /// Runs under the guest feature set (without `scroll-compress-info`): mismatched
    /// compression infos are rejected before execution, so placeholder values suffice.
    #[test]
    fn compression_info_count_mismatches_are_rejected_without_compress_info_feature() {
        use sbv_primitives::{
            chainspec::{Chain, build_chain_spec_force_hardfork},
            hardforks::Hardfork,
        };

        let witnesses = [
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"),
        ]
        .map(|json| serde_json::from_str::<BlockWitness>(json).unwrap());
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(6_281_971), Hardfork::Tsuki);
        let placeholder = |n: usize| vec![(U256::ZERO, 0); n];
        let txs = witnesses[1].transactions.len();
        assert!(txs > 0, "fixture's second block must have transactions");

        for infos in [placeholder(txs - 1), placeholder(txs + 1)] {
            let all = vec![placeholder(witnesses[0].transactions.len()), infos];
            assert!(matches!(
                run(&witnesses, chain_spec.clone(), all),
                Err(StatelessValidationError::InvalidCompressionInfos)
            ));
        }
        assert!(matches!(
            run(&witnesses, chain_spec, vec![placeholder(0)]),
            Err(StatelessValidationError::InvalidCompressionInfos)
        ));
    }

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

#[cfg(all(test, feature = "scroll-compress-info"))]
mod host_tests {
    use super::*;
    use sbv_primitives::{
        Bytes,
        chainspec::{Chain, build_chain_spec_force_hardfork},
        hardforks::Hardfork,
        keccak256,
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
            "../../../testdata/scroll/feynman/534352-19604670.json"
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
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/13.json"),
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
            "../../../testdata/dogeos/next-message-index/6281971-19.json"
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
    fn corrupt_pre_state_root_retains_root_and_typed_source() {
        use std::error::Error;

        let mut witness: BlockWitness = serde_json::from_str(include_str!(
            "../../../testdata/dogeos/next-message-index/6281971-19.json"
        ))
        .unwrap();
        // Point the pre-state root at a witness node that is present but is not valid RLP. This
        // exercises `SparseState::new` itself; an absent root remains an unresolved lazy trie node
        // and only fails when it is later accessed.
        let malformed_root_node = Bytes::from_static(&[0xff]);
        let corrupt_root = keccak256(&malformed_root_node);
        witness.states.push(malformed_root_node);
        witness.prev_state_root = corrupt_root;
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Tsuki);

        let error = run_host(&[witness], chain_spec)
            .expect_err("an unrelated pre-state root must not be revealed by the witness");

        match &error {
            StatelessValidationError::SparseStateCreationFailed {
                pre_state_root,
                source,
            } => {
                assert_eq!(*pre_state_root, corrupt_root);
                assert!(!source.to_string().is_empty());
            }
            other => panic!("expected sparse-state creation failure, got {other:?}"),
        }
        assert!(
            error.source().is_some(),
            "typed RLP source must be retained"
        );
        assert!(error.to_string().contains(&format!("{corrupt_root}")));
    }

    fn tsuki_empty_queue_witnesses() -> [BlockWitness; 3] {
        [
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/13.json"),
        ]
        .map(|json| serde_json::from_str::<BlockWitness>(json).unwrap())
    }

    fn honest_compression_infos(witnesses: &[BlockWitness]) -> Vec<Vec<(U256, usize)>> {
        witnesses
            .iter()
            .map(BlockWitness::compression_infos)
            .collect()
    }

    /// Drop the last info (`delta < 0`) or append a copy of the last info (`delta > 0`).
    fn resize_infos(infos: &mut Vec<(U256, usize)>, delta: isize) {
        match delta {
            -1 => {
                infos.pop().expect("block must have a transaction");
            }
            0 => {}
            1 => {
                let extra = infos.last().copied().unwrap_or((U256::ZERO, 0));
                infos.push(extra);
            }
            _ => unreachable!(),
        }
    }

    /// One block with N transactions: N-1 and N+1 compression infos are rejected, N passes.
    #[rstest::rstest]
    #[case::short(-1)]
    #[case::exact(0)]
    #[case::long(1)]
    fn compression_infos_must_match_tx_count(#[case] delta: isize) {
        let witness: BlockWitness = serde_json::from_str(include_str!(
            "../../../testdata/dogeos/next-message-index/6281971-19.json"
        ))
        .unwrap();
        let txs = witness.transactions.len();
        assert!(txs > 0);
        let mut infos = honest_compression_infos(std::slice::from_ref(&witness));
        resize_infos(&mut infos[0], delta);
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Tsuki);

        let result = run(&[witness], chain_spec, infos);

        if delta == 0 {
            result.expect("matching compression infos must verify");
        } else {
            assert!(matches!(
                result,
                Err(StatelessValidationError::InvalidCompressionInfos)
            ));
        }
    }

    #[rstest::rstest]
    #[case::last_block_short(2, -1)]
    #[case::middle_block_long(1, 1)]
    #[case::first_block_short(0, -1)]
    fn compression_infos_mismatch_in_any_block_is_rejected(
        #[case] index: usize,
        #[case] delta: isize,
    ) {
        let witnesses = tsuki_empty_queue_witnesses();
        let mut infos = honest_compression_infos(&witnesses);
        resize_infos(&mut infos[index], delta);
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(6_281_971), Hardfork::Tsuki);

        assert!(matches!(
            run(&witnesses, chain_spec, infos),
            Err(StatelessValidationError::InvalidCompressionInfos)
        ));
    }

    /// A block-count mismatch is a typed error, not a `zip_eq` panic.
    #[rstest::rstest]
    #[case::too_few(2)]
    #[case::too_many(4)]
    fn compression_infos_block_count_mismatch_is_an_error(#[case] vectors: usize) {
        let witnesses = tsuki_empty_queue_witnesses();
        let mut infos = honest_compression_infos(&witnesses);
        infos.resize(vectors, Vec::new());
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(6_281_971), Hardfork::Tsuki);

        assert!(matches!(
            run(&witnesses, chain_spec, infos),
            Err(StatelessValidationError::InvalidCompressionInfos)
        ));
    }

    /// A block with no transactions takes an empty vector, and rejects a non-empty one.
    #[rstest::rstest]
    #[case::empty(0)]
    #[case::extra(1)]
    fn compression_infos_for_empty_block(#[case] delta: isize) {
        let witness: BlockWitness = serde_json::from_str(include_str!(
            "../../../testdata/scroll/feynman/534352-19604675.json"
        ))
        .unwrap();
        assert!(witness.transactions.is_empty());
        let mut infos = vec![Vec::new()];
        resize_infos(&mut infos[0], delta);
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Feynman);

        let result = run(&[witness], chain_spec, infos);

        if delta == 0 {
            result.expect("an empty block with no compression infos must verify");
        } else {
            assert!(matches!(
                result,
                Err(StatelessValidationError::InvalidCompressionInfos)
            ));
        }
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
