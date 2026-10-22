use std::sync::Arc;

pub use reth_chainspec::{Chain, EthChainSpec, ForkCondition, NamedChain};

pub use dogeos_chainspec as scroll;
pub use dogeos_chainspec::{DOGEOS_CHIKYU, DOGEOS_DEV, DOGEOS_MAINNET};

/// Scroll chain spec type.
pub type ChainSpec = scroll::DogeosChainSpec;

/// Get chain spec
pub fn get_chain_spec(chain: Chain) -> Option<Arc<ChainSpec>> {
    registered_spec(chain).cloned()
}

/// The registered DogeOS specification for a chain id, if the chain is one we know.
///
/// The chain ids come from the specifications themselves (mainnet and Chikyū from their frozen
/// genesis documents) so a genesis change cannot leave a stale literal behind.
fn registered_spec(chain: Chain) -> Option<&'static Arc<ChainSpec>> {
    [&DOGEOS_MAINNET, &DOGEOS_CHIKYU, &DOGEOS_DEV]
        .into_iter()
        .map(|spec| &**spec)
        .find(|spec| spec.chain == chain)
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
pub fn build_chain_spec_force_hardfork(
    chain: Chain,
    hardfork: crate::hardforks::Hardfork,
) -> Arc<ChainSpec> {
    use crate::hardforks::Hardfork;
    use dogeos_chainspec::DogeosChainSpecBuilder;

    // The base-fee rule reads the L2SystemConfig contract named by the chain's own
    // `ScrollChainConfig`, and the EIP-1559 parameters come from the chain's own genesis document,
    // so a forced-hardfork spec has to start from the specification of the chain being proven
    // rather than always from the development chain. An unregistered chain id keeps the
    // development blueprint, whose `l2_system_config_address` is the zero address.
    let mut builder = if chain == DOGEOS_MAINNET.chain {
        DogeosChainSpecBuilder::dogeos_mainnet()
    } else if chain == DOGEOS_CHIKYU.chain {
        DogeosChainSpecBuilder::dogeos_chikyu()
    } else {
        DogeosChainSpecBuilder::dev()
    }
    .chain(chain);
    for fork in [
        Hardfork::Feynman,
        Hardfork::Galileo,
        Hardfork::GalileoV2,
        Hardfork::Tsuki,
    ] {
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

    let config = registered_spec(chain).map_or(DOGEOS_DEV.config, |spec| spec.config);
    Arc::new(builder.build(config))
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_chain_specs_are_dogeos_only() {
        use super::*;

        for (chain, spec) in [
            (Chain::from_id(6_281_971), &*DOGEOS_CHIKYU),
            (Chain::from_id(1_122_005), &*DOGEOS_MAINNET),
            (Chain::dev(), &*DOGEOS_DEV),
        ] {
            let resolved = get_chain_spec(chain).expect("known DogeOS chain");
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

    /// Chikyū is the exception, and it is recorded here rather than left to be discovered: its
    /// published genesis hash is a hash the specification carries, not one the DogeOS header
    /// derivation reproduces from its genesis document. A Chikyū chunk therefore cannot be
    /// anchored to a genesis header built from the chain spec.
    #[test]
    fn the_chikyu_genesis_hash_is_stored_not_derived() {
        use super::*;
        use dogeos_chainspec::DOGEOS_CHIKYU_GENESIS_HASH;

        assert_eq!(DOGEOS_CHIKYU.genesis_hash(), DOGEOS_CHIKYU_GENESIS_HASH);
        assert_ne!(
            DOGEOS_CHIKYU.genesis_header().hash_slow(),
            DOGEOS_CHIKYU_GENESIS_HASH
        );
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
