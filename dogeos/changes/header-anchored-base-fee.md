# Header-anchored Endpoints: Base-fee Rule and Trusted Parent Overlay

## Summary

This overlay gives `verifier::VerifyContext` a trusted parent header and makes the verifier
enforce the protocol rules that a state-root check alone cannot see: the DogeOS
header rules, the L2 base-fee rule, the header's gas accounting, and the body
commitments.

Proof endpoints become L2 **block hashes** rather than state roots. The guest
therefore has to open the parent header of a chunk's first block, and the goal is
that everything the block hash commits to is either checked against what the
block executed or checked against the rule that fixes it. What remains
unchecked is named under "Residue" below.

## Scope

- `verifier::VerifyContext` groups `witnesses`, an owned `parent_header: Header`,
  `chain_spec`, and `compression_infos`. Its `verify(self)` method consumes the
  context and destructures it at entry. `verifier::run_host` computes compression
  information and delegates to that method, also taking an owned parent header.
- The trusted parent is **required on every fork**, including chains that never
  activate Tsuki. The verifier checks that the first block is at
  `parent_header.number + 1`, that its `parent_hash` is
  `keccak(rlp(parent_header))`, and that the witness's `prev_state_root` is
  `parent_header.state_root`. No timestamp or fork schedule can bypass this anchor.
- Per block, every fork: the shared `DogeosConsensus` header rules.
  `validate_header` covers the fields the block hash commits to and nothing
  reads during execution (difficulty, mix hash, coinbase, nonce, extra data, the
  blob and withdrawals fields), the base-fee cap, and `gas_used <= gas_limit`.
  `validate_header_against_parent` covers the parent link, the height, a
  timestamp that does not move backwards, and the gas-limit bound — for each
  consecutive pair, starting from the trusted parent. This is the **only** place
  these rules run: callers that fold the result into public inputs rely on it and
  must not repeat it. Two of the fields feed back into the base-fee rule, so both
  have to be checked on every block and not only on the trusted parent:
  `gas_limit` is one of the calculator's inputs, and the timestamp decides which
  EIP-1559 parameters apply.
- Per block, from Tsuki onwards: `header.base_fee_per_gas` must equal
  `ScrollBaseFeeProvider::next_block_base_fee(pre-state, parent, timestamp)`,
  the same calculator the execution client uses. The parent is the trusted
  parent for the first block, and the previous block otherwise.
- Per block, every fork: `header.gas_used` must equal the gas the block spent.
- Per block, every fork: the shared `DogeosConsensus` body checks run before
  execution (transactions root, ommers, withdrawals, L1 message order) and
  after it (gas used, receipts root, logs bloom).
- `build_chain_spec_force_hardfork` starts from the specification of the chain
  being proven instead of always from the development chain, so the
  L2SystemConfig address and the EIP-1559 parameters match that chain. It
  refuses to build for a chain whose genesis declares `tsukiFeeParamsTime`: the
  execution client reads the fee schedule from the genesis document, so a forced
  schedule would either ignore the declaration — diverging from the node for
  exactly the chains that make one — or override the fork the caller asked for.
- `get_chain_spec` resolves each registered chain by the chain id its own
  genesis document declares. It previously mapped DogeOS mainnet to chain id
  `0xff`, which the frozen mainnet genesis document does not use.
- `sbv-cli run` replays its witnesses in block order, anchoring each one to the
  previous one's header, and requires `--parent-header <path>` for the first on
  every fork. All witnesses must form one consecutive sequence on the same chain;
  gaps, duplicates, and mixed chains are rejected.

## The overhead read

The overhead lives in slot `101` of the L2SystemConfig contract named by the
chain's own `ScrollChainConfig`, read from each block's **pre-state**. A
transaction that changes it therefore only moves the base fee from the next
block onwards, including when that next block opens the next chunk.

The read goes through `BaseFeeDatabase`, not the raw trie: `SparseState::storage`
assumes its caller loaded the account first, both to build the account's storage
trie and because an account that is not in the state has no storage trie at all.

Three cases, deliberately distinct:

- the slot is authenticated empty, or the account is authenticated absent: the
  overhead was never configured, so the calculator's default applies;
- the witness does not resolve the account or the slot: the sparse trie aborts
  on unresolved node access and the proof fails. It must never fall back to the
  default, because the prover chooses what the witness contains;
- the chain is **not registered** in this build: its configuration is the
  development blueprint's, whose L2SystemConfig address is the zero address.
  The zero account is normally an authenticated absence, which would read as the
  default overhead while nothing had been verified, so an unregistered chain is
  refused rather than defaulted. A *registered* chain whose own configuration
  names the zero address — the development chain does — is proven as it is,
  because its node reads that same account and finds that same absence.

## Residue: what the block hash commits to and the verifier does not check

The header rules fix every constant field, the cap and the gas and timestamp
bounds, so what remains is the fields that are free by design and are pinned
elsewhere: `state_root` (checked against execution), `transactions_root`,
`receipts_root` and `logs_bloom` (checked against the body and the receipts),
`gas_used` and `base_fee_per_gas` (checked by their own rules), `parent_hash`
and `number` (checked against the parent). A timestamp that moves *forward*
faster than real time is admissible: the guest has no wall clock, and the host
deliberately skips upstream's future-block check
(`#[cfg(not(target_os = "zkvm"))]`) so that the two agree and a replay does not
depend on the machine's clock. Admitting a future-dated block is a node's
concern, not a verifier's. That check returns early, so the whole validator is
re-run against a copy whose timestamp cannot be in the future rather than
dropping its error: the rules behind it — the gas rules among them — still
apply, which `the_gas_rules_survive_a_future_dated_header` pins.

