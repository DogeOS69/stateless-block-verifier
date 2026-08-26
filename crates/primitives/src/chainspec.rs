use std::sync::Arc;

pub use reth_chainspec::{self, *};

#[cfg(feature = "scroll-chainspec")]
pub use dogeos_chainspec as scroll;
#[cfg(feature = "scroll-chainspec")]
pub use dogeos_chainspec::{DOGEOS_CHIKYU, DOGEOS_DEV, DOGEOS_MAINNET};

/// An Ethereum chain specification.
///
/// A chain specification describes:
///
/// - Meta-information about the chain (the chain ID)
/// - The genesis block of the chain (Genesis)
/// - What hardforks are activated, and under which conditions
#[cfg(not(feature = "scroll-chainspec"))]
pub type ChainSpec = reth_chainspec::ChainSpec;
/// Scroll chain spec type.
#[cfg(feature = "scroll-chainspec")]
pub type ChainSpec = scroll::DogeosChainSpec;

/// Get chain spec
#[cfg(not(feature = "scroll-chainspec"))]
pub fn get_chain_spec(chain: Chain) -> Option<Arc<ChainSpec>> {
    if chain == Chain::from_named(NamedChain::Mainnet) {
        return Some(MAINNET.clone());
    }
    if chain == Chain::from_named(NamedChain::Sepolia) {
        return Some(SEPOLIA.clone());
    }
    if chain == Chain::from_named(NamedChain::Holesky) {
        return Some(HOLESKY.clone());
    }
    if chain == Chain::dev() {
        return Some(DEV.clone());
    }
    None
}

/// Get chain spec
#[cfg(feature = "scroll-chainspec")]
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
        #[cfg(not(feature = "scroll-chainspec"))]
        let mut spec = {
            let mut spec = (**DEV).clone();
            spec.chain = chain;
            spec
        };
        #[cfg(feature = "scroll-chainspec")]
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
#[cfg(feature = "scroll-chainspec")]
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

/// Build a chain spec with a hardfork, enabling all hardforks up to the specified one.
#[cfg(not(feature = "scroll"))]
pub fn build_chain_spec_force_hardfork(
    chain: Chain,
    hardfork: crate::hardforks::Hardfork,
) -> Arc<ChainSpec> {
    use crate::{U256, hardforks::Hardfork};
    use std::sync::{Arc, LazyLock};

    static BASE_HARDFORKS: LazyLock<ChainHardforks> = LazyLock::new(|| {
        ChainHardforks::new(vec![(
            EthereumHardfork::Frontier.boxed(),
            ForkCondition::Block(0),
        )])
    });

    let mut hardforks = BASE_HARDFORKS.clone();

    if hardfork >= Hardfork::Homestead {
        hardforks.insert(hardfork, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Dao {
        hardforks.insert(Hardfork::Dao, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Tangerine {
        hardforks.insert(Hardfork::Tangerine, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::SpuriousDragon {
        hardforks.insert(Hardfork::SpuriousDragon, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Byzantium {
        hardforks.insert(Hardfork::Byzantium, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Constantinople {
        hardforks.insert(Hardfork::Constantinople, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Petersburg {
        hardforks.insert(Hardfork::Petersburg, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Istanbul {
        hardforks.insert(Hardfork::Istanbul, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Berlin {
        hardforks.insert(Hardfork::Berlin, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::London {
        hardforks.insert(Hardfork::London, ForkCondition::Block(0));
    }

    if hardfork >= Hardfork::Paris {
        hardforks.insert(
            Hardfork::Paris,
            ForkCondition::TTD {
                activation_block_number: 0,
                fork_block: Some(0),
                total_difficulty: U256::ZERO,
            },
        );
    }

    if hardfork >= Hardfork::Shanghai {
        hardforks.insert(Hardfork::Shanghai, ForkCondition::Timestamp(0));
    }

    if hardfork >= Hardfork::Cancun {
        hardforks.insert(Hardfork::Cancun, ForkCondition::Timestamp(0));
    }

    if hardfork >= Hardfork::Prague {
        hardforks.insert(Hardfork::Prague, ForkCondition::Timestamp(0));
    }

    if hardfork >= Hardfork::Osaka {
        hardforks.insert(Hardfork::Osaka, ForkCondition::Timestamp(0));
    }

    Arc::new(ChainSpec {
        chain,
        hardforks,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "scroll-chainspec")]
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

    #[cfg(feature = "scroll-chainspec")]
    #[test]
    fn force_tsuki_chain_spec_activates_tsuki() {
        use super::*;
        use crate::hardforks::Hardfork;

        let chain_spec = build_chain_spec_force_hardfork(Chain::from_id(42424242), Hardfork::Tsuki);

        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::GalileoV2, 0));
        assert!(chain_spec.is_fork_active_at_timestamp(Hardfork::Tsuki, 0));
    }
}
