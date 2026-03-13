# Next Message Index Overlay

## Summary

This overlay exposes Scroll's `nextMessageIndex` from the L2 message queue as
part of `VerifyResult`.

## Scope

- Read storage slot `1` from the Scroll `L2MessageQueue` predeploy at
  `0x5300000000000000000000000000000000000000`.
- Convert the value to `u64` and return an error if it does not fit.
- Thread the value out through `VerifyResult` so downstream crates can include
  it in public inputs.

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

## Future test coverage

- Add DogeOS-only witness fixtures under `testdata/dogeos/next-message-index/`.
- Prefer one narrow regression test that proves the extracted value matches the
  queue contract state for a known witness.
