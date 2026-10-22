#!/usr/bin/env bash
# Produces a multi-block Tsuki chain on a throwaway DogeOS chain and dumps one
# `debug_executionWitness` per block, for regenerating the guest's Tsuki fixtures.
#
# The chain runs the Tsuki EIP-1559 parameters (denominator 48, elasticity 10) from genesis and
# covers what the base-fee rule needs to be exercised end to end:
#
#   blocks 1-3  the system config contract does not exist yet, so the rule runs on the default
#               overhead and its witnesses have to prove the account's absence: ordinary
#               transfers, a block filled past the gas target so the base fee moves, and an L1
#               message alongside ordinary transactions
#   block 4     deploys the system config contract at the chain's configured address
#   block 5     a transaction that sets the L2 base-fee overhead at slot 101
#   blocks 6-7  the first blocks judged by the new overhead
#
# Blocks are driven through the Engine API exactly as the rollup node drives a sequencer, so the
# witnesses come from the same `newPayload` path the base-fee rule is enforced on.
#
# Usage: scripts/tsuki-witness-corpus/generate.sh [OUT_DIR]
# Requires: cast (foundry), jq, and a dogeos-reth binary (built here if absent).

set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# The dogeos-reth worktree that supplies the node. Overridable so this copy of the recipe can
# live outside it.
repo_root="${REPO_ROOT:-$(cd -- "$here/../.." && pwd)}"
out_dir="${1:-$repo_root/target/tsuki-witness-corpus}"
work_dir="$out_dir/node"
genesis="${GENESIS:-$here/genesis.json}"
# How the L2SystemConfig account comes about: `deployed` by a block (the original recipe),
# `predeployed` in the genesis document, `created` mid-chain by a plain transfer to it (which
# gives the account but no code, so the slot stays authenticated-empty), or `absent` for a chain
# that never has one.
system_config_mode="${SYSTEM_CONFIG:-deployed}"
predeployed=0
[[ "$system_config_mode" == "deployed" ]] || predeployed=1
case "$system_config_mode" in absent | created) OVERHEAD_BLOCKS=0 ;; esac
# Set to 0 to stop after the blocks that run before any overhead is configured.
overhead_blocks="${OVERHEAD_BLOCKS:-1}"
port_base="${PORT_BASE:-18545}"

rpc="http://127.0.0.1:$port_base"
authrpc="http://127.0.0.1:$(( port_base + 6 ))"
jwt="$work_dir/jwt.hex"

# The first well-known development key; funded in genesis.json.
sender=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
sender_key=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
recipient=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
# `JUMPDEST; PUSH1 0; JUMP` in genesis.json: burns exactly the gas it is given and runs out, so a
# block can be pushed past the 3M gas target without 3M gas worth of calldata.
gas_burner=0x00000000000000000000000000000000000B0121
system_config="$(jq -r '.config.scroll.l1Config.l2SystemConfigAddress' "$genesis")"
new_overhead=200000000

for tool in cast jq; do
  command -v "$tool" >/dev/null || { echo "missing required tool: $tool" >&2; exit 1; }
done

binary="$repo_root/target/debug/dogeos-reth"
if [[ ! -x "$binary" ]]; then
  echo "building dogeos-reth"
  (cd "$repo_root" && cargo build -p dogeos-reth --jobs 8)
fi
node_commit="$(git -C "$repo_root" rev-parse HEAD)"

rm -rf "$out_dir"
mkdir -p "$work_dir"
openssl rand -hex 32 > "$jwt"

"$binary" node \
  --chain "$genesis" \
  --datadir "$work_dir/data" \
  --http --http.addr 127.0.0.1 --http.port "$port_base" --http.api eth,debug,net,web3 \
  --authrpc.addr 127.0.0.1 --authrpc.port "$(( port_base + 6 ))" --authrpc.jwtsecret "$jwt" \
  --disable-discovery --port "$(( port_base - 242 ))" --network.scroll-wire=false \
  --rpc.eth-proof-window 1024 \
  --log.file.directory "$work_dir/logs" \
  > "$work_dir/node.log" 2>&1 &
node_pid=$!
trap 'kill "$node_pid" 2>/dev/null || true; wait "$node_pid" 2>/dev/null || true' EXIT
echo "node pid $node_pid, log $work_dir/node.log"

for _ in $(seq 60); do
  if cast chain-id --rpc-url "$rpc" >/dev/null 2>&1; then break; fi
  sleep 1
done
cast chain-id --rpc-url "$rpc" >/dev/null || { echo "node did not come up" >&2; exit 1; }

engine() { cast rpc --rpc-url "$authrpc" --jwt-secret "$(cat "$jwt")" "$@"; }

