//! Scroll block witness verifier implementation.

use crate::{
    BlockWitness, EvmExecutor,
    database::{BaseFeeDatabase, WitnessDatabase},
    error::StatelessValidationError,
    witness::BlockWitnessChunkExt,
};
use itertools::Itertools;
use reth_primitives_traits::RecoveredBlock;
use sbv_primitives::{
    Address, B256, U256,
    alloy_primitives::map::B256Map,
    chainspec::ChainSpec,
    hardforks::DogeosHardforks,
    types::{
        Header,
        consensus::BlockHeader,
        evm::ScrollBaseFeeProvider,
        reth::{
            consensus::{Consensus, DogeosConsensus, FullConsensus},
            evm::execute::ProviderError,
            execution_types::BlockExecutionResult,
            primitives::{Block, Receipt},
        },
        revm::Bytecode,
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
/// Storage slot of the L2 base-fee overhead in the L2SystemConfig contract.
///
/// Mirrors the private constant the shared calculator reads; it exists here so the proof's
/// documentation and tests can name the slot the rule depends on.
pub const L2_BASE_FEE_OVERHEAD_SLOT: U256 = U256::from_limbs([101, 0, 0, 0]);

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
    parent: Option<&Header>,
    chain_spec: Arc<ChainSpec>,
) -> Result<VerifyResult, StatelessValidationError> {
    let compression_infos = witnesses
        .iter()
        .map(|block| block.compression_infos())
        .collect::<Vec<_>>();
    run(witnesses, parent, chain_spec, compression_infos)
}

/// Verify the block witness and return the gas used.
///
/// `parent` is the trusted header of the block before the first witness: the caller vouches for
/// it, typically because its hash is the previous proof endpoint. It anchors the chunk's first
/// block (parent link, block number and pre-state root) and supplies the parent inputs of that
/// block's base-fee rule. It is **required** once Tsuki is active at the first block's timestamp;
/// earlier forks keep their historical behaviour and may pass `None`.
pub fn run(
    witnesses: &[BlockWitness],
    parent: Option<&Header>,
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

    let first = &witnesses[0].header;

    if parent.is_none() && chain_spec.is_tsuki_active_at_timestamp(first.timestamp) {
        return Err(StatelessValidationError::MissingParentHeader {
            block_number: first.number,
        });
    }

    // The height is checked before the hash so that a chunk starting at the wrong block reports
    // its height rather than the hash mismatch that follows from it. The hash is what actually
    // binds the chunk to the trusted parent, and with it the parent's state root.
    if let Some(parent) = parent {
        if first.number != parent.number + 1 {
            return Err(StatelessValidationError::ParentNumberMismatch {
                got: first.number,
                expected: parent.number,
            });
        }
        let parent_hash = parent.hash_slow();
        if first.parent_hash != parent_hash {
            return Err(StatelessValidationError::ParentHashMismatch {
                got: first.parent_hash,
                expected: parent_hash,
            });
        }
        if witnesses[0].prev_state_root != parent.state_root {
            return Err(StatelessValidationError::ParentStateRootMismatch {
                got: witnesses[0].prev_state_root,
                expected: parent.state_root,
            });
        }
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

    // The first block's base fee comes from the trusted parent; every later block's comes from
    // its predecessor, whose link to it was checked above.
    let mut block_parent = parent;

    for (block, compression_infos) in blocks.iter().zip_eq(compression_infos) {
        // Transactions root, ommers, withdrawals and L1 message ordering: body commitments that
        // replaying transactions and checking state roots does not cover.
        validate_body_pre_execution(block)?;

        if chain_spec.is_tsuki_active_at_timestamp(block.timestamp()) {
            let block_parent =
                block_parent.ok_or(StatelessValidationError::MissingParentHeader {
                    block_number: block.number,
                })?;
            check_base_fee(&chain_spec, &trie, &bytecode, block_parent, block)?;
        }

        let db = WitnessDatabase::new(&trie, &bytecode);

        let executor = EvmExecutor::new(chain_spec.clone(), db, block, Some(compression_infos));

        let output = executor.execute().map_err(|source| {
            StatelessValidationError::StatelessExecutionFailed {
                block_number: block.number,
                source,
            }
        })?;
        if block.gas_used != output.gas_used {
            return Err(StatelessValidationError::GasUsedMismatch {
                block_number: block.number,
                got: block.gas_used,
                expected: output.gas_used,
            });
        }
        gas_used += output.gas_used;

        validate_body_post_execution(block, &output.result)?;
        block_parent = Some(block.header());

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

/// Checks that a block header's base fee is the one the protocol rule derives.
///
/// The overhead comes from slot [`L2_BASE_FEE_OVERHEAD_SLOT`] of the L2SystemConfig contract named
/// by the chain's own `ScrollChainConfig`, read from this block's **pre-state**. A transaction that
/// changes the overhead therefore only moves the base fee from the next block onwards, including
/// when that next block is the first block of the next chunk.
///
/// The read goes through [`BaseFeeDatabase`], not the raw trie, so the account is loaded before the
/// slot and the sparse storage trie initialises. A witness that does not resolve the account or the
/// slot fails the proof; only an *authenticated* absence falls back to the default overhead.
fn check_base_fee(
    chain_spec: &Arc<ChainSpec>,
    trie: &SparseState,
    bytecode: &B256Map<Bytecode>,
    parent: &Header,
    block: &RecoveredBlock<Block>,
) -> Result<(), StatelessValidationError> {
    let mut db = BaseFeeDatabase(WitnessDatabase::new(trie, bytecode));
    let expected = ScrollBaseFeeProvider::new(chain_spec.clone())
        .next_block_base_fee(&mut db, parent, block.timestamp())
        .map_err(
            |source| StatelessValidationError::BaseFeeOverheadReadFailed {
                block_number: block.number,
                source,
            },
        )?;
    // `DogeosConsensus::validate_header` rejects a header without a base fee, and every Feynman+
    // header carries one; treat an absent base fee as zero so a malformed witness mismatches
    // instead of panicking.
    let got = block.base_fee_per_gas().unwrap_or_default();
    if got != expected {
        return Err(StatelessValidationError::BaseFeeMismatch {
            block_number: block.number,
            got,
            expected,
        });
    }
    Ok(())
}

/// Runs the shared DogeOS pre-execution body checks: transactions root, ommers, withdrawals and
/// L1 message ordering.
fn validate_body_pre_execution(
    block: &RecoveredBlock<Block>,
) -> Result<(), StatelessValidationError> {
    DogeosConsensus
        .validate_block_pre_execution(block.sealed_block())
        .map_err(|source| StatelessValidationError::BodyValidationFailed {
            block_number: block.number,
            source,
        })
}

/// Runs the shared DogeOS post-execution body checks: gas used, receipts root and logs bloom.
///
/// Executing the transactions and checking the state root does not cover these: a witness could
/// commit to a body whose receipts hash differently from the ones the block actually produced,
/// and under header-anchored endpoints the block hash the proof publishes covers both roots.
fn validate_body_post_execution(
    block: &RecoveredBlock<Block>,
    result: &BlockExecutionResult<Receipt>,
) -> Result<(), StatelessValidationError> {
    DogeosConsensus
        .validate_block_post_execution(block, result, None)
        .map_err(|source| StatelessValidationError::BodyValidationFailed {
            block_number: block.number,
            source,
        })
}

/// Vectors for the L2 base-fee rule, read through the sparse witness state.
///
/// These mirror the vectors in dogeos-reth's own `base_fee.rs` so the guest and the execution
/// client are known to agree on the same rule, and add the cases that are specific to proving it
/// from a witness: an authenticated empty slot, an authenticated absent account, and a witness
/// that does not resolve the account at all.
#[cfg(test)]
mod base_fee_rule_tests {
    use super::*;
    use sbv_primitives::{
        alloy_primitives::map::B256Map,
        chainspec::{
            Chain, EthChainSpec, build_chain_spec_force_hardfork,
            scroll::{DOGEOS_CHIKYU, DOGEOS_MAINNET},
        },
        hardforks::Hardfork,
        keccak256,
        types::{
            consensus::Header,
            evm::{DEFAULT_BASE_FEE_OVERHEAD, MAX_L2_BASE_FEE},
            rpc::ExecutionWitness,
        },
    };
    use sbv_trie::{EMPTY_ROOT_HASH, HashedPostState, HashedStorage};

    /// Mainnet-scale block gas limit.
    const GAS_LIMIT: u64 = 30_000_000;
    /// The 420 gwei overhead planned for activation.
    const OVERHEAD: u64 = 420_000_000_000;
    /// An address that is not the L2SystemConfig contract of any chain.
    const OTHER_ACCOUNT: Address =
        sbv_primitives::address!("00000000000000000000000000000000000000ff");

    /// An empty sparse state whose root is a node the witness does not carry, so any access to it
    /// is an unresolved node access.
    fn unresolved_state() -> SparseState {
        let root = sbv_primitives::b256!(
            "1111111111111111111111111111111111111111111111111111111111111111"
        );
        SparseState::new(&ExecutionWitness::default(), root)
            .expect("an unresolved root node is not a decoding failure")
            .0
    }

    /// A sparse state holding exactly the given accounts, each with the given storage.
    ///
    /// Building it from an empty trie and committing a post state leaves every other key
    /// *authenticated* absent rather than unresolved, which is what distinguishes "the contract
    /// is not deployed" from "the witness does not cover the contract".
    fn state_with(accounts: &[(Address, &[(U256, U256)])]) -> SparseState {
        let (mut trie, _) =
            SparseState::new(&ExecutionWitness::default(), EMPTY_ROOT_HASH).expect("an empty trie");
        let mut post = HashedPostState::default();
        for (address, slots) in accounts {
            let hashed = keccak256(address);
            post.accounts.insert(
                hashed,
                Some(reth_primitives_traits::Account {
                    nonce: 1,
                    balance: U256::ZERO,
                    bytecode_hash: None,
                }),
            );
            post.storages.insert(
                hashed,
                HashedStorage::from_iter(
                    false,
                    slots
                        .iter()
                        .map(|(slot, value)| (keccak256(B256::from(*slot)), *value)),
                ),
            );
        }
        trie.calculate_state_root(post);
        trie
    }

    fn parent(base_fee: u64, gas_used: u64) -> Header {
        Header {
            base_fee_per_gas: Some(base_fee),
            gas_limit: GAS_LIMIT,
            gas_used,
            timestamp: 1,
            difficulty: U256::from(1),
            ..Default::default()
        }
    }

    /// The base fee the rule derives for a child of `parent`, reading the overhead from `state`.
    fn next_base_fee(
        chain_spec: &Arc<ChainSpec>,
        state: &SparseState,
        parent: &Header,
    ) -> Result<u64, ProviderError> {
        let bytecode = B256Map::default();
        let mut db = BaseFeeDatabase(WitnessDatabase::new(state, &bytecode));
        ScrollBaseFeeProvider::new(chain_spec.clone()).next_block_base_fee(
            &mut db,
            parent,
            parent.timestamp + 1,
        )
    }

    fn mainnet_tsuki() -> Arc<ChainSpec> {
        build_chain_spec_force_hardfork(DOGEOS_MAINNET.chain, Hardfork::Tsuki)
    }

    fn system_config(chain_spec: &ChainSpec) -> Address {
        chain_spec.config.l1_config.l2_system_config_address
    }

    /// The forced-hardfork spec must name the proven chain's own L2SystemConfig contract, or the
    /// guest would read the overhead from the wrong account.
    #[test]
    fn the_system_config_address_follows_the_proven_chain() {
        for (chain, expected) in [
            (
                DOGEOS_MAINNET.chain,
                DOGEOS_MAINNET.config.l1_config.l2_system_config_address,
            ),
            (
                DOGEOS_CHIKYU.chain,
                DOGEOS_CHIKYU.config.l1_config.l2_system_config_address,
            ),
        ] {
            let spec = build_chain_spec_force_hardfork(chain, Hardfork::Tsuki);
            assert_eq!(system_config(&spec), expected, "chain {chain}");
            assert_ne!(expected, Address::ZERO, "chain {chain}");
        }
        // An unregistered chain id keeps the development blueprint, whose L2SystemConfig address
        // is the zero address - the same account the development node itself reads.
        let spec = build_chain_spec_force_hardfork(Chain::from_id(424_242), Hardfork::Tsuki);
        assert_eq!(system_config(&spec), Address::ZERO);
    }

    /// Mainnet proves with Tsuki's D48/E10 parameters, earlier forks with Feynman's D8/E2.
    #[test]
    fn the_eip1559_parameters_follow_the_proven_fork() {
        use sbv_primitives::chainspec::BaseFeeParams;

        assert_eq!(
            mainnet_tsuki().base_fee_params_at_timestamp(1),
            BaseFeeParams::new(48, 10)
        );
        assert_eq!(
            build_chain_spec_force_hardfork(DOGEOS_MAINNET.chain, Hardfork::Feynman)
                .base_fee_params_at_timestamp(1),
            BaseFeeParams::new(8, 2)
        );
    }

    /// An authenticated empty slot is "the overhead was never configured": the rule's default
    /// applies, and at the gas target the fee is unchanged.
    #[test]
    fn an_authenticated_empty_slot_uses_the_default_overhead() {
        let chain_spec = mainnet_tsuki();
        let state = state_with(&[(system_config(&chain_spec), &[])]);
        let target = GAS_LIMIT / 10;

        assert_eq!(
            next_base_fee(&chain_spec, &state, &parent(1_000_000_000, target)).unwrap(),
            1_000_000_000
        );
        // The default is the overhead the whole fee sits on: an empty block decays only the part
        // above it.
        let empty = next_base_fee(&chain_spec, &state, &parent(1_000_000_000, 0)).unwrap();
        let excess = 1_000_000_000 - DEFAULT_BASE_FEE_OVERHEAD.saturating_to::<u64>();
        assert_eq!(
            empty,
            excess - excess / 48 + DEFAULT_BASE_FEE_OVERHEAD.saturating_to::<u64>()
        );
    }

    /// An authenticated absent account is also "never configured". The sparse storage trie of an
    /// account that does not exist cannot be built, so this is the case the base-fee database has
    /// to handle itself.
    #[test]
    fn an_authenticated_absent_account_uses_the_default_overhead() {
        let chain_spec = mainnet_tsuki();
        let state = state_with(&[(OTHER_ACCOUNT, &[(U256::from(1), U256::from(1))])]);
        assert!(
            state
                .account(system_config(&chain_spec))
                .expect("the absence is authenticated")
                .is_none()
        );

        assert_eq!(
            next_base_fee(&chain_spec, &state, &parent(1_000_000_000, GAS_LIMIT / 10)).unwrap(),
            1_000_000_000
        );
    }

    /// A witness that does not resolve the account must fail the proof, never silently take the
    /// default. The sparse trie refuses unresolved node access by aborting.
    #[test]
    #[should_panic(expected = "MPT: Unresolved node access")]
    fn an_unresolved_account_node_fails_the_proof() {
        let chain_spec = mainnet_tsuki();
        let _ = next_base_fee(
            &chain_spec,
            &unresolved_state(),
            &parent(1_000_000_000, GAS_LIMIT / 10),
        );
    }

    /// The configured overhead is read from the pre-state, and the escalation applies only to the
    /// part of the fee above it.
    #[test]
    fn the_configured_overhead_is_read_from_the_pre_state() {
        let chain_spec = mainnet_tsuki();
        let state = state_with(&[(
            system_config(&chain_spec),
            &[(L2_BASE_FEE_OVERHEAD_SLOT, U256::from(OVERHEAD))],
        )]);
        let target = GAS_LIMIT / 10;

        // 2x target: the part above the overhead grows by 1/48.
        assert_eq!(
            next_base_fee(
                &chain_spec,
                &state,
                &parent(OVERHEAD + 1_000_000_000, 2 * target)
            )
            .unwrap(),
            OVERHEAD + 1_020_833_333
        );
        // Full block (10x target): it grows by 9/48.
        assert_eq!(
            next_base_fee(
                &chain_spec,
                &state,
                &parent(OVERHEAD + 1_000_000_000, GAS_LIMIT)
            )
            .unwrap(),
            OVERHEAD + 1_187_500_000
        );
        // Empty block: it shrinks by 1/48, down to the idle fixed point of overhead + 47 wei.
        assert_eq!(
            next_base_fee(&chain_spec, &state, &parent(OVERHEAD + 48, 0)).unwrap(),
            OVERHEAD + 47
        );
        assert_eq!(
            next_base_fee(&chain_spec, &state, &parent(OVERHEAD + 47, 0)).unwrap(),
            OVERHEAD + 47
        );
    }

    /// The protocol cap applies after the overhead is added back.
    #[test]
    fn the_base_fee_is_capped() {
        let chain_spec = mainnet_tsuki();
        let state = state_with(&[(
            system_config(&chain_spec),
            &[(L2_BASE_FEE_OVERHEAD_SLOT, U256::from(OVERHEAD))],
        )]);

        for parent_base_fee in [MAX_L2_BASE_FEE, MAX_L2_BASE_FEE - 1] {
            assert_eq!(
                next_base_fee(&chain_spec, &state, &parent(parent_base_fee, GAS_LIMIT)).unwrap(),
                MAX_L2_BASE_FEE
            );
        }
    }

    /// An overhead that does not fit a `u64`, or one above the parent's base fee, saturates
    /// instead of wrapping or aborting: the fee becomes the capped overhead.
    #[test]
    fn an_out_of_range_overhead_saturates() {
        let chain_spec = mainnet_tsuki();
        let huge = state_with(&[(
            system_config(&chain_spec),
            &[(L2_BASE_FEE_OVERHEAD_SLOT, U256::MAX)],
        )]);
        assert_eq!(
            next_base_fee(&chain_spec, &huge, &parent(1_000_000_000, GAS_LIMIT)).unwrap(),
            MAX_L2_BASE_FEE
        );

        // An overhead above the parent's base fee leaves nothing to escalate, so only EIP-1559's
        // minimum increase of one wei lands on top of the overhead.
        let above = state_with(&[(
            system_config(&chain_spec),
            &[(L2_BASE_FEE_OVERHEAD_SLOT, U256::from(OVERHEAD))],
        )]);
        assert_eq!(
            next_base_fee(&chain_spec, &above, &parent(1, GAS_LIMIT)).unwrap(),
            OVERHEAD + 1
        );
    }

    /// The base fees the two recorded Tsuki lineages actually carry follow the rule exactly, with
    /// the default overhead and the Feynman fee parameters their producing node used.
    ///
    /// This pins the rule against real chain data, and records why those witnesses cannot be
    /// replayed through a Tsuki `run`: their node predates the D48/E10 parameters, and they carry
    /// no proof for the L2SystemConfig account.
    #[test]
    fn the_recorded_lineages_follow_the_rule() {
        let lineage: [BlockWitness; 3] = [
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/13.json"),
        ]
        .map(|json| serde_json::from_str::<BlockWitness>(json).unwrap());

        // The producing node applied the Feynman fee parameters, so force that fork.
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(lineage[0].chain_id), Hardfork::Feynman);
        let state = state_with(&[(system_config(&chain_spec), &[])]);

        for pair in lineage.windows(2) {
            let (parent, child) = (&pair[0].header, &pair[1].header);
            let bytecode = B256Map::default();
            let mut db = BaseFeeDatabase(WitnessDatabase::new(&state, &bytecode));
            let expected = ScrollBaseFeeProvider::new(chain_spec.clone())
                .next_block_base_fee(&mut db, parent, child.timestamp)
                .unwrap();
            assert_eq!(
                child.base_fee_per_gas,
                Some(expected),
                "block {}",
                child.number
            );
        }
    }
}

/// The trusted parent header anchors a chunk, and the body commitments and gas accounting of
/// every block are checked against what the block actually executed.
#[cfg(test)]
#[cfg(feature = "scroll-compress-info")]
mod anchor_tests {
    use super::*;
    use sbv_primitives::{
        Bloom,
        chainspec::{Chain, build_chain_spec_force_hardfork},
        hardforks::Hardfork,
        types::consensus::Header,
    };

    /// Five consecutive mainnet blocks, from a fork whose chunks need no trusted parent, so the
    /// anchoring and body rules can be exercised against real chain data.
    fn consecutive_blocks() -> Vec<BlockWitness> {
        [
            include_str!("../../../testdata/scroll/feynman/534352-19604670.json"),
            include_str!("../../../testdata/scroll/feynman/534352-19604671.json"),
            include_str!("../../../testdata/scroll/feynman/534352-19604672.json"),
            include_str!("../../../testdata/scroll/feynman/534352-19604673.json"),
            include_str!("../../../testdata/scroll/feynman/534352-19604674.json"),
        ]
        .into_iter()
        .map(|json| serde_json::from_str::<BlockWitness>(json).unwrap())
        .collect()
    }

    /// The Tsuki lineage, whose first witness can serve as the trusted parent of the rest.
    fn tsuki_lineage() -> Vec<BlockWitness> {
        [
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/13.json"),
        ]
        .into_iter()
        .map(|json| serde_json::from_str::<BlockWitness>(json).unwrap())
        .collect()
    }

    fn feynman_spec(witness: &BlockWitness) -> Arc<ChainSpec> {
        build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Feynman)
    }

    fn tsuki_spec(witness: &BlockWitness) -> Arc<ChainSpec> {
        build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Tsuki)
    }

    /// A chunk anchored to its real parent verifies, and the endpoint the proof would publish is
    /// the hash of its last block.
    #[test]
    fn a_chunk_anchored_to_its_parent_verifies() {
        let blocks = consecutive_blocks();
        let (parent, chunk) = blocks.split_first().unwrap();
        let spec = feynman_spec(parent);

        let result = run_host(chunk, Some(&parent.header), spec).unwrap();

        assert_eq!(result.pre_state_root, parent.header.state_root);
        assert_eq!(
            result.blocks.last().unwrap().hash(),
            chunk.last().unwrap().header.hash_slow()
        );
    }

    /// Two chunks of the same chain meet at one block: the hash the first chunk ends on is the
    /// hash the second chunk opens its parent header with.
    #[test]
    fn consecutive_chunks_meet_at_the_boundary_block() {
        let blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);

        let first = run_host(&blocks[1..3], Some(&blocks[0].header), spec.clone()).unwrap();
        let boundary = &blocks[2].header;
        let second = run_host(&blocks[3..5], Some(boundary), spec).unwrap();

        assert_eq!(first.blocks.last().unwrap().hash(), boundary.hash_slow());
        assert_eq!(second.blocks[0].header().parent_hash, boundary.hash_slow());
        assert_eq!(second.pre_state_root, boundary.state_root);
    }

    /// A Tsuki chunk without a trusted parent header is rejected: the rule that binds it to the
    /// previous endpoint, and its first block's base fee, both depend on it.
    #[test]
    fn a_tsuki_chunk_without_a_parent_header_is_rejected() {
        let blocks = tsuki_lineage();
        let spec = tsuki_spec(&blocks[0]);

        assert!(matches!(
            run_host(&blocks, None, spec),
            Err(StatelessValidationError::MissingParentHeader { block_number: 11 })
        ));
    }

    /// Earlier forks keep their historical behaviour and verify without a parent.
    #[test]
    fn a_legacy_chunk_without_a_parent_header_is_accepted() {
        let blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);

        run_host(&blocks[1..3], None, spec).expect("a Feynman chunk needs no trusted parent");
    }

    /// A chunk that does not start at the block after the parent is rejected on its height.
    #[test]
    fn a_chunk_that_skips_a_block_is_rejected() {
        let blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);

        assert!(matches!(
            run_host(&blocks[2..4], Some(&blocks[0].header), spec),
            Err(StatelessValidationError::ParentNumberMismatch {
                got: 19_604_672,
                expected: 19_604_670,
            })
        ));
    }

    /// A chunk that repeats the parent's own height is rejected the same way.
    #[test]
    fn a_chunk_that_repeats_the_parent_block_is_rejected() {
        let blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);

        assert!(matches!(
            run_host(&blocks[0..2], Some(&blocks[0].header), spec),
            Err(StatelessValidationError::ParentNumberMismatch {
                got: 19_604_670,
                expected: 19_604_670,
            })
        ));
    }

    /// A parent header at the right height but with any other difference does not hash to the
    /// chunk's parent hash, so the chunk is not anchored to it.
    #[test]
    fn a_substituted_parent_header_is_rejected() {
        let blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);
        let mut parent = blocks[0].header.clone();
        let real = parent.hash_slow();
        parent.nonce = [1; 8].into();
        assert_ne!(parent.hash_slow(), real);

        assert!(matches!(
            run_host(&blocks[1..3], Some(&parent), spec),
            Err(StatelessValidationError::ParentHashMismatch { expected, .. })
                if expected == parent.hash_slow()
        ));
    }

    /// The witness's claimed pre-state root has to be the trusted parent's state root; execution
    /// would otherwise start from a state the parent never committed to.
    #[test]
    fn a_pre_state_root_that_is_not_the_parents_is_rejected() {
        let mut blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);
        let parent = blocks[0].header.clone();
        blocks[1].prev_state_root = B256::repeat_byte(9);

        assert!(matches!(
            run_host(&blocks[1..2], Some(&parent), spec),
            Err(StatelessValidationError::ParentStateRootMismatch {
                got,
                expected,
            }) if got == B256::repeat_byte(9) && expected == parent.state_root
        ));
    }

    /// The recorded Tsuki lineage is anchored correctly but carries no proof for the
    /// L2SystemConfig account, so the base-fee rule cannot be evaluated and the proof fails
    /// rather than falling back to the default overhead.
    ///
    /// Replacing this corpus with witnesses from a node that writes slot 101 into the execution
    /// witness is what un-ignores the Tsuki `run` tests.
    #[test]
    #[should_panic(expected = "MPT: Unresolved node access")]
    fn a_tsuki_chunk_without_an_l2_system_config_proof_fails_the_proof() {
        let blocks = tsuki_lineage();
        let spec = tsuki_spec(&blocks[0]);

        let _ = run_host(&blocks[1..3], Some(&blocks[0].header), spec);
    }

    /// A header that claims gas the block did not spend is rejected. Replaying transactions and
    /// checking the state root does not cover this, and the block hash the proof publishes
    /// commits to the claim.
    #[test]
    fn a_header_that_misreports_gas_used_is_rejected() {
        let mut blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);
        let honest = blocks[1].header.gas_used;
        blocks[1].header.gas_used = honest + 1;

        assert!(matches!(
            run_host(&blocks[1..2], Some(&blocks[0].header.clone()), spec),
            Err(StatelessValidationError::GasUsedMismatch {
                got,
                expected,
                ..
            }) if got == honest + 1 && expected == honest
        ));
    }

    /// The body commitments in the header have to match the body and the receipts the block
    /// produced: transactions root, receipts root and logs bloom.
    #[rstest::rstest]
    #[case::transactions_root(|h: &mut Header| h.transactions_root = B256::repeat_byte(7))]
    #[case::receipts_root(|h: &mut Header| h.receipts_root = B256::repeat_byte(7))]
    #[case::logs_bloom(|h: &mut Header| h.logs_bloom = Bloom::repeat_byte(7))]
    fn a_header_that_misreports_its_body_is_rejected(#[case] mutate: fn(&mut Header)) {
        let mut blocks = consecutive_blocks();
        let spec = feynman_spec(&blocks[0]);
        let parent = blocks[0].header.clone();
        mutate(&mut blocks[1].header);

        assert!(matches!(
            run_host(&blocks[1..2], Some(&parent), spec),
            Err(StatelessValidationError::BodyValidationFailed { block_number, .. })
                if block_number == 19_604_671
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
        let result = run_host(&[witness], None, chain_spec).unwrap();
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

        let result = run_host(&[witness], None, chain_spec).unwrap();

        assert_eq!(result.next_message_index, 0);
    }

    /// Real DogeOS witnesses from a Tsuki materializer run must replay as a contiguous chunk.
    ///
    /// These blocks contain no L1 messages, so the queue index remains zero. The forced Tsuki
    /// chain spec is intentional: the local fixture predates the public Chikyū activation time,
    /// but was produced by a Tsuki-enabled development network.
    #[test]
    #[ignore = "the Tsuki corpus carries no L2SystemConfig proof: see the base-fee rule gate"]
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
        let result = run_host(&witnesses, None, chain_spec).unwrap();

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
    #[ignore = "the Tsuki corpus carries no L2SystemConfig proof: see the base-fee rule gate"]
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

        let result = run_host(&[witness], None, chain_spec).unwrap();
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
        // The fork only decides which post-execution reads happen; sparse-state construction
        // fails first either way. Feynman keeps the witness out of the Tsuki parent-header and
        // base-fee rules, which are covered on their own below.
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Feynman);

        let error = run_host(&[witness], None, chain_spec)
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

    fn honestcompression_infos(witnesses: &[BlockWitness]) -> Vec<Vec<(U256, usize)>> {
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
        let mut infos = honestcompression_infos(std::slice::from_ref(&witness));
        resize_infos(&mut infos[0], delta);
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Tsuki);

        let result = run(&[witness], None, chain_spec, infos);

        if delta == 0 {
            // A Tsuki witness from the current corpus cannot satisfy the base-fee rule, so this
            // case asserts only that matching counts get past the compression-info check.
            assert!(!matches!(
                result,
                Err(StatelessValidationError::InvalidCompressionInfos)
            ));
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
        let mut infos = honestcompression_infos(&witnesses);
        resize_infos(&mut infos[index], delta);
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(6_281_971), Hardfork::Tsuki);

        assert!(matches!(
            run(&witnesses, None, chain_spec, infos),
            Err(StatelessValidationError::InvalidCompressionInfos)
        ));
    }

    /// A block-count mismatch is a typed error, not a `zip_eq` panic.
    #[rstest::rstest]
    #[case::too_few(2)]
    #[case::too_many(4)]
    fn compression_infos_block_count_mismatch_is_an_error(#[case] vectors: usize) {
        let witnesses = tsuki_empty_queue_witnesses();
        let mut infos = honestcompression_infos(&witnesses);
        infos.resize(vectors, Vec::new());
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(6_281_971), Hardfork::Tsuki);

        assert!(matches!(
            run(&witnesses, None, chain_spec, infos),
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

        let result = run(&[witness], None, chain_spec, infos);

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
                run(&witnesses, None, chain_spec.clone(), all),
                Err(StatelessValidationError::InvalidCompressionInfos)
            ));
        }
        assert!(matches!(
            run(&witnesses, None, chain_spec, vec![placeholder(0)]),
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
