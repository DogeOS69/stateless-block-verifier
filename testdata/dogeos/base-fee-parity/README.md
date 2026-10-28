# Base-fee parity vectors

Byte-for-byte copies of dogeos-reth's base-fee test data at the revision `Cargo.toml` pins
(`crates/dogeos-reth-evm/testdata/`):

| File | Covers |
| --- | --- |
| `base-fee-parity-vectors.json` | The shared parity vectors: mainnet (TsukiV2 from genesis), Chikyu before TsukiV2 (D8/E2, 10 gwei cap), and a chain whose TsukiV2 follows Tsuki |
| `chikyu-base-fee-history.json` | Recorded Chikyu Tsuki-era headers, with the fee each would have under TsukiV2 |

`SHA256SUMS` pins the committed bytes. When repinning dogeos-reth, copy both files again from the
new revision (`git show <rev>:crates/dogeos-reth-evm/testdata/<file>`) and regenerate the sums;
a divergence on any row is a chain split between the guest and the node.
