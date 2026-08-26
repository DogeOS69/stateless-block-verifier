# Next Message Index Fixture

## DogeOS Tsuki empty-queue fixture

`tsuki-empty-queue/{11,12,13}.json` are real, consecutive DogeOS block witnesses
copied from the fixture merged in `DogeOS69/scroll-zkvm-prover#20`. They came
from the Tsuki materializer run
`tsuki-definitive-6a13b33f-vast-retry-20260714T011747Z` on chain `6281971`,
using SBV revision `ec6059bd`.

These blocks contain no L1 messages, so replaying all three under an explicitly
forced Tsuki chain spec must leave `next_message_index` at `0`. The forced spec
is required because the development fixture predates Chikyū's public Tsuki
activation timestamp even though the source network was running Tsuki code.

Original SHA-256 digests:

- `11.json`: `e076dd8c56a7320ce3b0c0cf623409d6ef88887c8925551441f7a28de3b1b134`
- `12.json`: `0eb289270eb75f240d66dd68adda6f7440647cc8aa0b2df1f18f752d50038afe`
- `13.json`: `c5409b760753723501632c46efdc72227ad803b17b26f81a4ece4178f4ec3979`

This is the non-advancing case. Keep a deposit-bearing Tsuki witness as a
separate fixture so the `next_message_index > 0` transition remains covered.

## Scroll mainnet transition fixture

`20240125.json` is a Scroll mainnet block witness for block `20240125`.

`14919991-missing-slot1-proof.json` is a companion **regression** fixture: a real
Scroll mainnet EuclidV2 block witness taken from before the `L2MessageQueue`
queue-proof backfill (`git show da8892b^:testdata/scroll/euclidv2/14919991.json`),
so it contains the slot-0 `messageRoot` proof but **not** the slot-1
`nextMessageIndex` proof. It exercises the pre-Tsuki gate: verification must
succeed and return the sentinel `0` without reading slot 1. If the Tsuki gate is
removed, the unconditional slot-1 read panics with `MPT: Unresolved node access`.
Do not backfill this fixture — its missing slot-1 proof is the point.

Why this block:

- upstream `sbv-cli run` replays it successfully
- `L2MessageQueue.nextMessageIndex` changes in this block, so the witness contains
  the trie path needed by DogeOS's post-execution queue-height read

Pinned expectation:

- queue contract: `0x5300000000000000000000000000000000000000`
- storage slot: `0x1` (`nextMessageIndex`)
- block: `20240125`
- expected value after execution: `208530`

How it was collected on March 13, 2026:

```bash
cargo run -p sbv-cli --features scroll -- dump \
  --rpc https://scroll.api.onfinality.io/public \
  --block 20240125 \
  --out-dir /tmp/sbv-probe-20240125

cast storage 0x5300000000000000000000000000000000000000 0x1 \
  --rpc-url https://scroll.api.pocket.network \
  --block 20240125
```

If this fixture needs to be refreshed, verify it still passes:

```bash
cargo run -p sbv-cli --features scroll -- run testdata/dogeos/next-message-index/20240125.json
```

On this branch, `sbv-cli dump --features scroll` appends `L2MessageQueue` proof nodes from
both the parent state and the block's own state automatically, so fixtures remain valid
whether or not the block itself mutates that contract.

The older fixtures under `testdata/scroll/` were backfilled with the same account/storage
proof nodes for `messageRoot` / `nextMessageIndex` so the full Scroll fixture sweep keeps
covering DogeOS's post-execution queue reads, except
`testdata/scroll/feynman/534352-19604670.json`. That fixture intentionally remains in its
original pre-backfill form to guard the pre-Tsuki missing-slot-1-proof path.
