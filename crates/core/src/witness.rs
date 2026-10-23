use auto_impl::auto_impl;
use itertools::Itertools;
use sbv_primitives::{
    B256, Bytes, ChainId, SignatureError, U256,
    types::{
        Header,
        consensus::{SignerRecoverable, TxEnvelope},
        eips::eip4895::Withdrawals,
        reth::primitives::{Block, BlockBody, RecoveredBlock, SealedBlock},
    },
};

/// Witness for a block.
#[serde_with::serde_as]
#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlockWitness {
    /// Chain id
    pub chain_id: ChainId,
    /// Block header representation.
    #[serde_as(as = "sbv_primitives::types::consensus::serde_bincode_compat::Header")]
    pub header: Header,
    /// State trie root before the block.
    pub prev_state_root: B256,
    /// Transactions in the block.
    #[serde_as(as = "Vec<sbv_primitives::types::consensus::serde_bincode_compat::TxEnvelope<'_>>")]
    pub transactions: Vec<TxEnvelope>,
    /// Withdrawals in the block.
    pub withdrawals: Option<Withdrawals>,
    /// Rlp encoded state trie nodes.
    #[serde(default)]
    pub states: Vec<Bytes>,
    /// Code bytecodes
    pub codes: Vec<Bytes>,
}

impl BlockWitness {
    /// Calculates compression ratios for all transactions in the block witness.
    ///
    /// # Panics
    ///
    /// Panics if called without the "scroll-compress-info" feature enabled, as this
    /// functionality is not intended to be used in guest environments.
    pub fn compression_infos(&self) -> Vec<(U256, usize)> {
        #[cfg(feature = "scroll-compress-info")]
        {
            use sbv_primitives::types::{consensus::Transaction, eips::Encodable2718};

            self.transactions
                .iter()
                .map(|tx| {
                    (
                        sbv_primitives::types::evm::compute_compression_ratio(&tx.input()),
                        sbv_primitives::types::evm::compute_compressed_size(&tx.encoded_2718()),
                    )
                })
                .collect()
        }
        #[cfg(not(feature = "scroll-compress-info"))]
        {
            unimplemented!("you should not build ChunkWitness in guest?");
        }
    }

    /// Build execution context from the witness.
    pub fn build_reth_block(&self) -> Result<RecoveredBlock<Block>, SignatureError> {
        let senders = self
            .transactions
            .iter()
            .map(|tx| tx.recover_signer())
            .collect::<Result<Vec<_>, _>>()
            .expect("Failed to recover signer");

        let body = BlockBody {
            transactions: self.transactions.clone(),
            ommers: vec![],
            withdrawals: self.withdrawals.clone(),
        };
        let block = RecoveredBlock::new_sealed(
            SealedBlock::seal_slow(Block {
                header: self.header.clone(),
                body,
            }),
            senders,
        );

        Ok(block)
    }
}

/// BlockWitnessCodeExt trait
#[auto_impl(&, &mut, Box, Rc, Arc)]
pub trait BlockWitnessChunkExt {
    /// Get the chain id.
    fn chain_id(&self) -> ChainId;
    /// Get the previous state root.
    fn prev_state_root(&self) -> B256;
    /// Check if all witnesses have the same chain id.
    fn has_same_chain_id(&self) -> bool;
    /// Check if all witnesses have a sequence block number.
    fn has_seq_block_number(&self) -> bool;
    /// Check if all witnesses have a sequence state root.
    fn has_seq_state_root(&self) -> bool;
}

impl BlockWitnessChunkExt for [BlockWitness] {
    #[inline(always)]
    fn chain_id(&self) -> ChainId {
        debug_assert!(self.has_same_chain_id(), "chain id mismatch");
        self.first().expect("empty witnesses").chain_id
    }

    #[inline(always)]
    fn prev_state_root(&self) -> B256 {
        self.first().expect("empty witnesses").prev_state_root
    }

