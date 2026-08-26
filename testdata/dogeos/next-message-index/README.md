# Next Message Index Fixtures

This directory covers the pre-Tsuki sentinel path and both advancing and
non-advancing Tsuki-enabled development-network replays.

## DogeOS Tsuki advancing fixture

`6281971-19.json` is a genuine block witness from the local DogeOS
`real_scroll_reth_withdrawal_readiness` topology. Block 19 calls
`withdrawToL1(address)` and changes `L2MessageQueue.nextMessageIndex` from `0`
in the parent state to `1` in the final state.

The test uses an explicitly forced Tsuki chain spec. This is intentional: the
fixture came from a Tsuki-enabled local network, while the registered Chikyū
spec does not activate Tsuki for its timestamp.

Pinned provenance:

- chain ID: `6281971` (`0x5fdaf3`)
- block number: `19` (`0x13`)
- timestamp: `1784102950` (`0x6a574026`)
- block hash: `0x17ce064e49dc6f59353d35487eee042140490b885e73275a69ad23982494ecbc`
- parent state root: `0x80479f9622ccb9d62a40ee02a217494c17413163ab10596ba189ab6be3901c88`
- post-state root: `0x0824e511633e44abe11ded8c271dbfcecea21263cb37fc84d1835dc379b5724d`
- parent slot-1 value: `0`
- final slot-1 value: `1`
- fixture SHA-256: `9083f9ec8bec979386d2944eb0777b6a8d4395fad4f3ed7906adcee7b0a54866`
- rollup-node image: `dogeos69/rollup-node:tsuki-5bce327d-reth-39b31f82`
- image digest: `sha256:ed13066066e22bd5c220827b678a6cf59858fbd0d44463f9fcd3ccd97ec76e5b`
- embedded `reth-scroll-cli` revision: `39b31f822cc2b4c54db32ba2f0484ca2a157c3f5`
- SBV collection/replay base: `ec6059bdc48fb60d8340ba86b32bbe8d41111cd4`

The witness came directly from `sbv-cli dump`; it was not hand-pruned and no
proof nodes were manually appended. Independent `eth_getProof` checks covered
queue slots 0/1 and the NativeDogeToken account at the parent and final states.
The canonical parent and header roots are unchanged.

## DogeOS Tsuki empty-queue fixture

`tsuki-empty-queue/{11,12,13}.json` are real, consecutive DogeOS block witnesses
copied from the fixture merged in `DogeOS69/scroll-zkvm-prover#20`. They came
from the Tsuki materializer run
`tsuki-definitive-6a13b33f-vast-retry-20260714T011747Z` on chain `6281971`,
using SBV revision `ec6059bd`.

These blocks contain no L1 messages, so replaying all three under an explicitly
forced Tsuki chain spec must leave `next_message_index` at `0`. The forced spec
is required because the registered Chikyū spec does not activate Tsuki for
these development-network fixtures.

Original SHA-256 digests:

- `11.json`: `e076dd8c56a7320ce3b0c0cf623409d6ef88887c8925551441f7a28de3b1b134`
- `12.json`: `0eb289270eb75f240d66dd68adda6f7440647cc8aa0b2df1f18f752d50038afe`
- `13.json`: `c5409b760753723501632c46efdc72227ad803b17b26f81a4ece4178f4ec3979`

This is the non-advancing case. Keep a deposit-bearing Tsuki witness as a
separate fixture so the `next_message_index > 0` transition remains covered.

## Scroll mainnet pre-Tsuki fixtures

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
