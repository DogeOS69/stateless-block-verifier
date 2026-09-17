//! Full block replay parity for the optional padded-code verifier entry point.

#![cfg(any(not(feature = "scroll"), feature = "scroll-compress-info"))]

use sbv_core::{BlockWitness, verifier};
use sbv_primitives::{Bytes, chainspec::ChainSpec};
use sbv_trie::PaddedCode;
use std::sync::Arc;

fn assert_replay_parity(mut witness: BlockWitness, chain_spec: Arc<ChainSpec>) {
    let padded_codes = witness
        .codes
        .iter()
        .map(|original| {
            let mut input =
                vec![0; original.len() + PaddedCode::MIN_PADDING + PaddedCode::ALIGNMENT - 1];
            let start = input.as_ptr().align_offset(PaddedCode::ALIGNMENT);
            let end = start + original.len() + PaddedCode::MIN_PADDING;
            input[start..start + original.len()].copy_from_slice(original);
            let padded = Bytes::from(input).slice(start..end);
            PaddedCode::new(padded, original.len()).unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        !padded_codes.is_empty(),
        "fixture must exercise witness code"
    );

    let ordinary = verifier::run_host(std::slice::from_ref(&witness), chain_spec.clone()).unwrap();
    #[cfg(feature = "scroll")]
    let compression_infos = vec![witness.compression_infos()];

    // Exercise both a complete ordinary witness plus sidecar and a witness with
    // all code supplied only through the padded-code entry point.
    for omit_ordinary_codes in [false, true] {
        if omit_ordinary_codes {
            witness.codes.clear();
        }
        let padded = verifier::run_with_padded_codes(
            std::slice::from_ref(&witness),
            chain_spec.clone(),
            #[cfg(feature = "scroll")]
            compression_infos.clone(),
            &padded_codes,
        )
        .unwrap();
        assert_eq!(padded.pre_state_root, ordinary.pre_state_root);
        assert_eq!(padded.post_state_root, ordinary.post_state_root);
        assert_eq!(padded.gas_used, ordinary.gas_used);
        assert_eq!(padded.blocks[0].hash(), ordinary.blocks[0].hash());
        #[cfg(feature = "scroll")]
        {
            assert_eq!(padded.withdraw_root, ordinary.withdraw_root);
            assert_eq!(padded.next_message_index, ordinary.next_message_index);
        }
    }
}

#[cfg(not(feature = "scroll"))]
#[test]
fn ethereum_block_replay_matches_ordinary_code() {
    use sbv_primitives::chainspec::{Chain, get_chain_spec};

    let witness: BlockWitness =
        serde_json::from_str(include_str!("../../../testdata/ethereum/1-23587691.json")).unwrap();
    let chain_spec = get_chain_spec(Chain::from_id(witness.chain_id)).unwrap();
    assert_replay_parity(witness, chain_spec);
}

#[cfg(all(feature = "scroll", feature = "scroll-compress-info"))]
#[test]
fn tsuki_block_replay_matches_ordinary_code() {
    use sbv_primitives::{
        chainspec::{Chain, build_chain_spec_force_hardfork},
        hardforks::Hardfork,
    };

    let witness: BlockWitness = serde_json::from_str(include_str!(
        "../../../testdata/dogeos/next-message-index/tsuki-empty-queue/11.json"
    ))
    .unwrap();
    let chain_spec =
        build_chain_spec_force_hardfork(Chain::from_id(witness.chain_id), Hardfork::Tsuki);
    assert_replay_parity(witness, chain_spec);
}