    #[inline(always)]
    fn has_same_chain_id(&self) -> bool {
        self.iter()
            .tuple_windows()
            .all(|(a, b)| a.chain_id == b.chain_id)
    }

    #[inline(always)]
    fn has_seq_block_number(&self) -> bool {
        self.iter()
            .tuple_windows()
            .all(|(a, b)| a.header.number + 1 == b.header.number)
    }

    #[inline(always)]
    fn has_seq_state_root(&self) -> bool {
        self.iter()
            .tuple_windows()
            .all(|(a, b)| a.header.state_root == b.prev_state_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sbv_primitives::{
        Address, Signature,
        types::{
            consensus::{
                SignableTransaction, TxEip1559, TxEip2930, TxEip7702, TxL1Message, TxLegacy,
            },
            eips::{
                eip2930::{AccessList, AccessListItem},
                eip4895::Withdrawal,
                eip7702::{Authorization, SignedAuthorization},
            },
        },
    };

    fn assert_serde_roundtrip(witness: &BlockWitness) {
        let json = serde_json::to_vec(witness).unwrap();
        let decoded: BlockWitness = serde_json::from_slice(&json).unwrap();
        assert_eq!(&decoded, witness);

        let config = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(witness, config).unwrap();
        let (decoded, consumed): (BlockWitness, _) =
            bincode::serde::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(&decoded, witness);
    }

    #[test]
    fn persisted_witness_serde_roundtrip() {
        for json in [
            include_str!("../../../testdata/scroll/feynman/534352-19604672.json"),
            include_str!("../../../testdata/scroll/feynman/5343513301-1.json"),
            include_str!("../../../testdata/dogeos/next-message-index/tsuki-empty-queue/12.json"),
        ] {
            let witness: BlockWitness = serde_json::from_str(json).unwrap();
            assert_serde_roundtrip(&witness);
        }
    }

    #[test]
    fn native_witness_serde_roundtrip_covers_all_transaction_variants() {
        let signature = Signature::new(U256::from(1), U256::from(2), false);
        let address = Address::repeat_byte(0x11);
        let access_list = AccessList(vec![AccessListItem {
            address,
            storage_keys: vec![B256::repeat_byte(0x22)],
        }]);
        let authorization = SignedAuthorization::new_unchecked(
            Authorization {
                chain_id: U256::from(1),
                address,
                nonce: 2,
            },
            0,
            signature.r(),
            signature.s(),
        );
        let mut witness = BlockWitness {
            chain_id: 1,
            header: Header::default(),
            prev_state_root: B256::repeat_byte(0x33),
            transactions: vec![
                TxLegacy::default().into_signed(signature).into(),
                TxEip2930 {
                    access_list,
                    ..Default::default()
                }
                .into_signed(signature)
                .into(),
                TxEip1559::default().into_signed(signature).into(),
                TxEip7702 {
                    authorization_list: vec![authorization],
                    ..Default::default()
                }
                .into_signed(signature)
                .into(),
                TxL1Message {
                    queue_index: 7,
                    sender: address,
                    ..Default::default()
                }
                .into(),
            ],
            withdrawals: None,
            states: vec![Bytes::from_static(&[0xc0])],
            codes: vec![Bytes::from_static(&[0x60, 0x00])],
        };
        assert_serde_roundtrip(&witness);

        witness.header.base_fee_per_gas = Some(42);
        witness.header.withdrawals_root = Some(B256::repeat_byte(0x44));
        witness.header.blob_gas_used = Some(1);
        witness.header.excess_blob_gas = Some(2);
        witness.header.parent_beacon_block_root = Some(B256::repeat_byte(0x55));
        witness.header.requests_hash = Some(B256::repeat_byte(0x66));
        witness.withdrawals = Some(Withdrawals::new(vec![Withdrawal {
            index: 3,
            validator_index: 4,
            address,
            amount: 5,
        }]));
        assert_serde_roundtrip(&witness);
    }
}
