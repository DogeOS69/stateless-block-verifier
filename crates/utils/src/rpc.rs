//! Rpc Extension

use crate::witness::WitnessBuilder;
use alloy_provider::Provider;
use alloy_transport::TransportResult;
#[cfg(feature = "scroll")]
use sbv_core::verifier::{L2_MESSAGE_QUEUE, NEXT_MESSAGE_INDEX_SLOT, WITHDRAW_TRIE_ROOT_SLOT};
use sbv_core::witness::BlockWitness;
#[cfg(feature = "scroll")]
use sbv_primitives::keccak256;
use sbv_primitives::{
    B256, BlockNumber, Bytes, ChainId,
    alloy_primitives::map::B256HashMap,
    types::{
        Network,
        eips::BlockNumberOrTag,
        rpc::{Block, ExecutionWitness},
    },
};
use serde::Deserialize;
#[cfg(feature = "scroll")]
use std::collections::HashSet;

/// Extension trait for [`Provider`](Provider).
#[async_trait::async_trait]
pub trait ProviderExt: Provider<Network> {
    /// Get the execution witness for a block.
    async fn debug_execution_witness(
        &self,
        number: BlockNumberOrTag,
    ) -> TransportResult<ExecutionWitness> {
        /// Represents the execution witness of a block. Contains an optional map of state preimages.
        #[derive(Debug, Deserialize)]
        struct GethExecutionWitness {
            pub state: B256HashMap<Bytes>,
            pub codes: B256HashMap<Bytes>,
        }

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum ExecutionWitnessDeHelper {
            Standard(ExecutionWitness),
            Geth(GethExecutionWitness),
        }

        self.client()
            .request::<_, ExecutionWitnessDeHelper>("debug_executionWitness", (number,))
            .await
            .map(|response| match response {
                ExecutionWitnessDeHelper::Standard(witness) => witness,
                ExecutionWitnessDeHelper::Geth(witness) => ExecutionWitness {
                    state: witness.state.into_values().collect(),
                    codes: witness.codes.into_values().collect(),
                    ..Default::default()
                },
            })
    }

    /// Dump the block witness for a block.
    ///
    /// # Panics
    ///
    /// This function will panic if the block number is 0.
    fn dump_block_witness(&self, number: BlockNumber) -> DumpBlockWitness<'_, Self>
    where
        Self: Sized,
    {
        assert_ne!(number, 0, "genesis block is not traceable");
        DumpBlockWitness::new(self, number)
    }

    /// Dump the ancestor blocks for a block.
    #[doc(hidden)]
    #[cfg(not(feature = "scroll"))]
    async fn dump_block_ancestors(
        &self,
        number: BlockNumber,
        ancestors: Option<usize>,
    ) -> TransportResult<Option<Vec<Block>>> {
        use std::future::IntoFuture;

        let ancestors = ancestors
            .unwrap_or_default()
            .clamp(1, (number as usize).min(256));

        let ancestors = futures::future::try_join_all((1..=ancestors).map(|offset| {
            let block_number = number - offset as BlockNumber;
            self.get_block_by_number(block_number.into()).into_future()
        }))
        .await?;

        if ancestors.iter().any(Option::is_none) {
            return Ok(None);
        }

        Ok(Some(ancestors.into_iter().map(Option::unwrap).collect()))
    }
}

impl<P: Provider<Network>> ProviderExt for P {}

#[cfg(feature = "scroll")]
fn extend_execution_witness_state<I>(execution_witness: &mut ExecutionWitness, nodes: I)
where
    I: IntoIterator<Item = Bytes>,
{
    let mut seen = execution_witness
        .state
        .iter()
        .map(keccak256)
        .collect::<HashSet<_>>();

    for node in nodes {
        if seen.insert(keccak256(&node)) {
            execution_witness.state.push(node);
        }
    }
}

#[cfg(feature = "scroll")]
async fn append_l2_message_queue_proofs<P: Provider<Network>>(
    provider: &P,
    number: BlockNumber,
    execution_witness: &mut ExecutionWitness,
) -> TransportResult<()> {
    let parent_number = number
        .checked_sub(1)
        .expect("dump_block_witness rejects genesis blocks");
    let storage_keys = vec![
        B256::from(WITHDRAW_TRIE_ROOT_SLOT),
        B256::from(NEXT_MESSAGE_INDEX_SLOT),
    ];

    for proof_block in [parent_number, number] {
        let proof = provider
            .get_proof(L2_MESSAGE_QUEUE, storage_keys.clone())
            // The witness executes from the parent root, but post-execution queue reads can still
            // require nodes from the block's final queue state if the contract was modified.
            .block_id(proof_block.into())
            .await?;

        extend_execution_witness_state(
            execution_witness,
            proof.account_proof.into_iter().chain(
                proof
                    .storage_proof
                    .into_iter()
                    .flat_map(|proof| proof.proof),
            ),
        );
    }

    Ok(())
}

/// DumpBlockWitness created via [`ProviderExt::dump_block_witness`].
#[must_use = "DumpBlockWitness does not execute until you call `send`"]
#[derive(Debug)]
pub struct DumpBlockWitness<'a, P> {
    provider: &'a P,
    number: BlockNumber,
    #[cfg(not(feature = "scroll"))]
    ancestors: Option<usize>,

    builder: WitnessBuilder,
}

