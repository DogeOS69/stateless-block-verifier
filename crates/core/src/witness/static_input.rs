//! Borrowing deserialization for witness input retained for the lifetime of the program.

use super::BlockWitness;
use sbv_primitives::{
    B256, Bytes, ChainId,
    types::{Header, consensus::TxEnvelope, eips::eip4895::Withdrawals},
};
use serde::{Deserialize, Deserializer, de::Visitor};
use serde_with::DeserializeAs;

// A remote derive preserves the public witness type and its ordinary, owning deserializer.
// It also checks that these fields still construct BlockWitness when its schema changes.
#[serde_with::serde_as]
#[derive(Deserialize)]
#[serde(remote = "BlockWitness", bound(deserialize = "'de: 'static"))]
pub(super) struct StaticBlockWitness {
    chain_id: ChainId,
    #[serde_as(as = "sbv_primitives::types::consensus::serde_bincode_compat::Header")]
    header: Header,
    prev_state_root: B256,
    #[serde_as(as = "Vec<sbv_primitives::types::consensus::serde_bincode_compat::TxEnvelope<'_>>")]
    transactions: Vec<TxEnvelope>,
    withdrawals: Option<Withdrawals>,
    #[cfg(not(feature = "scroll"))]
    block_hashes: Vec<B256>,
    #[serde(default)]
    #[serde_as(deserialize_as = "Vec<StaticBytes>")]
    states: Vec<Bytes>,
    #[serde_as(deserialize_as = "Vec<StaticBytes>")]
    codes: Vec<Bytes>,
}

struct StaticBytes;

impl<'de: 'static> DeserializeAs<'de, Bytes> for StaticBytes {
    fn deserialize_as<D>(deserializer: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return Bytes::deserialize(deserializer);
        }

        // Unlike deserialize_byte_buf, deserialize_bytes lets slice-backed formats supply
        // the original input. The 'static bound makes this safe without extending a lifetime.
        deserializer.deserialize_bytes(StaticBytesVisitor)
    }
}

struct StaticBytesVisitor;

impl<'de: 'static> Visitor<'de> for StaticBytesVisitor {
    type Value = Bytes;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a byte array")
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> Result<Self::Value, E> {
        Ok(Bytes::from_static(value))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E> {
        Ok(Bytes::copy_from_slice(value))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E> {
        Ok(Bytes::from(value))
    }

    fn visit_seq<A>(self, sequence: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        Vec::<u8>::deserialize(serde::de::value::SeqAccessDeserializer::new(sequence))
            .map(Bytes::from)
    }
}
