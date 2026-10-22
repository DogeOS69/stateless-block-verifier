# Base-fee rule corpora

Three short Tsuki chains, each built by a node that enforces the base-fee rule at import and writes
the L2SystemConfig overhead slot into its execution witnesses. Together they cover every state the
overhead can be in, so the rule is replayed rather than only checked against vectors.

| Directory | Chain | L2SystemConfig | Covers |
| --- | --- | --- | --- |
| `configured-overhead/` | 6281971 | predeployed in genesis, slot unset | authenticated empty slot → default overhead (blocks 1–4); block 4's transaction sets the overhead; blocks 5–7 judged on the configured one |
| `absent-then-created/` | 6281971 | none, until block 4's transfer creates it | authenticated **absent account** in the pre-states of blocks 1–4, the account appearing in block 4's post-state, and an authenticated empty slot in the pre-states of blocks 5–7 — the default overhead throughout |
| `development-chain/` | 1337 | the zero address, as the development chain's own configuration names it | the registered development chain, whose node reads slot 101 of the zero account and finds an authenticated absence |

Every chain also covers, across its blocks: ordinary transfers, a block pushed past the 3M gas
target so the base fee moves up, an L1 message, and an empty block.

`parent.json` is the chain's genesis header, in the shape `eth_getBlockByNumber` returns. The
chain's first chunk is anchored to it, so its hash is the chain's first proof endpoint.

Each block's overhead is read from its **pre-state**, so the block whose transaction changes
something is still judged on the old value and its successor on the new one. That is why the two
tables above describe pre-states: block 4 of `configured-overhead` sets the overhead and is itself
judged on the default, and block 4 of `absent-then-created` creates the account while still being
judged through its absence.

## Provenance

Each directory's `provenance.json` records the node commit, the chain id, the L2SystemConfig
address and the overhead values. `SHA256SUMS` pins the committed bytes, which
`the_fixture_manifest_matches_the_fixtures` checks. The witnesses come from `debug_executionWitness` by way of
`sbv-cli dump`, driven through the Engine API exactly as a sequencer drives one, so they are
produced on the same `newPayload` path the rule is enforced on.

## Reproduction

`generate.sh` is this directory's copy of the node lane's recipe
(`scripts/tsuki-witness-corpus/generate.sh` in dogeos-reth), with three additions: the genesis
document, the port and the way the L2SystemConfig account comes about are all taken from the
environment, and the witnesses are dumped as SBV `BlockWitness` documents rather than raw
`debug_executionWitness` responses.

```bash
reth=/path/to/dogeos-reth            # at the revision in provenance.json
sbv_cli=$(pwd)/target/debug/sbv-cli  # built with --features scroll

for variant in \
  "genesis-present.json predeployed 18645 configured-overhead" \
  "genesis-absent.json  created     18745 absent-then-created" \
  "genesis-dev.json     absent      18845 development-chain"
do
  set -- $variant
  REPO_ROOT="$reth" GENESIS="$PWD/$1" SYSTEM_CONFIG="$2" PORT_BASE="$3" SBV_CLI="$sbv_cli" \
    bash ./generate.sh /tmp/corpus-$4
done
```

A fresh run reproduces the same chain and the same witnesses, but not the same **bytes**:
`debug_executionWitness` returns a block's trie nodes in an unspecified order, so the `states`
array comes back permuted. `SHA256SUMS` therefore pins what is committed here, not what a rerun
produces; compare node sets, not checksums, when reproducing.

The recipe's own chain id is 1122099, which no DogeOS build knows. The guest reads the overhead
from the L2SystemConfig contract **its chain's configuration names**, so a chain it does not know
has no authenticated address to read and is refused. These corpora therefore run on chain ids the
guest does know, with the L2SystemConfig at that chain's own configured address: 6281971's
`0x2669B071…7400`, and the development chain's zero address.

## What is not here

- No block withdraws to L1, so the authenticated L2 message queue index stays zero. A non-zero
  index on real data still needs a Tsuki chain that withdraws, which is tracked outside this
  change; `l2_message_queue_info`'s own tests cover non-zero and out-of-range indices directly.
  `test_next_message_index_post_tsuki_transition` stays `#[ignore]`d until such a chain exists,
  and goes with the lineage under `../next-message-index/` whenever that happens. That lineage
  itself stays: several live tests use its blocks as real-chain material rather than as a
  verifiable chunk (`tsuki_witness_11`, `tsuki_lineage`, the compression-info tests,
  `the_recorded_lineages_follow_the_rule`).
- The L1 message in block 3 bumps its sender's account nonce, so the ordinary transaction the
  recipe pairs with it is dropped as a stale nonce. Block 3 is the L1 message alone.
