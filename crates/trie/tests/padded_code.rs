//! Semantic and ownership checks for code borrowed from padded witness input.

use alloy_trie::EMPTY_ROOT_HASH;
use sbv_primitives::{
    Address, Bytes, keccak256,
    types::{revm::Bytecode, rpc::ExecutionWitness},
};
use sbv_trie::{PaddedCode, PaddedCodeError, SparseState};

fn buffer(original: &[u8], padding: usize) -> Bytes {
    // Select an aligned range explicitly: Vec<u8> itself only promises alignment 1.
    let mut input = vec![0; original.len() + padding + PaddedCode::ALIGNMENT - 1];
    let start = input.as_ptr().align_offset(PaddedCode::ALIGNMENT);
    let end = start + original.len() + padding;
    input[start..start + original.len()].copy_from_slice(original);
    let input: &'static [u8] = input.leak();
    Bytes::from_static(&input[start..end])
}

fn assert_unchanged(original: &[u8], padding: usize) -> Bytecode {
    let input = buffer(original, padding);
    let input_ptr = input.as_ptr();
    let padded = PaddedCode::new(input.clone(), original.len()).unwrap();
    let bytecode = padded.to_bytecode();
    let ordinary = Bytecode::new_raw(Bytes::copy_from_slice(original));

    assert_eq!(padded.padded_bytes().as_ptr(), input_ptr);
    assert_eq!(padded.clone().padded_bytes().as_ptr(), input_ptr);
    assert_eq!(bytecode.bytecode_ptr(), input_ptr, "payload was copied");
    assert_eq!(bytecode.clone().bytecode_ptr(), input_ptr);
    assert_eq!(padded.original_len(), original.len());
    assert_eq!(padded.original_byte_slice(), original);
    assert_eq!(padded.original_bytes().as_ref(), original);
    assert_eq!(bytecode.len(), original.len());
    assert_eq!(bytecode.is_empty(), original.is_empty());
    assert_eq!(bytecode.original_byte_slice(), original);
    assert_eq!(bytecode.original_bytes().as_ref(), original);
    assert_eq!(bytecode.hash_slow(), keccak256(original));
    assert_eq!(bytecode.hash_slow(), ordinary.hash_slow());
    assert_eq!(bytecode.kind(), ordinary.kind());
    if let Some(jumps) = bytecode.legacy_jump_table() {
        let ordinary_jumps = ordinary.legacy_jump_table().unwrap();
        for offset in 0..input.len() + 2 {
            assert_eq!(
                jumps.is_valid(offset),
                ordinary_jumps.is_valid(offset),
                "jump destination differs at {offset} for {original:?}"
            );
        }
    }
    drop(input);
    drop(padded);
    assert_eq!(bytecode.original_byte_slice(), original);
    bytecode
}

#[test]
fn legacy_original_bytes_hash_length_and_payload_pointer_are_preserved() {
    for original in [
        vec![],
        vec![0x00],
        vec![0x5b, 0x60, 0x5b, 0x5b, 0x00],
        vec![0x60, 0x01, 0x00, 0x00, 0x00],
        vec![0xef],
        vec![0xef, 0x00],
        vec![0x5b; 131_072],
    ] {
        for padding in [33, 34, 36, 65] {
            assert_unchanged(&original, padding);
        }
    }
}

#[test]
fn every_truncated_push_immediate_uses_the_borrowed_padding() {
    for immediate_len in 1..=32 {
        for present in 0..immediate_len {
            // JUMPDEST in immediate data must never become a valid jump target.
            let mut original = vec![0x5b, 0x60, 0x5b, 0x5b, 0x5f + immediate_len];
            original.extend(std::iter::repeat_n(0x5b, usize::from(present)));
            let bytecode = assert_unchanged(&original, PaddedCode::MIN_PADDING);
            assert!(
                bytecode.bytes_slice()[original.len()..]
                    .iter()
                    .all(|byte| *byte == 0)
            );
        }
    }
}

#[test]
fn immediate_opcode_tails_keep_revm_analysis_and_original_lengths() {
    // DUPN, SWAPN and EXCHANGE each consume a one-byte immediate.
    for opcode in [0xe6, 0xe7, 0xe8] {
        for tail in [vec![], vec![0x00], vec![0x5b], vec![0x00, 0x00]] {
            let mut original = vec![0x5b, opcode];
            original.extend(tail);
            assert_unchanged(&original, PaddedCode::MIN_PADDING);
        }
    }
}

#[test]
fn eip7702_uses_only_the_original_23_byte_view() {
    let address = Address::repeat_byte(0x42);
    let mut original = vec![0xef, 0x01, 0x00];
    original.extend_from_slice(address.as_slice());
    let bytecode = assert_unchanged(&original, PaddedCode::MIN_PADDING);

    assert!(bytecode.is_eip7702());
    assert_eq!(bytecode.eip7702_address(), Some(address));
    assert_eq!(bytecode.bytes_slice().len(), 23);
    assert!(bytecode.legacy_jump_table().is_none());
}

