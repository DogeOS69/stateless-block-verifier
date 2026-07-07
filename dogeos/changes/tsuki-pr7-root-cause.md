# Tsuki PR #7 Root-Cause Note

## Summary

`cargo test --workspace --features scroll,scroll-all` is failing because the
Tsuki dependency stack changes Scroll execution semantics for existing Scroll
mainnet witnesses. This is not explained by the local `SparseState` de-traiting,
the test hardfork selection, or SBV's precomputed compression-info path.

Treat this as a semantic blocker unless the planner explicitly decides these
Scroll mainnet fixtures should no longer be consensus fixtures for this branch.

## Evidence

- Baseline `dogeos/main` at `69e5dd4` passes the full scroll matrix: 46
  `sbv-core` scroll tests pass, including `test_next_message_index_feynman_fixture`.
- PR commit bisect:
  - `83fe217` does not compile due to a `scroll-alloy-network`
    `NetworkWallet<Scroll>` implementation conflict with alloy's blanket impl.
  - `baea300` compiles and passes the full scroll matrix.
  - `eabc621` compiles and first introduces the 36 `PostStateRootMismatch`
    failures.
  - Branch tip `c9a883a` has the same 36-failure shape.
- The local trie implementation is not the cause. The `baea300..eabc621` diff
  only removes the `reth_stateless::StatelessTrie` trait wrapper and changes
  error mapping; the state-root calculation body is otherwise the same.
- The spec mapping is not the cause:
  - The tests still force `EuclidV2` and `Feynman` for the fixture folders.
  - `spec_id_at_timestamp_and_number` maps these to the same `EUCLID` and
    `FEYNMAN` `ScrollSpecId`s in the passing and failing dependency revisions.
  - `SCROLL_MAINNET` hardfork timestamps for `EuclidV2` and `Feynman` are stable
    across the checked revisions. Tsuki is not active for the affected mainnet
    fixture timestamps.
- Bypassing SBV's precomputed compression-info path did not change the result.
  A temporary diagnostic run changed the scroll executor call from
  `Some(_compression_infos)` to `None`; `14919991` still passed and `14919992`
  still failed with the same computed root.
- The pass/fail pattern points at execution semantics for normal L2
  transactions:
  - Mainnet zero-transaction Feynman blocks pass.
  - The `5343513301-*` Feynman fixtures are L1-message fixtures and pass.
  - A simple EuclidV2 transfer with empty calldata (`14919991`) passes.
  - Non-empty calldata L2 transactions fail, including the next-message-index
    fixture at Scroll mainnet block `20240125`.

## Why This Is Not A Fixture-Only Staleness Fix

The affected fixtures are Scroll mainnet block witnesses whose
`header.state_root` values are canonical block-header roots. Re-dumping the same
blocks from a public Scroll RPC would preserve those expected roots. Updating
only `header.state_root` to the newly computed roots, or skipping/deleting these
fixtures, would make tests green but would stop proving that SBV reproduces the
canonical Scroll execution result for those blocks.

## Root Cause Mechanism

The first failing commit changes the dependency lane from the old
`scroll-v91.1`/`dogeos-reth 4036577` stack to the Tsuki `revm`/`dogeos-revm` and
`dogeos-reth` stack. The exact failing rule is CALL-family static gas after the
`revm` v103 gas-parameter refactor:

- In `scroll-tech/revm` `scroll-v91.1` (`3992cf8`), CALL-family helpers charged
  `calc_call_static_gas()` during opcode execution. For Berlin-and-later specs
  that function charged `WARM_STORAGE_READ_COST` (`100`) before dynamic account
  access gas.
- In `scroll-tech/revm` `feat/v103`, commit
  `2befb622978d29a70a5b73f8902a484d294f33de` (`feat: Gas params (#3132)`)
  removed that helper-side static charge and moved per-spec static gas into
  `instruction_table_gas_changes_spec()`. The bare v103 instruction table keeps
  CALL, CALLCODE, DELEGATECALL, and STATICCALL at the legacy base `40`, while
  the Berlin+ table overlay raises them to `100`.
- The active DogeOS pin in this branch,
  `DogeOS69/dogeos-revm?tag=tsuki-v0.4#d415a9e3ff44bfefd1efbe090a6df70404fe4426`,
  still builds the Scroll custom table with plain `instruction_table()` in
  `make_scroll_instruction_table()`. It overrides Scroll-specific opcodes but
  does not apply `instruction_table_gas_changes_spec()`, so warm CALL-family
  opcodes are undercharged by `60` gas each.

