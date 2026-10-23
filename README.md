# Stateless Block Verifier

This project provides tools for stateless verification of Scroll/DogeOS blocks with MPT state roots.
All builds use Scroll execution, transaction, receipt, and RPC types. Ethereum block verification
is not supported.

The existing `scroll` and `scroll-*` Cargo features remain available for downstream compatibility.
They no longer select a chain mode; capability features such as `chainspec` and `revm-types` now
expose Scroll/DogeOS types directly. The `ethereum-all` feature has been removed.

`sbv-core` can be used by zkVM guests without additional features. Host callers of `run_host`
must enable `scroll-compress-info`; this feature includes compression support that must stay out
of guest builds. The CLI enables it automatically. Public-input-only consumers can continue to
use `sbv-primitives/scroll-reth-consensus-types` without enabling Scroll execution or compression.

Verification uses `sbv_core::verifier::VerifyContext`, with a required owned parent header on
every fork. The caller must authenticate that header as the previous proof endpoint; the verifier
checks the first witness's height, parent hash, and pre-state root against it. Replace calls to
`verifier::run` with:

```rust
use sbv_core::verifier::VerifyContext;

let result = VerifyContext {
    witnesses,
    parent_header: parent_header.clone(),
    chain_spec,
    compression_infos,
}
.verify()?;
```

`verify(self)` consumes the context. `run_host` computes compression information for host callers
and accepts the same required owned `Header`.

Witness consumers should use `sbv-core::BlockWitness`, which holds native Alloy `Header`,
Scroll `TxEnvelope`, and Alloy `Withdrawals` values. The `rkyv` features,
`sbv-primitives::legacy_types`, `BlockWitness::into_legacy`, and conversion from the legacy
witness have been removed. Older JSON using `pre_state_root` and `transaction` must be migrated
with an earlier SBV revision before upgrading. Current witness JSON and binary Serde formats
continue to use the upstream `serde_bincode_compat` adapters: native headers omit optional fields
and native transaction envelopes use tagged Serde representations that are unsuitable for bincode.

## DogeOS dependency pins

The workspace uses the following release tags, matching dogeos-reth's dependency pins:

| Repository | Revision |
| --- | --- |
| `DogeOS69/dogeos-reth` | `tsuki-v0.9` (`581afef2e3960a4e3e755c9a56412f89b2e2f265`) |
| `DogeOS69/reth` | `v2.0.0-dogeos-20261023.1` (`42b56304227691327c44415262d23ff680a44ffb`) |
| `DogeOS69/dogeos-revm` | `tsuki-v0.9` (`d4ac4dac66e5648099a0277eb63f606fc04caff7`) |

Direct and transitive dependencies use the same tags and Git sources; `Cargo.lock` pins their commits.
The executor now reads L2SystemConfig slot 101 before execution on every fork, including empty
blocks. Witnesses must include the account/storage proof or an authenticated absence. Legacy
Feynman replay tests are ignored because their witnesses lack this proof; general verifier
regressions run against the complete Tsuki corpora.

## Cli Interface

See [./crates/bin/README.md](./crates/bin/README.md) for more details.