head_hash="$(cast block 0 --rpc-url "$rpc" --json | jq -r .hash)"
nonce=0
timestamp="$(( $(cast block 0 --rpc-url "$rpc" --json | jq -r .timestamp | cast to-dec) ))"

# Signs a legacy transaction offline; the blocks are driven by the Engine API, so nothing goes
# through the transaction pool and every block's contents are exactly what we list.
sign_tx() { # to value gas [sig arg]
  local to="$1" value="$2" gas="$3"
  shift 3
  cast mktx --rpc-url "$rpc" --private-key "$sender_key" \
    --legacy --gas-price 2000000000 --gas-limit "$gas" --nonce "$nonce" \
    --value "$value" "$to" "$@"
}

# An L1 message (type 0x7e) is not signed and never passes through the pool, so it is encoded
# directly. See scripts/tsuki-witness-corpus/l1_message.py for the field order.
l1_message() { # queue_index gas to value
  python3 "$here/l1_message.py" "$1" "$2" "$3" "$4" 0x "$sender"
}

build_block() { # label forced_txs_json
  local label="$1" forced="$2"
  timestamp=$(( timestamp + 1 ))
  local fcs attrs response payload_id payload status
  fcs="{\"headBlockHash\":\"$head_hash\",\"safeBlockHash\":\"$head_hash\",\"finalizedBlockHash\":\"$head_hash\"}"
  attrs="$(jq -nc --argjson txs "$forced" --arg ts "$(cast to-hex "$timestamp")" '{
      payloadAttributes: {
        timestamp: $ts,
        prevRandao: "0x0000000000000000000000000000000000000000000000000000000000000000",
        suggestedFeeRecipient: "0x0000000000000000000000000000000000000000"
      },
      transactions: $txs,
      noTxPool: true,
      blockDataHint: {},
      gasLimit: null
    }')"
  response="$(engine engine_forkchoiceUpdatedV1 "$fcs" "$attrs")"
  payload_id="$(jq -r .payloadId <<<"$response")"
  [[ "$payload_id" != "null" ]] || { echo "no payload id for $label: $response" >&2; exit 1; }
  payload="$(engine engine_getPayloadV1 "$payload_id")"
  status="$(engine engine_newPayloadV1 "$payload" | jq -r .status)"
  [[ "$status" == "VALID" ]] || { echo "$label rejected at newPayload: $status" >&2; exit 1; }
  head_hash="$(jq -r .blockHash <<<"$payload")"
  engine engine_forkchoiceUpdatedV1 \
    "{\"headBlockHash\":\"$head_hash\",\"safeBlockHash\":\"$head_hash\",\"finalizedBlockHash\":\"$head_hash\"}" \
    null > /dev/null
  echo "$label: block $(jq -r .blockNumber <<<"$payload" | cast to-dec)" \
       "base fee $(jq -r .baseFeePerGas <<<"$payload" | cast to-dec)" \
       "gas used $(jq -r .gasUsed <<<"$payload" | cast to-dec)"
}

# 1: ordinary transfers.
txs="[]"
for _ in 1 2 3; do
  txs="$(jq -c --arg tx "$(sign_tx "$recipient" 1000000000000000 21000)" '. + [$tx]' <<<"$txs")"
  nonce=$(( nonce + 1 ))
done
build_block "ordinary transfers" "$txs"

# 2: past the 3M gas target, so the base fee rises. Each call runs out of gas on purpose; the
# receipts are failures and the gas is spent, which is what moves the fee.
txs="[]"
for _ in 1 2 3; do
  txs="$(jq -c --arg tx "$(sign_tx "$gas_burner" 0 1400000)" '. + [$tx]' <<<"$txs")"
  nonce=$(( nonce + 1 ))
done
build_block "busy block" "$txs"

# 3: an L1 message in front of ordinary transactions.
txs="$(jq -nc --arg m "$(l1_message 0 0x186a0 "$recipient" 0)" '[$m]')"
txs="$(jq -c --arg tx "$(sign_tx "$recipient" 1000000000000 21000)" '. + [$tx]' <<<"$txs")"
nonce=$(( nonce + 1 ))
build_block "L1 message" "$txs"

# 4: deploy the system config contract at the chain's configured address, unless genesis already
# carries it. Its runtime code is `SSTORE(101, calldataload(4)); STOP`, a stand-in for the real
# overhead setter; the deploy address is the sender's CREATE address at this nonce.
if [[ "$predeployed" == "0" ]]; then
  [[ "$nonce" == "7" ]] || { echo "the deploy nonce moved; repin l2SystemConfigAddress" >&2; exit 1; }
  deployed="$(cast compute-address "$sender" --nonce "$nonce" | awk '{print $NF}')"
  [[ "${deployed,,}" == "${system_config,,}" ]] \
    || { echo "deploy address $deployed is not the configured $system_config" >&2; exit 1; }
  txs="$(jq -nc --arg tx "$(cast mktx --rpc-url "$rpc" --private-key "$sender_key" \
    --legacy --gas-price 2000000000 --gas-limit 200000 --nonce "$nonce" \
    --create 0x666004356065550060005260076019f3)" '[$tx]')"
  nonce=$(( nonce + 1 ))
  build_block "system config deployed" "$txs"
