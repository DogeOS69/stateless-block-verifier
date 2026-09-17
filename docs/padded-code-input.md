# Padded code input

SBV can execute code directly from a retained guest input buffer using the existing
public APIs in pinned `revm-bytecode` 9. The optional padded-code verifier entry
point preserves the original code bytes, code length, code hash, and block replay
result. It requires no REVM changes or dependency patches.

[`BlockWitness::deserialize_from_static`](../crates/core/src/witness.rs) accepts a
deserializer over input that remains valid for the program's lifetime. Its
[`StaticBytes` visitor](../crates/core/src/witness/static_input.rs) wraps borrowed
binary byte slices with `Bytes::from_static`. Cloning those `Bytes`, slicing them,
and collecting cloned handles into witness or trie maps do not copy the payload.
The ordinary `BlockWitness` Serde implementation and serialized layout remain
unchanged. Host deserialization continues to own its data; the static entry point
also falls back to owned bytes for formats or readers that cannot lend static
slices. Human-readable formats use the ordinary bytes deserializer.

Static deserialization does not add padding or guarantee alignment. The caller
must supply the padded-code buffers separately and validate each one with
`PaddedCode::new` before using the padded-code verifier entry point.

[`PaddedCode`](../crates/trie/src/padded_code.rs) holds private `Bytes` and
`original_len` fields. Its constructor checks that the absolute buffer address is
4-byte aligned, that `original_len + 33` does not overflow and fits in the buffer,
and that every byte after `original_len` is zero. Original trailing zeros remain
part of the original code. The constructor rejects invalid EIP-7702 designations
whose original bytes start with `0xef01`.

For legacy code, `to_bytecode()` calls `Bytecode::new_legacy(padded.clone())` to
compute the jump table locally. At least 33 trailing zeros cover a truncated
PUSH32 immediate and a terminating STOP; they also cover DUPN, SWAPN and EXCHANGE
tails. With the pinned REVM implementation, analysis retains this already padded
payload. SBV then calls `Bytecode::new_analyzed(analyzed.bytes(), original_len,
cloned_jump_table)` to restore the original length. No witness-supplied jump map
is accepted, and zero padding introduces no jump destinations. This path still
allocates metadata and jump tables; cloning the jump table copies its bitset.

EIP-7702 uses `Bytecode::new_eip7702_raw` on the original 23-byte shared view,
preserving delegation behavior and excluding the envelope padding. Empty legacy
code keeps an original length of zero while retaining a nonempty execution buffer.

The public entry points are:

| API | Purpose |
| --- | --- |
| `sbv::core::BlockWitness::deserialize_from_static(deserializer)` | Borrow eligible byte fields from retained static input. |
| `sbv::trie::PaddedCode::new(padded: Bytes, original_len: usize)` | Validate input and return `Result<PaddedCode, PaddedCodeError>`. Constants `ALIGNMENT` and `MIN_PADDING` expose the required layout. |
| `PaddedCode::{original_bytes, original_byte_slice, original_len, padded_bytes}` | Access original or padded views without copying the payload. |
| `PaddedCode::to_bytecode()` | Produce analyzed REVM bytecode backed by the same payload. |
| `SparseState::new_with_padded_codes(witness, pre_state_root, padded_codes)` | Construct trie state and the code map using `&[PaddedCode]`. |
| `sbv::core::verifier::run_with_padded_codes(witnesses, chain_spec, ..., padded_codes)` | Verify blocks using padded code; Scroll additionally takes `compression_infos` before `padded_codes`. |

[`SparseState`](../crates/trie/src/lib.rs) computes each lookup key from the
original code bytes. Callers cannot associate arbitrary hash keys with code.
Padded entries are inserted before ordinary witness code; an ordinary entry with
the same hash does not trigger a second bytecode analysis or payload copy. Codes
absent from the sidecar retain the ordinary constructor path, which may allocate
execution padding. Supplying the same code in both inputs still hashes it twice.
A caller that supplies every code through the sidecar can clear the corresponding
`BlockWitness.codes` vectors before verification to avoid that duplicate work.

[`verifier::run`](../crates/core/src/verifier/mod.rs) retains its existing signature
and shares the verification implementation with the new entry point. State-node
handles follow the existing trie path.

Validation is covered by
[`padded_code.rs`](../crates/trie/tests/padded_code.rs),
[`padded_verifier.rs`](../crates/core/tests/padded_verifier.rs), and
[`static_witness.rs`](../crates/core/tests/static_witness.rs). The focused tests
check payload-pointer reuse, original bytes/hash/length, jump-destination parity,
every truncated PUSH1–PUSH32 immediate, STOP and immediate-opcode tails, empty
code, EIP-7702, and malformed ranges, alignment, and padding. Real Ethereum and
Tsuki fixtures replay with identical gas use, roots, and block hashes through
ordinary and padded paths, including when ordinary code vectors are empty.
Tsuki also compares the withdrawal root and next-message index. Both feature
configurations can be checked together with the existing core/static-witness tests:

```sh
cargo test --offline --locked -j4 -p sbv-core --lib --test static_witness --test padded_verifier -p sbv-trie --test padded_code
cargo test --offline --locked -j4 -p sbv-core --features scroll,scroll-compress-info --lib --test static_witness --test padded_verifier -p sbv-trie --test padded_code
```
