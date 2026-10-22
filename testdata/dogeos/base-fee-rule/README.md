# Base-fee rule corpora

Three short Tsuki chains, each built by a node that enforces the base-fee rule at import and writes
the L2SystemConfig overhead slot into its execution witnesses. Together they cover every state the
overhead can be in, so the rule is replayed rather than only checked against vectors.

| Directory | Chain | L2SystemConfig | Covers |
| --- | --- | --- | --- |
| `configured-overhead/` | 6281971 | predeployed in genesis, slot unset | authenticated empty slot → default overhead (blocks 1–4); block 4's transaction sets the overhead; blocks 5–7 judged on the configured one |
| `absent-then-created/` | 6281971 | none, until block 4 creates it with a plain transfer | authenticated **absent account** → default overhead (blocks 1–3); the account appearing mid-chain; authenticated empty slot afterwards (blocks 4–7) |
| `development-chain/` | 1337 | the zero address, as the development chain's own configuration names it | the registered development chain, whose node reads slot 101 of the zero account and finds an authenticated absence |

Every chain also covers, across its blocks: ordinary transfers, a block pushed past the 3M gas
target so the base fee moves up, an L1 message, and an empty block.

`parent.json` is the chain's genesis header, in the shape `eth_getBlockByNumber` returns. The
chain's first chunk is anchored to it, so its hash is the chain's first proof endpoint.

## Provenance

Each directory's `provenance.json` records the node commit, the chain id, the L2SystemConfig
address and the overhead values. The witnesses come from `debug_executionWitness` by way of
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

The recipe's own chain id is 1122099, which no DogeOS build knows. The guest reads the overhead
from the L2SystemConfig contract **its chain's configuration names**, so a chain it does not know
has no authenticated address to read and is refused. These corpora therefore run on chain ids the
guest does know, with the L2SystemConfig at that chain's own configured address: 6281971's
`0x2669B071…7400`, and the development chain's zero address.

## What is not here

- No block withdraws to L1, so the authenticated L2 message queue index stays zero. A non-zero
  index still needs a chain that withdraws; see the ignored `next_message_index` test.
- The L1 message in block 3 bumps its sender's account nonce, so the ordinary transaction the
  recipe pairs with it is dropped as a stale nonce. Block 3 is the L1 message alone.