#[test]
fn malformed_ranges_padding_and_alignment_are_rejected() {
    let good = buffer(&[0x60], PaddedCode::MIN_PADDING);
    for original_len in [usize::MAX, usize::MAX - 32] {
        assert_eq!(
            PaddedCode::new(good.clone(), original_len).unwrap_err(),
            PaddedCodeError::LengthOverflow
        );
    }
    for original_len in [2, good.len(), good.len() + 1] {
        assert_eq!(
            PaddedCode::new(good.clone(), original_len).unwrap_err(),
            PaddedCodeError::InsufficientPadding
        );
    }
    assert_eq!(
        PaddedCode::new(Bytes::new(), 0).unwrap_err(),
        PaddedCodeError::InsufficientPadding
    );
    assert_eq!(
        PaddedCode::new(buffer(&[], 32), 0).unwrap_err(),
        PaddedCodeError::InsufficientPadding
    );
    for offset in 1..PaddedCode::ALIGNMENT {
        let unaligned =
            buffer(&[], PaddedCode::MIN_PADDING + PaddedCode::ALIGNMENT).slice(offset..);
        assert_eq!(
            PaddedCode::new(unaligned, 0).unwrap_err(),
            PaddedCodeError::Misaligned
        );
    }
    for padding_offset in [1, 17, 33, 40] {
        let mut invalid = vec![0; 41];
        invalid[padding_offset] = 0x5b;
        assert_eq!(
            PaddedCode::new(buffer(&invalid, 0), 1).unwrap_err(),
            PaddedCodeError::NonzeroPadding
        );
    }
}

#[test]
fn malformed_eip7702_is_not_reinterpreted_as_legacy_code() {
    for original_len in [2, 3, 22, 24] {
        let mut original = vec![0; original_len];
        original[..2].copy_from_slice(&[0xef, 0x01]);
        assert_eq!(
            PaddedCode::new(buffer(&original, 33), original.len()).unwrap_err(),
            PaddedCodeError::InvalidEip7702
        );
    }
    let mut original = vec![0; 23];
    original[..3].copy_from_slice(&[0xef, 0x01, 0x01]);
    assert_eq!(
        PaddedCode::new(buffer(&original, 33), original.len()).unwrap_err(),
        PaddedCodeError::InvalidEip7702
    );
}

#[test]
fn sparse_state_hashes_original_code_and_keeps_the_ordinary_fallback() {
    let original = [0x5b, 0x7f];
    let padded = PaddedCode::new(buffer(&original, 33), original.len()).unwrap();
    let padded_ptr = padded.padded_bytes().as_ptr();
    let ordinary = Bytes::from_static(&[0x60, 0x2a, 0x00]);
    let witness = ExecutionWitness {
        // A matching ordinary entry must not displace the borrowed padded bytes.
        codes: vec![Bytes::copy_from_slice(&original), ordinary.clone()],
        ..Default::default()
    };
    let (_, codes) = SparseState::new_with_padded_codes(
        &witness,
        EMPTY_ROOT_HASH,
        std::slice::from_ref(&padded),
    )
    .unwrap();

    assert_eq!(codes.len(), 2);
    let code = &codes[&keccak256(original)];
    assert_eq!(code.bytecode_ptr(), padded_ptr);
    assert_eq!(code.original_byte_slice(), original);
    assert_eq!(code.hash_slow(), keccak256(original));
    assert!(!codes.contains_key(&keccak256(padded.padded_bytes())));
    assert_eq!(codes[&keccak256(&ordinary)].original_bytes(), ordinary);

    let (_, ordinary_codes) = SparseState::new(&witness, EMPTY_ROOT_HASH).unwrap();
    for (hash, code) in codes {
        assert_eq!(code, ordinary_codes[&hash]);
    }
}

#[test]
fn sparse_state_accepts_padded_only_empty_and_eip7702_code() {
    let original = [0xef, 0x01, 0x00]
        .into_iter()
        .chain([0x42; 20])
        .collect::<Vec<_>>();
    let padded_codes = [
        PaddedCode::new(buffer(&[], 33), 0).unwrap(),
        PaddedCode::new(buffer(&original, 33), original.len()).unwrap(),
    ];
    let (_, codes) = SparseState::new_with_padded_codes(
        &ExecutionWitness::default(),
        EMPTY_ROOT_HASH,
        &padded_codes,
    )
    .unwrap();
    assert_eq!(codes.len(), 2);
    assert!(codes[&keccak256([])].is_empty());
    assert!(codes[&keccak256(&original)].is_eip7702());
    for padded in &padded_codes {
        assert_eq!(
            codes[&keccak256(padded.original_byte_slice())].bytecode_ptr(),
            padded.padded_bytes().as_ptr()
        );
    }
}
