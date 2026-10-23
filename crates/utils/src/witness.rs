//! Witness builder.

use sbv_core::witness::BlockWitness;
use sbv_primitives::{
    B256, ChainId,
    types::rpc::{Block as RpcBlock, ExecutionWitness},
};

/// Block witness builder.
#[derive(Debug, Default)]
pub struct WitnessBuilder {
    pub(crate) chain_id: Option<ChainId>,
    pub(crate) block: Option<RpcBlock>,
    pub(crate) execution_witness: Option<ExecutionWitness>,
    pub(crate) prev_state_root: Option<B256>,
}

/// Witness build error.
#[derive(Debug, thiserror::Error)]
pub enum WitnessBuildError {
    /// Missing field.
    #[error("missing field: {0}")]
    MissingField(&'static str),
}

impl WitnessBuilder {
    /// Create a new witness builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the chain ID.
    pub fn chain_id(mut self, chain_id: ChainId) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Set the block.
    pub fn block(mut self, block: RpcBlock) -> Self {
        self.block = Some(block);
        self
    }

    /// Set the execution witness
    pub fn execution_witness(mut self, execution_witness: ExecutionWitness) -> Self {
        self.execution_witness = Some(execution_witness);
        self
    }

    /// Set the previous state root.
    pub fn prev_state_root(mut self, prev_state_root: B256) -> Self {
        self.prev_state_root = Some(prev_state_root);
        self
    }

    /// Build the block witness.
    pub fn build(self) -> Result<BlockWitness, WitnessBuildError> {
        let block = self.block.ok_or(WitnessBuildError::MissingField("block"))?;
        let execution_witness = self
            .execution_witness
            .ok_or(WitnessBuildError::MissingField("execution_witness"))?;
        Ok(BlockWitness {
            chain_id: self
                .chain_id
                .ok_or(WitnessBuildError::MissingField("chain_id"))?,
            header: block.header.into(),
            prev_state_root: self
                .prev_state_root
                .ok_or(WitnessBuildError::MissingField("prev_state_root"))?,
            transactions: block
                .transactions
                .into_transactions()
                .map(|tx| tx.inner.into_inner())
                .collect(),
            withdrawals: block.withdrawals,
            states: execution_witness.state,
            codes: execution_witness.codes,
        })
    }
}
