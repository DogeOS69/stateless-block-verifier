use std::sync::Arc;

pub use reth_chainspec::{
    BaseFeeParams, Chain, EthChainSpec, ForkCondition, Hardforks, NamedChain,
};

pub use dogeos_chainspec as scroll;
pub use dogeos_chainspec::{DOGEOS_CHIKYU, DOGEOS_DEV, DOGEOS_MAINNET};

/// Scroll chain spec type.
pub type ChainSpec = scroll::DogeosChainSpec;

/// Get chain spec
pub fn get_chain_spec(chain: Chain) -> Option<Arc<ChainSpec>> {
    registered(chain).map(|entry| entry.spec.clone())
}

/// Whether this build knows the chain, and so has its genesis document and its configuration.
///
/// An unknown chain falls back to the development blueprint, whose L2SystemConfig address is the
/// zero address, so a rule that depends on that configuration has to refuse an unknown chain
/// rather than read the blueprint's placeholder.
pub fn is_registered_chain(chain: Chain) -> bool {
    registered(chain).is_some()
}

/// A chain this build knows: its specification, and the builder blueprint that starts from that
/// chain's own genesis document.
struct RegisteredChain {
    spec: &'static Arc<ChainSpec>,
    blueprint: fn() -> dogeos_chainspec::DogeosChainSpecBuilder,
}

/// The one registry of known DogeOS chains.
///
/// The chain ids come from the specifications themselves, so a genesis change cannot leave a stale
/// literal behind, and the blueprint lives beside the specification it belongs to: adding a chain
/// here cannot leave the forced-hardfork path grafting that chain's configuration onto the
/// development genesis.
fn registered(chain: Chain) -> Option<RegisteredChain> {
    use dogeos_chainspec::DogeosChainSpecBuilder;

    [
        (
            &*DOGEOS_MAINNET,
            DogeosChainSpecBuilder::dogeos_mainnet as fn() -> DogeosChainSpecBuilder,
        ),
        (
            &*DOGEOS_CHIKYU,
            DogeosChainSpecBuilder::dogeos_chikyu as fn() -> DogeosChainSpecBuilder,
        ),
        (
            &*DOGEOS_DEV,
            DogeosChainSpecBuilder::dev as fn() -> DogeosChainSpecBuilder,
        ),
    ]
    .into_iter()
    .find(|(spec, _)| spec.chain == chain)
    .map(|(spec, blueprint)| RegisteredChain { spec, blueprint })
}

/// Get chain spec or build one from dev config as blueprint
pub fn get_chain_spec_or_build<F>(chain: Chain, f: F) -> Arc<ChainSpec>
where
    F: Fn(&mut ChainSpec),
{
    get_chain_spec(chain).unwrap_or_else(|| {
        let mut spec = {
            let mut spec = (**DOGEOS_DEV).clone();
            spec.inner.chain = chain;
            spec
        };

        f(&mut spec);
        Arc::new(spec)
    })
}

/// Build a chain spec with a hardfork, enabling all hardforks up to the specified one.
///
/// A forced-hardfork spec starts from the registered specification of the chain being proven,
/// because the base-fee rule reads that chain's own L2SystemConfig contract and anchoring opens
/// that chain's own genesis header. An unregistered chain id keeps the development blueprint,
/// whose `l2_system_config_address` is the zero address; the verifier refuses to derive a base fee
/// from an unset L2SystemConfig address rather than reading the zero account and taking the
/// default overhead.
///
/// The forced schedule covers every DogeOS execution fork, and only those.
///
/// # Panics
///
/// If the chain's genesis document declares `tsukiFeeParamsTime`. The EIP-1559 fee schedule is a
/// property of the chain and the execution client reads it from the genesis document, so a forced
/// schedule either ignores the declaration - diverging from the node for exactly the chains that
/// make one - or silently overrides the fork the caller asked for. Neither is safe for a rule that
/// feeds public inputs, so such a chain has to be proven with its own specification.
pub fn build_chain_spec_force_hardfork(
    chain: Chain,
    hardfork: crate::hardforks::Hardfork,
) -> Arc<ChainSpec> {
    use dogeos_chainspec::DogeosChainSpecBuilder;

    match registered(chain) {
        Some(entry) => build_forced_spec(
            chain,
            hardfork,
            entry.blueprint,
            entry.spec.genesis().clone(),
            entry.spec.config,
        ),
        None => build_forced_spec(
            chain,
            hardfork,
            DogeosChainSpecBuilder::dev,
            DOGEOS_DEV.genesis().clone(),
            DOGEOS_DEV.config,
        ),
    }
}

