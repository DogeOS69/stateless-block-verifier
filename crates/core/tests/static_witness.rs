//! Compatibility and ownership tests for static witness deserialization.

use sbv_core::BlockWitness;
use sbv_primitives::{B256, Bytes, types::Header};

fn witness() -> BlockWitness {
    BlockWitness {
        chain_id: 6_281_971,
        header: Header {
            number: 42,
            gas_limit: 30_000_000,
            gas_used: 42_000,
            extra_data: Bytes::from_static(b"static witness"),
            ..Default::default()
        },
        prev_state_root: B256::repeat_byte(0x12),
        transactions: Vec::new(),
        withdrawals: Some(Default::default()),
        #[cfg(not(feature = "scroll"))]
        block_hashes: vec![B256::repeat_byte(0x34)],
        states: vec![
            Bytes::from_static(&[0xc4, 1, 0x82, 0xaa, 0xbb]),
            Bytes::new(),
        ],
        codes: vec![
            Bytes::from_static(&[0x60, 0, 0x00]),
            vec![0x5b; 257].into(),
            Bytes::new(),
        ],
    }
}

fn encode(witness: &BlockWitness) -> Vec<u8> {
    bincode::serde::encode_to_vec(witness, bincode::config::standard()).unwrap()
}

fn decode_static(input: &'static [u8]) -> BlockWitness {
    let mut decoder =
        bincode::serde::BorrowedSerdeDecoder::from_slice(input, bincode::config::standard(), ());
    BlockWitness::deserialize_from_static(decoder.as_deserializer()).unwrap()
}

fn inside(input: &[u8], bytes: &[u8]) -> bool {
    let input = input.as_ptr_range();
    let bytes = bytes.as_ptr_range();
    bytes.start >= input.start && bytes.end <= input.end
}

#[test]
fn static_binary_decode_reuses_input_and_preserves_wire_format() {
    let expected = witness();
    let input: &'static [u8] = encode(&expected).leak();
    let decoded = decode_static(input);

    assert_eq!(decoded, expected);
    assert_eq!(encode(&decoded), input);
    for bytes in decoded.states.iter().chain(&decoded.codes) {
        if !bytes.is_empty() {
            assert!(inside(input, bytes), "witness bytes were copied");
        }
    }

    let cloned = decoded.clone();
    for (original, cloned) in decoded
        .states
        .iter()
        .chain(&decoded.codes)
        .zip(cloned.states.iter().chain(&cloned.codes))
    {
        assert_eq!(original.as_ptr(), cloned.as_ptr());
    }
    drop(decoded);
    assert_eq!(cloned, expected);
    assert_eq!(cloned.states.last().unwrap().len(), 0);
    assert_eq!(cloned.codes.last().unwrap().len(), 0);
}

#[test]
fn ordinary_binary_decode_keeps_ownership_of_witness_bytes() {
    let expected = witness();
    let mut input = encode(&expected);
    let (decoded, used): (BlockWitness, usize) =
        bincode::serde::decode_from_slice(&input, bincode::config::standard()).unwrap();
    assert_eq!(used, input.len());
    for bytes in decoded.states.iter().chain(&decoded.codes) {
        if !bytes.is_empty() {
            assert!(!inside(&input, bytes), "ordinary decode borrowed its input");
        }
    }

    input.fill(0xff);
    drop(input);
    assert_eq!(decoded, expected);
}

#[test]
fn static_entry_accepts_owned_reader_data_without_borrowing() {
    let expected = witness();
    let mut input = encode(&expected);
    let decoded = {
        let reader = bincode::de::read::SliceReader::new(&input);
        let mut decoder =
            bincode::serde::OwnedSerdeDecoder::from_reader(reader, bincode::config::standard());
        BlockWitness::deserialize_from_static(decoder.as_deserializer()).unwrap()
    };
    for bytes in decoded.states.iter().chain(&decoded.codes) {
        if !bytes.is_empty() {
            assert!(!inside(&input, bytes));
        }
    }

    input.fill(0xff);
    drop(input);
    assert_eq!(decoded, expected);
}

#[test]
fn static_entry_preserves_json_hex_and_byte_array_support() {
    let mut expected = witness();
    expected.states = vec![Bytes::from_static(&[1, 2]), Bytes::new()];
    expected.codes = vec![Bytes::from_static(&[0, 0xab]), Bytes::new()];
    let mut json = serde_json::to_value(&expected).unwrap();
    json["states"] = serde_json::json!(["0x0102", []]);
    json["codes"] = serde_json::json!([[0, 171], "0x"]);
    let input: &'static str = serde_json::to_string(&json).unwrap().leak();
    let mut decoder = serde_json::Deserializer::from_str(input);
    let decoded = BlockWitness::deserialize_from_static(&mut decoder).unwrap();

    assert_eq!(decoded, expected);
    for bytes in decoded.states.iter().chain(&decoded.codes) {
        if !bytes.is_empty() {
            assert!(!inside(input.as_bytes(), bytes));
        }
    }
}

#[test]
fn static_entry_preserves_missing_states_default() {
    let mut expected = witness();
    expected.states.clear();
    let mut json = serde_json::to_value(&expected).unwrap();
    json.as_object_mut().unwrap().remove("states");
    let input: &'static str = serde_json::to_string(&json).unwrap().leak();
    let mut decoder = serde_json::Deserializer::from_str(input);

    assert_eq!(
        BlockWitness::deserialize_from_static(&mut decoder).unwrap(),
        expected
    );
}