This is an accidental Scroll custom instruction-provider wiring gap against the
v103 API, not a deliberate next-fork repricing. The standard v103 handler path
does apply `instruction_table_gas_changes_spec()`.

## Diagnostic Addendum

Temporary instrumentation compared one failing EuclidV2 fixture and one failing
Feynman fixture on the passing `baea300` stack and the first failing `eabc621`
stack. Full logs were written under `target/diagnostics/` and the temporary
test harness was removed.

- `testdata/scroll/euclidv2/14919992.json`
  - `baea300`: header gas `49274`, executor gas `49274`, root matches.
  - `eabc621`: header gas `49274`, executor gas `49214`, root mismatches.
  - Calldata tokens are `236`; the EIP-7623 floor is `23360`, far below actual
    gas, so the floor is not binding here.
  - L1 fee is unchanged at `1043192161431`.
  - Sender and fee-vault balance deltas differ by exactly
    `60 * 47857518 = 2871451080`, where `60` is the gas delta and `47857518` is
    the transaction gas price.
- `testdata/dogeos/next-message-index/20240125.json`
  - `baea300`: header gas `753811`, executor gas `753811`, root matches.
  - `eabc621`: header gas `753811`, executor gas `744271`, root mismatches.
  - Calldata tokens are `6914`; the EIP-7623 floor is `90140`, far below actual
    gas, so the floor is not binding here.
  - L1 fee is unchanged at `430666570430`.
  - Sender and fee-vault balance deltas differ by exactly
    `9540 * 240316 = 2292614640`, where `9540` is the gas delta and `240316` is
    the transaction gas price.
- Sorted post-state storage writes are identical between `baea300` and
  `eabc621`; the root mismatch comes from gas-derived account balances, not from
  different contract storage execution.

This pins the immediate mechanism as gas accounting/fee settlement for normal
non-L1 transactions. It is not an L1 data fee calculation change, and it is not
the EIP-7623 calldata floor in the traced fixtures.

Additional per-transaction and per-step gas diagnostics name the rule:

- In `14919992`, refund accounting is not involved. The passing `baea300` stack
  reports `spent_before_refund=49274` and `gas_refunded=0`; the first failing
  `eabc621` stack reports `spent_before_refund=49214` and `gas_refunded=0`.
- The first trace divergence is the proxy `DELEGATECALL` (`0xf4`) at depth `1`,
  program counter `31`. The child frame receives more forwarded gas on the v103
  stack because the parent undercharges the warm DELEGATECALL static gas by
  `60`; after the child returns, the parent frame remains exactly `60` gas
  cheaper through settlement.
- The larger Feynman fixture delta is the same mechanism at scale:
  `9540 = 159 * 60`, matching 159 affected warm CALL-family opcode executions.

## Upstream Calibration

No upstream green calibration point was found for this fixture corpus on
`revm` `feat/v103`. Checked refs in `scroll-tech/stateless-block-verifier` show:

- Current upstream `master`, release tag `scroll-v91.2`, and the green
  GalileoV2 PR #156 use `scroll-tech/revm?tag=scroll-v91` and
  `scroll-tech/reth?tag=scroll-v91.2`.
- Other live upstream branches use older or different revm branches/commits
  such as `feat/reth-v78`, `feat/reth-v74`, or `scroll-evm-executor/feat/v55`.
- No upstream branch, tag, PR search result, or recent CI run was found that
  consumes `revm` `feat/v103` or `scroll-v103`.

So upstream CI does not currently prove that the `feat/v103` stack preserves the
same Scroll fixture semantics.

## Release Impact

The traced fixtures are pre-Tsuki Scroll-history fixtures, but the mechanism is
not confined to pre-Tsuki spec ranges. DogeOS mainnet maps to
`ScrollSpecId::TSUKI` at genesis, and `TSUKI` is ordered after `FEYNMAN` and
`GALILEO` in `dogeos-revm`. The non-L1 transaction execution path that reports
receipt gas and settles sender/fee-vault balances is still the normal path under
`TSUKI`.

Therefore this should be treated as a DogeOS release-relevant consensus
question, not merely a Scroll-history replay policy question, unless an explicit
TSUKI fixture/policy decision proves the lower gas accounting is the intended
DogeOS semantics.

## Decision Needed

This falls on the semantic side of the handoff decision fork. Do not apply a
crate-code or dependency fix until the planner/human approves the pin cascade
impact. A fixture-only workaround would be possible only by weakening or
redefining this test coverage, not by refreshing the same canonical fixtures.