fi
if [[ "$system_config_mode" == "absent" || "$system_config_mode" == "created" ]]; then
  [[ "$(cast code "$system_config" --rpc-url "$rpc")" == "0x" ]] \
    || { echo "the system config contract was expected to be absent" >&2; exit 1; }
  if [[ "$system_config_mode" == "created" ]]; then
    # The account itself appears mid-chain, as it will on any chain whose genesis does not carry
    # it: a plain transfer creates it with no code, so slot 101 goes from an authenticated absent
    # account to an authenticated empty slot and the default overhead applies throughout.
    [[ "$(cast balance "$system_config" --rpc-url "$rpc")" == "0" ]] \
      || { echo "the system config account already exists" >&2; exit 1; }
    txs="$(jq -nc --arg tx "$(sign_tx "$system_config" 1000000000000 21000)" '[$tx]')"
    nonce=$(( nonce + 1 ))
    build_block "system config account created" "$txs"
    [[ "$(cast balance "$system_config" --rpc-url "$rpc")" != "0" ]] \
      || { echo "the system config account was not created" >&2; exit 1; }
    for label in "first block with the account present" "second block with the account present"; do
      txs="$(jq -nc --arg tx "$(sign_tx "$recipient" 1000000000000 21000)" '[$tx]')"
      nonce=$(( nonce + 1 ))
      build_block "$label" "$txs"
    done
  fi
else
  [[ "$(cast code "$system_config" --rpc-url "$rpc")" == "0x60043560655500" ]] \
    || { echo "the system config contract is not in place" >&2; exit 1; }
fi

# 5: set the overhead at system config slot 101, then 6 and 7 on the new overhead.
if [[ "$overhead_blocks" != "0" ]]; then
  txs="$(jq -nc --arg tx "$(sign_tx "$system_config" 0 200000 \
    'setL2BaseFeeOverhead(uint256)' "$new_overhead")" '[$tx]')"
  nonce=$(( nonce + 1 ))
  build_block "overhead change" "$txs"
  [[ "$(cast storage "$system_config" 101 --rpc-url "$rpc" | cast to-dec)" == "$new_overhead" ]] \
    || { echo "the overhead was not set" >&2; exit 1; }

  for label in "first block on the new overhead" "second block on the new overhead"; do
    txs="$(jq -nc --arg tx "$(sign_tx "$recipient" 1000000000000 21000)" '[$tx]')"
    nonce=$(( nonce + 1 ))
    build_block "$label" "$txs"
  done
fi

# The import rules the witnesses depend on, checked against the running node.
#
# A block whose base fee does not follow the rule must be answered INVALID rather than taken, and
# the node must still be there afterwards. The parent here has been `newPayload`-ed but not yet
# made canonical by a forkchoice update, so this also covers an in-memory parent.
echo "checking that a wrong base fee is rejected at newPayload"
timestamp=$(( timestamp + 1 ))
fcs="{\"headBlockHash\":\"$head_hash\",\"safeBlockHash\":\"$head_hash\",\"finalizedBlockHash\":\"$head_hash\"}"
attrs="$(jq -nc --arg ts "$(cast to-hex "$timestamp")" '{
    payloadAttributes: {
      timestamp: $ts,
      prevRandao: "0x0000000000000000000000000000000000000000000000000000000000000000",
      suggestedFeeRecipient: "0x0000000000000000000000000000000000000000"
    },
    transactions: [], noTxPool: true, blockDataHint: {}, gasLimit: null
  }')"
payload_id="$(engine engine_forkchoiceUpdatedV1 "$fcs" "$attrs" | jq -r .payloadId)"
good="$(engine engine_getPayloadV1 "$payload_id")"

# Same payload, one wei off the rule's output. The block hash is recomputed so the payload is
# well formed and only the fee is wrong.
bad="$(jq -c --arg h "$(cast to-hex "$(( $(jq -r .baseFeePerGas <<<"$good" | cast to-dec) + 1 ))")" \
  '.baseFeePerGas = $h' <<<"$good")"
status="$(engine engine_newPayloadV1 "$bad" | jq -r .status)"
[[ "$status" == "INVALID" ]] || { echo "a wrong base fee was answered $status, expected INVALID" >&2; exit 1; }

