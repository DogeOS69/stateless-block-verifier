use std::sync::Arc;

use alloy_consensus::Header;
use alloy_primitives::U256;
use dogeos_chainspec::{
    DOGEOS_BASE_FEE_PARAMS_FEYNMAN, DOGEOS_BASE_FEE_PARAMS_TSUKI, ScrollChainConfig,
};
use dogeos_hardforks::DogeosChainHardforks;
use reth_chainspec::BaseFeeParamsKind;
use reth_primitives_traits::SealedHeader;

use crate::hardforks::{
    ChainHardforks, EthereumHardfork, EthereumHardforks, Hardfork, HardforkTrait,
};

pub use reth_chainspec::{
    BaseFeeParams, Chain, EthChainSpec, ForkCondition, Hardforks, NamedChain,
};

pub use dogeos_chainspec as scroll;
pub use dogeos_chainspec::{DOGEOS_CHIKYU, DOGEOS_DEV, DOGEOS_MAINNET};

mod genesis;

/// Scroll chain spec type.
pub type ChainSpec = scroll::DogeosChainSpec;

/// Get the full host chain spec, including genesis allocations.
///
/// This initializes the selected upstream genesis document. Guests should use
/// [`build_chain_spec_force_hardfork`] instead.
pub fn get_chain_spec(chain: Chain) -> Option<Arc<ChainSpec>> {
    registered(chain).map(|entry| match entry {
        RegisteredChain::Mainnet => DOGEOS_MAINNET.clone(),
        RegisteredChain::Chikyu => DOGEOS_CHIKYU.clone(),
        RegisteredChain::Dev => DOGEOS_DEV.clone(),
    })
}

/// Whether this build knows the chain's execution configuration, without loading its genesis.
///
/// Unknown chains fall back to the development configuration, whose L2SystemConfig address is
/// zero. The verifier refuses to derive a base fee from that placeholder for an unknown chain.
pub fn is_registered_chain(chain: Chain) -> bool {
    registered(chain).is_some()
}

enum RegisteredChain {
    Mainnet,
    Chikyu,
    Dev,
}

fn registered(chain: Chain) -> Option<RegisteredChain> {
    match chain.id() {
        1_122_005 => Some(RegisteredChain::Mainnet),
        6_281_971 => Some(RegisteredChain::Chikyu),
        id if id == Chain::dev().id() => Some(RegisteredChain::Dev),
        _ => None,
    }
}

/// Get chain spec or build one from dev config as blueprint.
///
/// Like [`get_chain_spec`], this is a host API that loads a full genesis document.
pub fn get_chain_spec_or_build<F>(chain: Chain, f: F) -> Arc<ChainSpec>
where
    F: Fn(&mut ChainSpec),
{
    get_chain_spec(chain).unwrap_or_else(|| {
        let mut spec = (**DOGEOS_DEV).clone();
        spec.inner.chain = chain;
        f(&mut spec);
        Arc::new(spec)
    })
}

/// Native inputs needed to construct a guest execution spec without parsing a genesis document.
///
/// These are trusted chain configuration, to be fixed by the guest program (or authenticated by
/// its caller), not supplied as unchecked witness data. The resulting spec retains the genesis
/// header and execution rules, but omits genesis allocations and JSON extension fields. It is
/// not suitable for initializing a node's genesis state or round-tripping a genesis document.
#[derive(Debug, Clone)]
pub struct GuestChainConfig {
    /// EVM chain identity.
    pub chain: Chain,
    /// Execution settings, including the fee vault and L2SystemConfig contract address.
    pub config: ScrollChainConfig,
    /// The chain's canonical genesis header, with its precomputed state root.
    pub genesis_header: Header,
}

