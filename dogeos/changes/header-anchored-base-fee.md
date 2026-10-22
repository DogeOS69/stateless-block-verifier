# Header-anchored Endpoints: Base-fee Rule and Trusted Parent Overlay

## Summary

This overlay gives `verifier::run` a trusted parent header and makes the verifier
enforce the protocol rules that a state-root check alone cannot see: the DogeOS
header rules, the L2 base-fee rule, the header's gas accounting, and the body
commitments.

Proof endpoints become L2 **block hashes** rather than state roots. The guest
therefore has to open the parent header of a chunk's first block, and the goal is
that everything the block hash commits to is either checked against what the
block executed or checked against the rule that fixes it. What remains
unchecked is named under "Residue" below.

## Scope

- `verifier::run` and `verifier::run_host` take `parent: Option<&Header>`, the
  trusted header of the block before the first witness. When it is supplied, the
  verifier checks that the first block is at `parent.number + 1`, that its
  `parent_hash` is `keccak(rlp(parent))`, and that the witness's
  `prev_state_root` is `parent.state_root`.
- The parent is **required** whenever the chain spec schedules Tsuki at all,
  however far in the future. The requirement deliberately does not look at the
  witnesses' own timestamps: those are prover data, and a chunk could otherwise
  backdate its first block below the activation and opt out of its own anchor.
  For the same reason the requirement covers a chunk that spans the transition.
  A chain that never activates Tsuki keeps its historical behaviour and may pass
  `None`.
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
  previous one's header, and takes `--parent-header <path>` for the first.

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
- The parent is an argument rather than a new witness field, so the witness
  encoding is unchanged and the caller that already knows the previous proof
  endpoint is the one that vouches for the header.

## Test coverage

- `verifier::scroll::base_fee_rule_tests` mirrors the vectors in dogeos-reth's
  own `base_fee.rs` (mainnet parameters, default overhead, configured overhead,
  cap, saturation) and adds the witness-specific cases: authenticated empty
  slot, authenticated absent account, and an unresolved account node, which must
  abort. `the_recorded_lineages_follow_the_rule` replays the base fees the
  recorded Tsuki witnesses actually carry.
- `verifier::scroll::anchor_tests` runs the anchoring, header, gas and body
  rules through `run_host` over five consecutive real mainnet blocks: a chunk
  anchored to its parent, two chunks meeting at a boundary block, a skipped
  block, a repeated block, a substituted parent header, a pre-state root that is
  not the parent's, a gas-limit jump and a backwards timestamp (first block
  against the trusted parent, mid-chunk, and last block), a `gas_used`
  misreported in either direction, each body commitment with the failure it has
  to produce, and each header rule. Two tests pin the anchoring gate itself: a
  chain that schedules Tsuki in the future still requires the parent even for a
  backdated chunk, and a chain that never activates Tsuki does not.
- `chainspec::tests` pins the chain-id lookup and that mainnet's derived genesis
  header hashes to the published mainnet genesis hash, which is what lets a
  chain's first chunk be anchored to its genesis header. Chikyū is recorded as
  the exception: its published genesis hash is stored in the specification, not
  derived from its genesis document.

## Known gap: the recorded Tsuki corpus

The Tsuki witnesses under `testdata/dogeos/next-message-index/` cannot satisfy
the base-fee rule, for two independent reasons:

- they carry no trie node that resolves the L2SystemConfig account, at any of
  the candidate addresses, so the overhead cannot be read at all;
- their producing node predates the Tsuki EIP-1559 parameters (denominator 48,
  elasticity 10). Their recorded base fees follow the rule exactly under the
  Feynman parameters (denominator 8, elasticity 2) with the default overhead,
  which is what `the_recorded_lineages_follow_the_rule` asserts.

`test_next_message_index_tsuki_empty_queue_witnesses` and
`test_next_message_index_post_tsuki_transition` are therefore `#[ignore]`d, and
`a_tsuki_chunk_without_an_l2_system_config_proof_fails_the_proof` pins the
failure. Un-ignoring them needs **three** things, not one: a corpus carrying the
slot-101 proof, a corpus produced under the Tsuki fee parameters, and a trusted
parent header at each call site. The first test already passes its chunk's real
parent (block 11 anchors blocks 12 and 13); the second is a single block, so it
also needs the header of block 18 as
`testdata/dogeos/next-message-index/6281971-18-header.json`. Each `#[ignore]`
reason names the exact thing it is waiting for.

Until then the Tsuki configuration this overlay ships — parent required, header
rules rooted in the trusted parent, base fee enforced per block — is covered by
vectors rather than by a replay: treat the corpus regeneration as a merge gate,
not a note. `l2_message_queue_info`'s Tsuki branch, which lost its only replay
with those tests, is driven directly in `base_fee_rule_tests` instead.

## Upkeep notes

- Rebase this branch onto `dogeos/main`, then rebase `dogeos/main` onto
  `master`.
- If the chain configuration ever grows a second overhead source, change
  `BaseFeeDatabase`'s documentation with it: the account-before-slot order it
  establishes is a property of `SparseState`, not of the base-fee rule.
