# Header-anchored Endpoints: Base-fee Rule and Trusted Parent Overlay

## Summary

This overlay gives `verifier::run` a trusted parent header and makes the verifier
enforce the protocol rules that a state-root check alone cannot see: the L2
base-fee rule, the header's gas accounting, and the body commitments.

Proof endpoints become L2 **block hashes** rather than state roots, so the guest
has to open the parent header of a chunk's first block, and everything the block
hash commits to has to be checked against what the block actually executed.

## Scope

- `verifier::run` and `verifier::run_host` take `parent: Option<&Header>`, the
  trusted header of the block before the first witness. When it is supplied, the
  verifier checks that the first block is at `parent.number + 1`, that its
  `parent_hash` is `keccak(rlp(parent))`, and that the witness's
  `prev_state_root` is `parent.state_root`.
- The parent is **required** once Tsuki is active at the first block's
  timestamp. Earlier forks keep their historical behaviour and may pass `None`.
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
  L2SystemConfig address and the EIP-1559 parameters match that chain.
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
  default, because the prover chooses what the witness contains.

## Why this stays maintainable

- The rules are the execution client's own: the base fee comes from
  `dogeos-reth-evm`'s calculator and the body checks from `dogeos-reth-consensus`.
  The overlay wires them in; it does not restate them.
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
- `verifier::scroll::anchor_tests` runs the anchoring, gas and body rules
  through `run_host` over five consecutive real mainnet blocks: a chunk anchored
  to its parent, two chunks meeting at a boundary block, a skipped block, a
  repeated block, a substituted parent header, a pre-state root that is not the
  parent's, a misreported `gas_used`, and a misreported transactions root,
  receipts root and logs bloom.
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
failure. Regenerating the corpus from a node whose execution witness carries
slot `101` is what un-ignores them.

## Upkeep notes

- Rebase this branch onto `dogeos/main`, then rebase `dogeos/main` onto
  `master`.
- If the chain configuration ever grows a second overhead source, change
  `BaseFeeDatabase`'s documentation with it: the account-before-slot order it
  establishes is a property of `SparseState`, not of the base-fee rule.