impl GuestChainConfig {
    /// Load native configuration for a registered chain, or the development fallback.
    ///
    /// No upstream lazy chain spec is initialized. Unknown chain IDs retain their identity but
    /// use development settings, matching [`build_chain_spec_force_hardfork`].
    pub fn for_chain(chain: Chain) -> Self {
        let (config, genesis_header) = match registered(chain).unwrap_or(RegisteredChain::Dev) {
            RegisteredChain::Mainnet => (ScrollChainConfig::dogeos_mainnet(), genesis::mainnet()),
            RegisteredChain::Chikyu => (ScrollChainConfig::dogeos_chikyu(), genesis::chikyu()),
            RegisteredChain::Dev => (ScrollChainConfig::dev(), genesis::dev()),
        };
        Self {
            chain,
            config,
            genesis_header,
        }
    }

    /// Build a native chain spec with all DogeOS execution forks through `hardfork` active at zero.
    ///
    /// Forcing Tsuki also forces TsukiV2 (see [`forced_through`]). Neither genesis JSON parsing
    /// nor genesis state-trie hashing is needed.
    pub fn build_force_hardfork(self, hardfork: Hardfork) -> Arc<ChainSpec> {
        let Self {
            chain,
            config,
            genesis_header,
        } = self;
        let hardfork = forced_through(hardfork);
        let mut hardforks = ChainHardforks::default();
        let baseline = DogeosChainHardforks::dev();
        // Reuse DogeOS's Ethereum execution baseline instead of enabling every Ethereum fork.
        for &fork in EthereumHardfork::VARIANTS {
            let condition = baseline.ethereum_fork_activation(fork);
            if condition != ForkCondition::Never {
                hardforks.insert(fork, condition);
            }
        }
        for &fork in Hardfork::VARIANTS {
            hardforks.insert(
                fork,
                if fork <= hardfork {
                    ForkCondition::Timestamp(0)
                } else {
                    ForkCondition::Never
                },
            );
        }

        // Keep header metadata available through EthChainSpec without carrying the allocations
        // or reconstructing the state root. Execution uses `hardforks` and `config` above.
        let genesis = alloy_genesis::Genesis {
            config: alloy_genesis::ChainConfig {
                chain_id: chain.id(),
                ..Default::default()
            },
            nonce: genesis_header.nonce.into(),
            timestamp: genesis_header.timestamp,
            extra_data: genesis_header.extra_data.clone(),
            gas_limit: genesis_header.gas_limit,
            difficulty: genesis_header.difficulty,
            mix_hash: genesis_header.mix_hash,
            coinbase: genesis_header.beneficiary,
            base_fee_per_gas: genesis_header.base_fee_per_gas.map(u128::from),
            ..Default::default()
        };
        Arc::new(ChainSpec {
            inner: reth_chainspec::ChainSpec {
                chain,
                genesis,
                genesis_header: SealedHeader::new_unhashed(genesis_header),
                hardforks,
                base_fee_params: BaseFeeParamsKind::Variable(
                    vec![
                        (Hardfork::Feynman.boxed(), DOGEOS_BASE_FEE_PARAMS_FEYNMAN),
                        (Hardfork::TsukiV2.boxed(), DOGEOS_BASE_FEE_PARAMS_TSUKI),
                    ]
                    .into(),
                ),
                paris_block_and_final_difficulty: Some((0, U256::ZERO)),
                ..Default::default()
            },
            config,
        })
    }
}

/// The last DogeOS fork a forced `hardfork` activates.
///
/// TsukiV2 changes only fee and gas-pricing rules, so the codec and the prover have no fork name
/// for it and keep calling the post-cut era Tsuki. The guest only proves blocks after the
/// TsukiV2 cut, so forcing Tsuki activates TsukiV2 as well; Tsuki without TsukiV2 is the
/// historical Chikyu era, which is never proven.
pub fn forced_through(hardfork: Hardfork) -> Hardfork {
    match hardfork {
        Hardfork::Tsuki => Hardfork::TsukiV2,
        other => other,
    }
}

