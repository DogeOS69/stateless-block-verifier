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

Witness consumers should use `sbv-core::BlockWitness`, which holds native Alloy `Header`,
Scroll `TxEnvelope`, and Alloy `Withdrawals` values. The `rkyv` features,
`sbv-primitives::legacy_types`, `BlockWitness::into_legacy`, and conversion from the legacy
witness have been removed. Older JSON using `pre_state_root` and `transaction` must be migrated
with an earlier SBV revision before upgrading. Current witness JSON and binary Serde formats
continue to use the upstream `serde_bincode_compat` adapters: native headers omit optional fields
and native transaction envelopes use tagged Serde representations that are unsuitable for bincode.

## Cli Interface

See [./crates/bin/README.md](./crates/bin/README.md) for more details.
