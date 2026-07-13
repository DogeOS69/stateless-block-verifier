# Next Message Index Overlay

## Summary

This overlay exposes Scroll's `nextMessageIndex` from the L2 message queue as
part of `VerifyResult`.

## Scope

- Read storage slot `1` from the Scroll `L2MessageQueue` predeploy at
  `0x5300000000000000000000000000000000000000`.
- Only read that slot once the Tsuki hardfork is active. Before Tsuki the slot
  proof is not guaranteed to be part of the witness (and may be absent), so the
  verifier returns the sentinel `0` instead of reading it. `0` is a sentinel, not
  necessarily the real on-chain `nextMessageIndex` for a pre-Tsuki block.
- Convert the value to `u64` and return an error if it does not fit.
- Thread the value out through `VerifyResult` so downstream crates can include
  it in public inputs. Consumers must not treat a pre-Tsuki `0` as authoritative.

## Why this stays maintainable

- The change is limited to the Scroll verifier path.
- The patch is a single-purpose commit that rebases cleanly.
- No upstream branch policy changes are required; `master` remains an exact
  upstream mirror.

## Upkeep notes

- Rebase this branch onto `dogeos/main`, then rebase `dogeos/main` onto
  `master`.
- If upstream starts exposing `nextMessageIndex` directly, drop this overlay
  commit instead of carrying a duplicate implementation.

## Test coverage

- `testdata/dogeos/next-message-index/20240125.json` pins a real Scroll mainnet
  witness where `nextMessageIndex` changes during the block (its real on-chain
  value after execution is `208530`). Because Scroll mainnet never activates
  Tsuki, `test_next_message_index_pre_tsuki_sentinel` asserts the verifier
  returns the **sentinel `0`**, not the on-chain value — this is the intended
  "don't break witness before Tsuki" behavior.
- `testdata/dogeos/next-message-index/14919991-missing-slot1-proof.json` is a
  pre-backfill Scroll mainnet EuclidV2 witness (from `da8892b^`) that lacks the
  slot-1 (`nextMessageIndex`) proof node.
  `test_next_message_index_pre_tsuki_missing_slot1_proof_is_sentinel` verifies it
  succeeds pre-Tsuki and returns `0`; removing the Tsuki gate makes it panic with
  `MPT: Unresolved node access`, so this test keeps the gate load-bearing.
- `test_next_message_index_overflow` asserts a slot value above `u64::MAX` is
  rejected with an error rather than panicking.
- The committed Scroll fixture sweep under `testdata/scroll/` is backfilled with
  `L2MessageQueue` proof nodes so the existing replay tests continue to work on
  the DogeOS verifier path.

## Pre-release follow-up

- Post-Tsuki extraction coverage is not yet asserted: it needs a genuine
  Tsuki-active DogeOS block witness in which `nextMessageIndex` changes,
  including the `NativeDogeToken` account proof (`0x5300..d09e`, touched by the
  Tsuki migration) and the slot-1 proof. Add such a fixture, assert the real
  extracted value, and land it before release.
