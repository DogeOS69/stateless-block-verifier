# Next Message Index Fixture

`20240125.json` is a Scroll mainnet block witness for block `20240125`.

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

Legacy fixtures under `testdata/scroll/` were collected before DogeOS started reading
`L2MessageQueue` after execution, so they do not contain the extra proof nodes needed
for `messageRoot` / `nextMessageIndex` unless the block touched that contract.