# ... and the node is still alive and still takes the correct payload.
status="$(engine engine_newPayloadV1 "$good" | jq -r .status)"
[[ "$status" == "VALID" ]] || { echo "the correct payload was answered $status after the rejection" >&2; exit 1; }
head_hash="$(jq -r .blockHash <<<"$good")"
engine engine_forkchoiceUpdatedV1 \
  "{\"headBlockHash\":\"$head_hash\",\"safeBlockHash\":\"$head_hash\",\"finalizedBlockHash\":\"$head_hash\"}" \
  null > /dev/null

# Building a block is not validating one: the pending block and a simulation with the fee rule
# turned off must both still work.
echo "checking that the pending block and eth_simulateV1 still build"
cast block pending --rpc-url "$rpc" --json | jq -e '.number != null' > /dev/null \
  || { echo "the pending block does not build" >&2; exit 1; }
cast rpc --rpc-url "$rpc" eth_simulateV1 \
  "$(jq -nc --arg from "$sender" --arg to "$recipient" '{
        blockStateCalls: [ { calls: [ { from: $from, to: $to, value: "0x1" } ] } ],
        validation: false
      }')" latest | jq -e 'length > 0' > /dev/null \
  || { echo "eth_simulateV1 with validation:false does not run" >&2; exit 1; }

head_number="$(cast block-number --rpc-url "$rpc")"
# Genesis has no parent, so it has no witness; the corpus starts at block 1, which is itself in
# the segment where the system config account does not exist yet.
echo "dumping witnesses for blocks 1..$head_number"
for n in $(seq 1 "$head_number"); do
  cast rpc --rpc-url "$rpc" debug_executionWitness "$(cast to-hex "$n")" \
    | jq . > "$out_dir/$n.json"
done

# The guest reads the overhead from slot 101 of the system config account in each block's
# pre-state, and must be able to tell an authenticated absence from a missing witness node. So
# every witness has to carry the account's path in the state trie, whether or not the account is
# there. Check it against the node's own proof for the parent state.
check_witness_covers_system_config() { # block
  local block="$1" parent proof missing=0 node
  parent="$(cast to-hex "$(( block - 1 ))")"
  proof="$(cast rpc --rpc-url "$rpc" eth_getProof "$system_config" \
    '["0x65"]' "$parent")"
  while read -r node; do
    jq -e --arg n "$node" 'any(.state[]; ascii_downcase == ($n | ascii_downcase))' \
      "$out_dir/$block.json" > /dev/null || { echo "  missing trie node $node"; missing=1; }
  done < <(jq -r '(.accountProof + ((.storageProof // [])[].proof // []))[]' <<<"$proof")
  [[ "$missing" == "0" ]] \
    || { echo "witness for block $block does not cover the system config path" >&2; exit 1; }
  echo "  block $block: witness covers the system config account path"
}
echo "checking that every witness carries the system config path"
for block in $(seq 1 "$head_number"); do
  check_witness_covers_system_config "$block"
done

# The guest's fixtures are SBV `BlockWitness` documents, not raw `debug_executionWitness`
# responses, so dump them from the same node while it is still up. `sbv-cli` ranges are
# end-exclusive.
[[ -x "$SBV_CLI" ]] || { echo "set SBV_CLI to an sbv-cli built with --features scroll" >&2; exit 1; }
mkdir -p "$out_dir/witnesses"
"$SBV_CLI" dump --rpc "$rpc" --block "1..$(( head_number + 1 ))" --out-dir "$out_dir/witnesses"
# Block 1's parent is the genesis block, which has no witness of its own. The chain's first chunk
# is anchored to its header, so keep it beside the witnesses.
cast block 0 --rpc-url "$rpc" --json > "$out_dir/witnesses/parent.json"

jq -n \
  --arg commit "$node_commit" \
  --arg chain_id "$(cast chain-id --rpc-url "$rpc")" \
  --arg genesis "$(basename "$genesis")" \
  --arg system_config "$system_config" \
  --arg head "$head_number" \
  --arg system_config_mode "$system_config_mode" \
  --arg overhead_blocks "$overhead_blocks" \
  --argjson overhead_before 15680000 \
  --argjson overhead_after "$new_overhead" \
  '{
     generator: "scripts/tsuki-witness-corpus/generate.sh (hae-guest variant)",
     dogeos_reth_commit: $commit,
     chain_id: $chain_id,
     genesis: $genesis,
     l2_system_config_address: $system_config,
     system_config: $system_config_mode,
     overhead_blocks: ($overhead_blocks != "0"),
     base_fee_params: { max_change_denominator: 48, elasticity_multiplier: 10 },
     overhead_slot: 101,
     overhead_before_change: $overhead_before,
     overhead_after_change: $overhead_after,
     blocks: $head
   }' > "$out_dir/provenance.json"

( cd "$out_dir" && sha256sum ./*.json witnesses/*.json > SHA256SUMS )
echo "witnesses and provenance in $out_dir"
