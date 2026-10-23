use std::sync::Arc;

pub use reth_chainspec::{Chain, EthChainSpec, ForkCondition, NamedChain};

pub use dogeos_chainspec as scroll;
pub use dogeos_chainspec::{DOGEOS_CHIKYU, DOGEOS_DEV, DOGEOS_MAINNET};

/// Scroll chain spec type.
pub type ChainSpec = scroll::DogeosChainSpec;

/// Get chain spec
pub fn get_chain_spec(chain: Chain) -> Option<Arc<ChainSpec>> {
    match chain.id() {
        6_281_971 => Some(DOGEOS_CHIKYU.clone()),
        0xff => Some(DOGEOS_MAINNET.clone()),
        id if id == Chain::dev().id() => Some(DOGEOS_DEV.clone()),
        _ => None,
    }
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

    let mut builder = DogeosChainSpecBuilder::dev().chain(chain);
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

    Arc::new(builder.build(DOGEOS_DEV.config))
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_chain_specs_are_dogeos_only() {
        use super::*;

        for (chain, spec) in [
            (Chain::from_id(6_281_971), &*DOGEOS_CHIKYU),
            (Chain::from_id(0xff), &*DOGEOS_MAINNET),
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

    #[test]
    fn force_tsuki_chain_spec_activates_tsuki() {
        use super::*;
        use crate::hardforks::Hardfork;

        let chain_spec = build_chain_spec_force_hardfork(Chain::from_id(42424242), Hardfork::Tsuki);

        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::GalileoV2, 0));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Tsuki, 0));
    }
}
