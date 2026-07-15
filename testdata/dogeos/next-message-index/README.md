# Next Message Index Fixtures

This directory covers both sides of the Tsuki gate around
`L2MessageQueue.nextMessageIndex` (queue storage slot `1`).

## Tsuki-active DogeOS transition

`6281971-19.json` is a genuine block witness from a local DogeOS ScrollReth node. The node was
created by the `real_scroll_reth_withdrawal_readiness` harness. Block 19's first transaction called
`withdrawToL1(address)`, enqueueing the first L2-to-L1 withdrawal message and changing
`nextMessageIndex` from `0` to `1`.

Pinned provenance:

- network: local Chikyū-compatible DogeOS chain, chain ID `6281971` (`0x5fdaf3`)
- block number: `19` (`0x13`)
- block hash: `0x17ce064e49dc6f59353d35487eee042140490b885e73275a69ad23982494ecbc`
- timestamp: `1784102950` (`0x6a574026`)
- parent hash: `0x926d50bb315d18f87b14d853b119e5001c4293611587240c5735c5ce00e712db`
- parent state root: `0x80479f9622ccb9d62a40ee02a217494c17413163ab10596ba189ab6be3901c88`
- post-state root: `0x0824e511633e44abe11ded8c271dbfcecea21263cb37fc84d1835dc379b5724d`
- parent slot-1 value: `0`
- final slot-1 value: `1` (the expected decimal `nextMessageIndex`)
- fixture SHA-256: `9083f9ec8bec979386d2944eb0777b6a8d4395fad4f3ed7906adcee7b0a54866`

Node and dependency pins:

- rollup-node image: `dogeos69/rollup-node:tsuki-5bce327d-reth-39b31f82`
- image digest: `sha256:ed13066066e22bd5c220827b678a6cf59858fbd0d44463f9fcd3ccd97ec76e5b`
- embedded `reth-scroll-cli` revision: `39b31f822cc2b4c54db32ba2f0484ca2a157c3f5`
- DogeOS harness base: `21b907c6818dcbfd6ce0e0efd3de6c2a844106ad`
- SBV collection/replay base: `ec6059bdc48fb60d8340ba86b32bbe8d41111cd4`

The original harness RPC was `http://127.0.0.1:15654`. The retained node database was reopened
locally at `http://127.0.0.1:28545` to collect this file. Both endpoints are loopback-only; no
public Chikyū Tsuki RPC existed at collection time (July 15, 2026).

Collection and independent state checks:

```bash
RPC=http://127.0.0.1:28545

cast chain-id --rpc-url "$RPC"
cast block 19 --rpc-url "$RPC"

cast storage 0x5300000000000000000000000000000000000000 0x1 \
  --block 18 --rpc-url "$RPC"
cast storage 0x5300000000000000000000000000000000000000 0x1 \
  --block 19 --rpc-url "$RPC"

# Independently confirm authenticated queue and NativeDogeToken paths at the parent and final
# states. These calls are checks; the returned nodes were not manually spliced into the fixture.
cast rpc --rpc-url "$RPC" eth_getProof \
  0x5300000000000000000000000000000000000000 '["0x0","0x1"]' 0x12
cast rpc --rpc-url "$RPC" eth_getProof \
  0x5300000000000000000000000000000000000000 '["0x0","0x1"]' 0x13
cast rpc --rpc-url "$RPC" eth_getProof \
  0x530000000000000000000000000000000000d09e '[]' 0x12
cast rpc --rpc-url "$RPC" eth_getProof \
  0x530000000000000000000000000000000000d09e '[]' 0x13

cargo run -p sbv-cli --features scroll -- dump \
  --rpc "$RPC" --block 19 --out-dir /tmp/sbv-tsuki-local-fixture
cargo run -p sbv-cli --features scroll -- run \
  /tmp/sbv-tsuki-local-fixture/19.json
shasum -a 256 /tmp/sbv-tsuki-local-fixture/19.json
```

The raw `sbv-cli dump` output had SHA-256
`d001e172e42212b6865753027716902dadee338242f0bcbb105b22b502628252`; the committed fixture adds
the repository's conventional final newline and otherwise has identical bytes.

At revision `ec6059b`, `sbv-cli dump` does **not** append queue proofs. This fixture needed no
post-processing: ScrollReth's `debug_executionWitness` response already retained the ordinary
execution nodes and code plus the queue and NativeDogeToken paths used during replay. The canonical
parent/header roots were not altered and the witness was not hand-pruned.

## Pre-Tsuki regression fixtures

`20240125.json` is a real Scroll mainnet block witness for block `20240125`. Its real post-block
slot-1 value is `208530`, but Scroll mainnet does not activate Tsuki in the registered chain spec,
so the verifier deliberately returns the pre-Tsuki sentinel `0`.

`14919991-missing-slot1-proof.json` is a real Scroll mainnet EuclidV2 witness from before the
`L2MessageQueue` proof backfill (`git show da8892b^:testdata/scroll/euclidv2/14919991.json`). It has
the slot-0 `messageRoot` path but not the slot-1 path. Verification must succeed without reading
slot 1 and return sentinel `0`; do not backfill this fixture because the missing proof is the point.

The historical `20240125.json` fixture was collected on March 13, 2026 with:

```bash
cargo run -p sbv-cli --features scroll -- dump \
  --rpc https://scroll.api.onfinality.io/public \
  --block 20240125 \
  --out-dir /tmp/sbv-probe-20240125

cast storage 0x5300000000000000000000000000000000000000 0x1 \
  --rpc-url https://scroll.api.pocket.network \
  --block 20240125
```
