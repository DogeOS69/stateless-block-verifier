use crate::{
    B256, Bytes, ChainId,
    legacy_types::{BlockHeader, Transaction, Withdrawal},
};

/// Witness for a block.
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlockWitness {
    /// Chain id
    pub chain_id: ChainId,
    /// Block header representation.
    pub header: BlockHeader,
    /// State trie root before the block.
    pub pre_state_root: B256,
    /// Transactions in the block.
    pub transaction: Vec<Transaction>,
    /// Withdrawals in the block.
    pub withdrawals: Option<Vec<Withdrawal>>,
    /// Rlp encoded state trie nodes.
    pub states: Vec<Bytes>,
    /// Code bytecodes
    pub codes: Vec<Bytes>,
}
