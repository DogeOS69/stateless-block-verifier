use crate::database::WitnessDatabase;
use sbv_primitives::{
    U256,
    chainspec::{ChainSpec, EthChainSpec, scroll::ChainConfig},
    types::{
        consensus::BlockHeader,
        reth::{
            evm::{EvmFactory, RethReceiptBuilder, block::BlockExecutionError},
            execution_types::BlockExecutionOutput,
            primitives::{Block, Receipt, RecoveredBlock},
        },
    },
};
use std::sync::Arc;

/// EVM executor that handles the block.
#[derive(Debug)]
pub struct EvmExecutor<'a> {
    chain_spec: Arc<ChainSpec>,
    db: WitnessDatabase<'a>,
    block: &'a RecoveredBlock<Block>,
    compression_infos: Option<Vec<(U256, usize)>>,
}

impl<'a> EvmExecutor<'a> {
    /// Create a new EVM executor
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        db: WitnessDatabase<'a>,
        block: &'a RecoveredBlock<Block>,
        compression_infos: Option<Vec<(U256, usize)>>,
    ) -> Self {
        Self {
            chain_spec,
            db,
            block,
            compression_infos,
        }
    }
}

impl EvmExecutor<'_> {
    /// Handle the block with the given witness
    pub fn execute(self) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError> {
        use sbv_primitives::types::{
            evm::{
                EvmEnv, ScrollBlockExecutionCtx, ScrollBlockExecutor, ScrollBlockExecutorFactory,
                ScrollDefaultPrecompilesFactory, ScrollEvmFactory, spec_id_at_timestamp_and_number,
            },
            reth::evm::execute::BlockExecutor,
            revm::{
                BlockEnv, CfgEnv, ScrollCfgExt,
                database::{State, states::bundle_state::BundleRetention},
            },
        };

        let factory = ScrollBlockExecutorFactory::new(
            RethReceiptBuilder,
            self.chain_spec.clone(),
            ScrollEvmFactory::<ScrollDefaultPrecompilesFactory>::default(),
        );

        let mut db = State::builder()
            .with_database(self.db)
            .with_bundle_update()
            .build();

        let header = self.block.header();
        let spec_id =
            spec_id_at_timestamp_and_number(header.timestamp(), header.number(), &self.chain_spec);
        let cfg_env = CfgEnv::new_scroll(spec_id).with_chain_id(self.chain_spec.chain().id());
        let beneficiary = self
            .chain_spec
            .chain_config()
            .fee_vault_address
            .unwrap_or_else(|| header.beneficiary());
        let evm = factory.evm_factory().create_evm(
            &mut db,
            EvmEnv::new(
                cfg_env,
                BlockEnv {
                    number: U256::from(header.number()),
                    beneficiary,
                    timestamp: U256::from(header.timestamp()),
                    gas_limit: header.gas_limit(),
                    basefee: header.base_fee_per_gas().unwrap_or_default(),
                    difficulty: header.difficulty(),
                    prevrandao: header.mix_hash(),
                    blob_excess_gas_and_price: None,
                    slot_num: 0,
                },
            ),
        );
        let ctx = ScrollBlockExecutionCtx {
            parent_hash: header.parent_hash(),
        };
        let executor =
            ScrollBlockExecutor::new(evm, ctx, factory.spec().clone(), factory.receipt_builder());

        let result = cycle_track!(
            match self.compression_infos {
                None => {
                    executor.execute_block(self.block.transactions_recovered())
                }
                Some(compression_infos) => executor.execute_block_with_compression_cache(
                    self.block.transactions_recovered(),
                    compression_infos,
                ),
            },
            "handle_block"
        )?;
        db.merge_transitions(BundleRetention::Reverts);

        Ok(BlockExecutionOutput {
            result,
            state: db.take_bundle(),
        })
    }
}