/// Build a guest execution spec without loading genesis JSON, enabling DogeOS forks through
/// `hardfork` (through TsukiV2 for Tsuki, see [`forced_through`]). Uses the registered chain's
/// execution settings and precomputed genesis header.
///
/// See [`GuestChainConfig`] for explicitly configured chains.
pub fn build_chain_spec_force_hardfork(chain: Chain, hardfork: Hardfork) -> Arc<ChainSpec> {
    GuestChainConfig::for_chain(chain).build_force_hardfork(hardfork)
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_chain_specs_are_dogeos_only() {
        use super::*;

        for spec in [&*DOGEOS_CHIKYU, &*DOGEOS_MAINNET, &*DOGEOS_DEV] {
            let resolved = get_chain_spec(spec.chain()).expect("known DogeOS chain");
            assert!(Arc::ptr_eq(&resolved, spec));
        }

        for chain in [
            NamedChain::Mainnet,
            NamedChain::Sepolia,
            NamedChain::Holesky,
        ] {
            assert!(get_chain_spec(Chain::from_named(chain)).is_none());
        }
    }

    #[test]
    fn test_build_chain_spec() {
        use super::*;
        use crate::hardforks::Hardfork;

        let chain_spec = get_chain_spec_or_build(Chain::from_id(42424242), |spec| {
            spec.inner
                .hardforks
                .insert(Hardfork::Galileo, ForkCondition::Block(10));
        });
        assert_eq!(chain_spec.chain, Chain::from_id(42424242));
        assert!(!chain_spec.is_fork_active_at_block(Hardfork::Galileo, 0));
        assert!(chain_spec.is_fork_active_at_block(Hardfork::Galileo, 10));
    }

    /// Every registered chain resolves by the chain id its own genesis document declares, so a
    /// genesis change cannot leave the lookup pointing at the wrong specification.
    #[test]
    fn registered_chains_resolve_by_their_own_chain_id() {
        use super::*;

        for spec in [&DOGEOS_MAINNET, &DOGEOS_CHIKYU, &DOGEOS_DEV] {
            let resolved = get_chain_spec(spec.chain).expect("a registered chain");
            assert_eq!(resolved.chain, spec.chain);
            assert_eq!(
                resolved.config.l1_config.l2_system_config_address,
                spec.config.l1_config.l2_system_config_address
            );
        }
        assert!(get_chain_spec(Chain::from_id(424_242)).is_none());
        assert!(!is_registered_chain(Chain::from_id(424_242)));
    }

    /// The precomputed guest header must anchor the first chunk to the published genesis hash.
    #[test]
    fn the_derived_mainnet_genesis_header_hashes_to_the_published_genesis_hash() {
        use super::*;
        use dogeos_chainspec::DOGEOS_MAINNET_GENESIS_HASH;

        assert_eq!(
            DOGEOS_MAINNET.genesis_header().hash_slow(),
            DOGEOS_MAINNET_GENESIS_HASH
        );
        assert_eq!(DOGEOS_MAINNET.genesis_hash(), DOGEOS_MAINNET_GENESIS_HASH);
        // A forced-hardfork spec keeps the chain's own genesis, so the guest and the node anchor
        // the first chunk to the same header.
        assert_eq!(
            build_chain_spec_force_hardfork(DOGEOS_MAINNET.chain, Hardfork::Tsuki)
                .genesis_header()
                .hash_slow(),
            DOGEOS_MAINNET_GENESIS_HASH
        );
    }

    /// Chikyū's published genesis hash must match the header derived from its genesis document,
    /// so its first chunk can be anchored to the same header as the execution client.
    #[test]
    fn the_derived_chikyu_genesis_header_hashes_to_the_published_genesis_hash() {
        use super::*;
        use dogeos_chainspec::DOGEOS_CHIKYU_GENESIS_HASH;

        assert_eq!(DOGEOS_CHIKYU.genesis_hash(), DOGEOS_CHIKYU_GENESIS_HASH);
        assert_eq!(
            DOGEOS_CHIKYU.genesis_header().hash_slow(),
            DOGEOS_CHIKYU_GENESIS_HASH
        );
    }

    /// The full host spec switches to D48/E10 at a declared TsukiV2 activation.
    #[test]
    fn a_declared_tsuki_v2_activation_is_honoured() {
        use super::*;
        use dogeos_chainspec::DogeosChainSpec;

        let activation = 2_000_000_000;
        let mut genesis = DOGEOS_MAINNET.genesis().clone();
        genesis
            .config
            .extra_fields
            .insert("tsukiV2Time".into(), activation.into());
        let declared = DogeosChainSpec::from_custom_genesis(genesis);

        assert_eq!(
            declared.base_fee_params_at_timestamp(activation - 1),
            BaseFeeParams::new(8, 2)
        );
        assert_eq!(
            declared.base_fee_params_at_timestamp(activation),
            BaseFeeParams::new(48, 10)
        );
    }

    /// Compare native configuration against the pinned upstream implementation. This catches
    /// stale IDs, headers, execution settings and fee schedules when dependencies are repinned.
    #[test]
    fn guest_specs_match_upstream_execution_configuration() {
        use super::*;
        use dogeos_chainspec::DogeosChainSpecBuilder;

        for (full, blueprint) in [
            (
                &*DOGEOS_MAINNET,
                DogeosChainSpecBuilder::dogeos_mainnet as fn() -> DogeosChainSpecBuilder,
            ),
            (&*DOGEOS_CHIKYU, DogeosChainSpecBuilder::dogeos_chikyu),
            (&*DOGEOS_DEV, DogeosChainSpecBuilder::dev),
        ] {
            let config = GuestChainConfig::for_chain(full.chain);
            assert!(is_registered_chain(full.chain));
            assert_eq!(config.config, full.config);
            assert_eq!(&config.genesis_header, full.genesis_header());

            for &forced in Hardfork::VARIANTS {
                let guest = config.clone().build_force_hardfork(forced);
                let mut builder = blueprint();
                for &fork in Hardfork::VARIANTS {
                    builder = builder.with_fork(
                        fork,
                        if fork <= forced_through(forced) {
                            ForkCondition::Timestamp(0)
                        } else {
                            ForkCondition::Never
                        },
                    );
                }
                let expected = builder.build(full.config);
                assert_eq!(guest.chain, expected.chain);
                assert_eq!(guest.genesis_hash(), expected.genesis_hash());
                assert_eq!(guest.genesis_timestamp(), expected.genesis_timestamp());
                assert_eq!(
                    guest.final_paris_total_difficulty(),
                    expected.final_paris_total_difficulty()
                );
                assert_eq!(guest.base_fee_params, expected.base_fee_params);
                let schedule = |spec: &ChainSpec| {
                    spec.forks_iter()
                        .map(|(fork, condition)| (fork.name(), condition))
                        .collect::<std::collections::BTreeMap<_, _>>()
                };
                assert_eq!(schedule(&guest), schedule(&expected));
                assert!(guest.genesis().alloc.is_empty());
                assert!(guest.genesis().config.extra_fields.is_empty());
            }
        }
    }

    /// The guest proves only post-cut blocks, which the prover names Tsuki: forcing Tsuki must
    /// also activate TsukiV2 and its D48/E10 parameters, on every chain including Chikyu, whose
    /// own schedule activates TsukiV2 later.
    #[test]
    fn forcing_tsuki_also_activates_tsuki_v2() {
        use super::*;
        use crate::hardforks::DogeosHardforks;

        assert_eq!(forced_through(Hardfork::Tsuki), Hardfork::TsukiV2);
        assert_eq!(forced_through(Hardfork::TsukiV2), Hardfork::TsukiV2);
        assert_eq!(forced_through(Hardfork::GalileoV2), Hardfork::GalileoV2);
        assert!(!DOGEOS_CHIKYU.is_tsuki_v2_active_at_timestamp(1_790_030_900));

        for chain in [
            DOGEOS_MAINNET.chain,
            DOGEOS_CHIKYU.chain,
            DOGEOS_DEV.chain,
            Chain::from_id(42424242),
        ] {
            for forced in [Hardfork::Tsuki, Hardfork::TsukiV2] {
                let spec = build_chain_spec_force_hardfork(chain, forced);
                assert_eq!(
                    spec.fork(Hardfork::TsukiV2),
                    ForkCondition::Timestamp(0),
                    "chain {chain}"
                );
                assert!(spec.is_tsuki_v2_active_at_timestamp(0), "chain {chain}");
                assert_eq!(
                    spec.base_fee_params_at_timestamp(0),
                    BaseFeeParams::new(48, 10),
                    "chain {chain}"
                );
            }
            // Below Tsuki, TsukiV2 stays off and the Feynman parameters apply.
            let spec = build_chain_spec_force_hardfork(chain, Hardfork::GalileoV2);
            assert_eq!(spec.fork(Hardfork::TsukiV2), ForkCondition::Never);
            assert_eq!(
                spec.base_fee_params_at_timestamp(u64::MAX),
                BaseFeeParams::new(8, 2)
            );
        }
    }

    /// Mainnet and the development chain run TsukiV2 from genesis, in both the host spec and the
    /// guest configuration derived from it.
    #[test]
    fn mainnet_at_genesis_activates_tsuki_v2_at_zero() {
        use super::*;
        use crate::hardforks::DogeosHardforks;

        for spec in [&*DOGEOS_MAINNET, &*DOGEOS_DEV] {
            assert_eq!(spec.fork(Hardfork::Tsuki), ForkCondition::Timestamp(0));
            assert_eq!(spec.fork(Hardfork::TsukiV2), ForkCondition::Timestamp(0));
            assert!(spec.is_tsuki_v2_active_at_timestamp(0));
            assert_eq!(
                spec.base_fee_params_at_timestamp(0),
                BaseFeeParams::new(48, 10)
            );
        }
    }

    /// Forcing a fork gives that fork's own fee parameters: D48/E10 from TsukiV2 (and so from a
    /// forced Tsuki), D8/E2 below it.
    #[test]
    fn forcing_a_fork_gives_that_forks_fee_parameters() {
        use super::*;
        use crate::hardforks::Hardfork;

        for chain in [DOGEOS_MAINNET.chain, DOGEOS_CHIKYU.chain, DOGEOS_DEV.chain] {
            assert_eq!(
                build_chain_spec_force_hardfork(chain, Hardfork::Tsuki)
                    .base_fee_params_at_timestamp(0),
                BaseFeeParams::new(48, 10),
                "chain {chain}"
            );
            for forced in [Hardfork::Feynman, Hardfork::GalileoV2] {
                assert_eq!(
                    build_chain_spec_force_hardfork(chain, forced).base_fee_params_at_timestamp(0),
                    BaseFeeParams::new(8, 2),
                    "chain {chain}"
                );
            }
        }
    }

    #[test]
    fn force_tsuki_chain_spec_activates_tsuki() {
        use super::*;
        use crate::hardforks::Hardfork;

        let chain_spec = build_chain_spec_force_hardfork(Chain::from_id(42424242), Hardfork::Tsuki);

        assert_eq!(chain_spec.chain, Chain::from_id(42424242));
        assert_eq!(chain_spec.config, ScrollChainConfig::dev());
        assert_eq!(chain_spec.genesis_header(), DOGEOS_DEV.genesis_header());
        assert!(chain_spec.genesis().alloc.is_empty());
        assert!(!is_registered_chain(chain_spec.chain));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::GalileoV2, 0));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Tsuki, 0));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::TsukiV2, 0));
    }
}