impl<'a, P: ProviderExt> DumpBlockWitness<'a, P> {
    fn new(provider: &'a P, number: BlockNumber) -> Self {
        Self {
            provider,
            number,
            #[cfg(not(feature = "scroll"))]
            ancestors: None,

            builder: WitnessBuilder::default(),
        }
    }

    /// Set the builder
    pub fn builder(mut self, builder: WitnessBuilder) -> Self {
        self.builder = builder;
        self
    }

    /// Set the number of ancestors to include in the witness.
    #[cfg(not(feature = "scroll"))]
    pub fn ancestors(mut self, ancestors: usize) -> Self {
        self.ancestors = Some(ancestors);
        self
    }

    /// Set the block number to dump.
    ///
    /// # Panics
    ///
    /// This function will panic if the block number is 0.
    pub fn with_number(mut self, number: BlockNumber) -> Self {
        assert_ne!(number, 0, "genesis block is not traceable");
        self.number = number;
        self
    }

    /// Set the chain ID.
    pub fn with_chain_id(mut self, chain_id: ChainId) -> Self {
        self.builder = self.builder.chain_id(chain_id);
        self
    }

    /// Use cached block.
    ///
    /// # Panics
    ///
    /// This function will panic if the block number does not match the builder's block number.
    pub fn with_cached_block(mut self, block: Block) -> Self {
        assert_eq!(
            block.header.number, self.number,
            "block number does not match builder's block number"
        );

        self.builder = self.builder.block(block);
        self
    }

    /// Use cached previous block.
    ///
    /// # Panics
    ///
    /// This function will panic if the block number
    pub fn with_cached_prev_block(mut self, prev_block: &Block) -> Self {
        assert_eq!(
            prev_block.header.number,
            self.number.checked_sub(1).expect("block number underflow"),
            "block number does not match builder's block number"
        );

        self.builder = self.builder.prev_state_root(prev_block.header.state_root);
        self
    }

    /// Set the execution witness of current block.
    pub fn with_cached_execution_witness(mut self, execution_witness: ExecutionWitness) -> Self {
        self.builder = self.builder.execution_witness(execution_witness);
        self
    }

    /// Use cached ancestor blocks.
    #[cfg(not(feature = "scroll"))]
    pub fn with_cached_ancestor_blocks<I>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = Block>,
    {
        self.builder = self.builder.ancestor_blocks(iter);
        self
    }

    /// Set the previous state root.
    pub fn with_prev_state_root(mut self, prev_state_root: B256) -> Self {
        self.builder = self.builder.prev_state_root(prev_state_root);
        self
    }

    /// Send the request to dump the block witness.
    pub async fn send(mut self) -> TransportResult<Option<BlockWitness>> {
        if self.builder.chain_id.is_none() {
            self.builder = self.builder.chain_id(self.provider.get_chain_id().await?);
        }

        if self.builder.block.is_none() {
            let Some(block) = self
                .provider
                .get_block_by_number(self.number.into())
                .full()
                .await?
            else {
                return Ok(None);
            };
            self.builder = self.builder.block(block);
        }

        if self.builder.prev_state_root.is_none() {
            let block = self.builder.block.as_ref().unwrap();
            let parent_block = self
                .provider
                .get_block_by_hash(block.header.parent_hash)
                .await?
                .expect("parent block should exist");

            self.builder = self.builder.prev_state_root(parent_block.header.state_root);
        }

        if self.builder.execution_witness.is_none() {
            let execution_witness = self
                .provider
                .debug_execution_witness(self.number.into())
                .await?;
            self.builder = self.builder.execution_witness(execution_witness);
        }

        #[cfg(feature = "scroll")]
        {
            let mut execution_witness = self.builder.execution_witness.take().expect(
                "execution_witness must be populated before appending L2 message queue proofs",
            );
            append_l2_message_queue_proofs(self.provider, self.number, &mut execution_witness)
                .await?;
            self.builder = self.builder.execution_witness(execution_witness);
        }

        #[cfg(not(feature = "scroll"))]
        if self.builder.blocks_hash.is_none() {
            let ancestors = self
                .provider
                .dump_block_ancestors(self.number, self.ancestors)
                .await?
                .unwrap();

            self.builder = self.builder.ancestor_blocks(ancestors);
        }

        Ok(Some(self.builder.build().unwrap()))
    }
}

#[cfg(all(test, feature = "scroll"))]
mod tests {
    use super::*;

    #[test]
    fn extend_execution_witness_state_dedups_nodes() {
        let existing = Bytes::from_static(b"existing");
        let inserted = Bytes::from_static(b"inserted");
        let inserted_again = inserted.clone();
        let mut execution_witness = ExecutionWitness {
            state: vec![existing.clone()],
            ..Default::default()
        };

        extend_execution_witness_state(
            &mut execution_witness,
            vec![existing.clone(), inserted.clone(), inserted_again],
        );

        assert_eq!(execution_witness.state, vec![existing, inserted]);
    }
}
