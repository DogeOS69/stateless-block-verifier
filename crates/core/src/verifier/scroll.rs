use crate::{
    BlockWitness,
    verifier::{VerifyResult, run},
};
use reth_stateless::{StatelessTrie, validation::StatelessValidationError};
use sbv_primitives::{
    Address, B256, U256, chainspec::ChainSpec, types::reth::evm::execute::ProviderError,
};
use sbv_trie::SparseState;
use std::{
    any::Any,
    io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{Arc, Mutex},
};

/// L2MessageQueue pre-deployed address.
pub const L2_MESSAGE_QUEUE: Address =
    sbv_primitives::address!("5300000000000000000000000000000000000000");
/// Storage slot of messageRoot in L2MessageQueue.
pub const WITHDRAW_TRIE_ROOT_SLOT: U256 = U256::ZERO;
/// Storage slot of nextMessageIndex in L2MessageQueue (inherited from AppendOnlyMerkleTree).
pub const NEXT_MESSAGE_INDEX_SLOT: U256 = U256::from_limbs([1, 0, 0, 0]);

static SILENCED_PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

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

fn next_message_index_from_value(next_message_index: U256) -> Result<u64, ProviderError> {
    u64::try_from(next_message_index).map_err(|_| {
        ProviderError::other(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("nextMessageIndex does not fit into u64: {next_message_index}"),
        ))
    })
}

/// Get the Scroll L2 message queue outputs committed after block execution.
///
/// Note: `withdraw_root` here should not be confused with the withdrawal root of the beacon
/// chain.
pub(super) fn l2_message_queue_info(state: &SparseState) -> Result<(B256, u64), ProviderError> {
    match catch_unwind_silenced(|| {
        ensure_l2_message_queue_account(state)?;
        let withdraw_root = state.storage(L2_MESSAGE_QUEUE, WITHDRAW_TRIE_ROOT_SLOT)?;
        let next_message_index = state.storage(L2_MESSAGE_QUEUE, NEXT_MESSAGE_INDEX_SLOT)?;
        Ok((
            withdraw_root.into(),
            next_message_index_from_value(next_message_index)?,
        ))
    }) {
        Ok(result) => result,
        Err(panic) => {
            let detail = panic_message(panic.as_ref()).unwrap_or("unknown panic");
            if detail.contains("Unresolved node access") {
                Err(incomplete_queue_proof_error(detail))
            } else {
                resume_unwind(panic)
            }
        }
    }
}

fn ensure_l2_message_queue_account(state: &SparseState) -> Result<(), ProviderError> {
    if state.account(L2_MESSAGE_QUEUE)?.is_none() {
        return Err(ProviderError::other(io::Error::new(
            io::ErrorKind::NotFound,
            format!("L2MessageQueue contract not found at {L2_MESSAGE_QUEUE}"),
        )));
    }
    // Rebuild from the current storage root so post-execution reads can use proof nodes appended
    // for the block's final queue state, even if execution touched other queue slots first.
    state.refresh_storage_trie(L2_MESSAGE_QUEUE)?;
    Ok(())
}

fn panic_message(panic: &(dyn Any + Send)) -> Option<&str> {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
}

fn incomplete_queue_proof_error(detail: &str) -> ProviderError {
    ProviderError::other(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "incomplete Scroll witness: missing L2MessageQueue proof nodes for slots 0x0 and 0x1; witnesses must include queue proofs ({detail})"
        ),
    ))
}

fn catch_unwind_silenced<F, T>(f: F) -> std::thread::Result<T>
where
    F: FnOnce() -> T,
{
    let _lock = SILENCED_PANIC_HOOK_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(hook);
    result
}

#[cfg(test)]
#[cfg(feature = "scroll-compress-info")]
mod tests {
    use super::*;
    use sbv_primitives::{
        b256,
        chainspec::{Chain, build_chain_spec_force_hardfork, get_chain_spec},
        hardforks::Hardfork,
    };
    use std::{fs, path::Path};

    fn read_witness(rel_path: &str) -> BlockWitness {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel_path);
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

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
        let witness = read_witness("../../testdata/dogeos/next-message-index/20240125.json");
        let chain_spec = get_chain_spec(Chain::from_id(witness.chain_id)).unwrap();

        let result = run_host(&[witness], chain_spec).unwrap();

        assert_eq!(result.next_message_index, 208530);
    }

    #[test]
    fn test_galileo_v2_raw_fixture_requires_queue_proofs() {
        let witness = read_witness("../../testdata/dogeos/queue-read/raw-20239240.json");
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::GalileoV2);

        let err = run_host(&[witness], chain_spec)
            .expect_err("raw prover fixture should fail without queue proofs");
        let StatelessValidationError::StatelessExecutionFailed(msg) = err else {
            panic!("unexpected error: {err:?}");
        };

        assert!(msg.contains("missing L2MessageQueue proof nodes"));
        assert!(msg.contains("witnesses must include queue proofs"));
    }

    #[test]
    fn test_galileo_v2_live_mainnet_fixture() {
        let witness = read_witness("../../testdata/dogeos/galileov2-mainnet/32144474.json");
        let chain_spec =
            build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::GalileoV2);

        let result = run_host(&[witness], chain_spec).unwrap();

        assert_eq!(
            result.withdraw_root,
            b256!("9d42c00e8305f065d4e8df4d073cd9424986b0dcc63becf39b718d9f61f99179")
        );
        assert_eq!(result.next_message_index, 221555);
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