## Why this stays maintainable

- The rules are the execution client's own: the base fee comes from
  `dogeos-reth-evm`'s calculator, and the header and body checks from
  `dogeos-reth-consensus`. The overlay wires them in; it does not restate them.
- The parent is a required context field rather than a new witness field, so the witness
  encoding is unchanged and the caller that already knows the previous proof
  endpoint is the one that vouches for the header.

## Test coverage

- `verifier::base_fee_rule_tests` mirrors the vectors in dogeos-reth's
  own `base_fee.rs` (mainnet parameters, default overhead, configured overhead,
  cap, saturation) and adds the witness-specific cases: authenticated empty
  slot, authenticated absent account, and an unresolved account node, which must
  abort. `the_recorded_lineages_follow_the_rule` replays the base fees the
  recorded Tsuki witnesses actually carry.
- `verifier::anchor_tests` runs the anchoring, header, gas and body
  rules through `run_host` over five consecutive recorded Tsuki blocks: a chunk
  anchored to its parent, two chunks meeting at a boundary block, a skipped
  block, a repeated block, a substituted parent header, a pre-state root that is
  not the parent's, a gas-limit jump and a backwards timestamp (first block
  against the trusted parent, mid-chunk, and last block), a `gas_used`
  misreported in either direction, each body commitment with the failure it has
  to produce, and each header rule. A substituted parent is also rejected before
  a scheduled Tsuki activation.
- A guest-feature test calls `VerifyContext::verify` with an owned parent and
  supplied compression information. The CLI rejects a missing `--parent-header`
  even with a forced pre-Tsuki fork. The two historical Feynman replay cases and
  the never-Tsuki replay test are ignored: their witnesses lack slot-101 proofs,
  which the pinned executor now reads on every fork. General anchor, body, gas
  and compression tests use the complete Tsuki corpus.
- `chainspec::tests` pins the chain-id lookup and verifies that both mainnet's
  and Chikyū's derived genesis headers hash to their published genesis hashes,
  so each chain's first chunk can be anchored to its genesis header.

## Witness producer compatibility

The dependency pins in the root README include dogeos-reth #39 and reth #9. The node validates
base fees through `ConfigureEvm::validate_block_pre_execution`; the block executor also reads
slot 101 before transitions or transactions so `debug_executionWitness` records its proof.
SBV keeps its explicit pre-state base-fee check for deterministic guest execution. Older witness
files are not completed by a dependency update and must be regenerated or supplied with valid
proof nodes; an unresolved account or slot still fails verification.

## The regenerated corpora

`testdata/dogeos/base-fee-rule/` holds three short Tsuki chains built by a node that enforces the
rule at import and writes the overhead slot into its execution witnesses. They replay through
`VerifyContext::verify` with every rule enforced, which is what turns the rule from a set of vectors into a replay:

- `configured-overhead/` — the contract is a genesis predeploy with its slot unset, block 4's
  transaction sets the overhead, and blocks 5–7 are judged on it. This is the corpus behind the
  run-level ordering test: block 4 follows the overhead *its own pre-state* holds and block 5
  follows the new one, as one chunk and as two chunks meeting at the boundary, with the successor
  carrying the stale fee rejected in both shapes.
- `absent-then-created/` — the account does not exist until block 4 creates it with a plain
  transfer: an authenticated absent account, then the account appearing mid-chain, then an
  authenticated empty slot. This is the shape of any chain whose genesis does not carry the
  contract, mainnet's included.
- `development-chain/` — the registered development chain, whose configuration names the zero
  address. Its node reads slot 101 of that account and finds an authenticated absence, and the
  verifier agrees rather than refusing the chain.

The corpora run on chain ids this build knows, with the L2SystemConfig at each chain's own
configured address. The node lane's recipe uses chain id 1122099, which no DogeOS build knows: the
guest reads the overhead from the contract **its chain's configuration names**, so a chain it does
not know has no authenticated address and is refused. `testdata/dogeos/base-fee-rule/README.md`
has the reproduction, and keeps this directory's copy of the recipe beside the genesis documents.

## Still missing: a non-zero message queue index

No block in these corpora withdraws to L1, so the authenticated L2 message queue index stays zero
— read, not assumed, which `the_queue_index_is_read_from_the_regenerated_corpora` pins. A non-zero
index was only ever covered by `test_next_message_index_post_tsuki_transition`, whose lineage
predates both the slot-101 proof and the Tsuki EIP-1559 parameters; it stays `#[ignore]`d until a
Tsuki chain that withdraws exists, together with the header of the block before its withdrawal
block. `test_next_message_index_tsuki_empty_queue_witnesses` is superseded by the corpora above
and stays as the record of what its lineage cannot satisfy.

## Upkeep notes

- If the chain configuration ever grows a second overhead source, change
  `BaseFeeDatabase`'s documentation with it: the account-before-slot order it
  establishes is a property of `SparseState`, not of the base-fee rule.