/// Builds a forced-hardfork spec from an explicit blueprint, genesis document and configuration.
///
/// Split out so a test can supply a genesis the registered chains do not have.
///
/// # Panics
///
/// If `genesis` declares `tsukiFeeParamsTime`; see [`build_chain_spec_force_hardfork`].
fn build_forced_spec(
    chain: Chain,
    hardfork: crate::hardforks::Hardfork,
    blueprint: fn() -> dogeos_chainspec::DogeosChainSpecBuilder,
    genesis: alloy_genesis::Genesis,
    config: dogeos_chainspec::ScrollChainConfig,
) -> Arc<ChainSpec> {
    use crate::hardforks::Hardfork;

    let declared = dogeos_chainspec::DogeosHardforkInfo::try_from(&genesis.config.extra_fields)
        .expect("a genesis document has valid hardfork timestamps")
        .tsuki_fee_params_time;
    assert!(
        declared.is_none(),
        "chain {chain} declares a delayed fee-parameter activation, which a forced-hardfork spec \
         cannot represent: use the chain's own specification"
    );

    let mut builder = blueprint().chain(chain).genesis(genesis);
    // `Hardfork::dev()` is the schedule of every DogeOS execution fork, so a fork added upstream
    // joins the forced schedule instead of silently keeping the blueprint chain's own activation.
    for (fork, _) in Hardfork::dev() {
        builder = builder.with_fork(
            fork,
            if fork <= hardfork {
                ForkCondition::Timestamp(0)
            } else {
                ForkCondition::Never
            },
        );
    }
    sbv_helpers::dev_info!(
        "Building chain spec for chain {} with hardfork {:?}",
        chain,
        hardfork
    );

    Arc::new(builder.build(config))
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
    }

    /// Mainnet's derived genesis header must hash to the published mainnet genesis hash: the
    /// chain's first chunk is anchored to that header, so its hash is the first proof endpoint,
    /// and the guest derives the header from the same genesis document the node does.
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
            build_chain_spec_force_hardfork(
                DOGEOS_MAINNET.chain,
                crate::hardforks::Hardfork::Tsuki
            )
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

    /// A genesis that declares its own delayed D48/E10 activation keeps it, and a forced-hardfork
    /// spec refuses to be built for such a chain rather than silently moving the activation.
    #[test]
    fn a_declared_fee_params_activation_is_honoured() {
        use super::*;
        use dogeos_chainspec::DogeosChainSpec;

        let activation = 2_000_000_000;
        let mut genesis = DOGEOS_MAINNET.genesis().clone();
        genesis
            .config
            .extra_fields
            .insert("tsukiFeeParamsTime".into(), activation.into());
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

    /// A genesis that declares a delayed activation cannot be represented by a forced schedule,
    /// so building one is refused through the construction path the public helper uses.
    #[test]
    #[should_panic(expected = "declares a delayed fee-parameter activation")]
    fn forcing_a_fork_over_a_declared_fee_params_activation_is_refused() {
        use super::*;
        use crate::hardforks::Hardfork;
        use dogeos_chainspec::DogeosChainSpecBuilder;

        let mut genesis = DOGEOS_MAINNET.genesis().clone();
        genesis
            .config
            .extra_fields
            .insert("tsukiFeeParamsTime".into(), 2_000_000_000u64.into());

        let _ = build_forced_spec(
            DOGEOS_MAINNET.chain,
            Hardfork::Tsuki,
            DogeosChainSpecBuilder::dogeos_mainnet,
            genesis,
            DOGEOS_MAINNET.config,
        );
    }

    /// The registered chains declare no delayed activation, so forcing a fork is representable
    /// for them and gives the fork's own parameters.
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
            assert_eq!(
                build_chain_spec_force_hardfork(chain, Hardfork::Feynman)
                    .base_fee_params_at_timestamp(0),
                BaseFeeParams::new(8, 2),
                "chain {chain}"
            );
        }
    }

    #[test]
    fn force_tsuki_chain_spec_activates_tsuki() {
        use super::*;
        use crate::hardforks::Hardfork;

        let chain_spec = build_chain_spec_force_hardfork(Chain::from_id(42424242), Hardfork::Tsuki);

        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::GalileoV2, 0));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Tsuki, 0));
    }
}
