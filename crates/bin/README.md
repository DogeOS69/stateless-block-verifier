# Command Line Utils

The CLI dumps and verifies Scroll/DogeOS block witnesses. Scroll mode and host compression
support are enabled without extra Cargo features.

## Build

```shell
cargo build --bin sbv-cli --release
```

## Verify witness files

Pass one or more JSON witness files to `run`. Without `--hardfork`, the CLI selects the
chain specification from the witness chain ID (DogeOS mainnet, Chikyu, or dev).

Every invocation requires the trusted parent header of the first witness with `--parent-header`,
including pre-Tsuki forks. The file accepts a witness's `header` object or an RPC block header.
The CLI sorts witnesses by block number and anchors each to the preceding witness. All witnesses
must be consecutive blocks of one chain; gaps, duplicates, and mixed chains are rejected.

```shell
./target/release/sbv-cli run --parent-header parent.json witness.json
```

Use `--hardfork tsuki` to replay the recorded Tsuki corpus with its matching rules:

```shell
./target/release/sbv-cli run --hardfork tsuki \
  --parent-header ./testdata/dogeos/base-fee-rule/configured-overhead/parent.json \
  ./testdata/dogeos/base-fee-rule/configured-overhead/[1-7].json
```

The legacy Feynman fixtures lack the slot-101 proof required by the current executor.
Their replay tests are ignored until complete witnesses are available.

## Dump witnesses from RPC

The RPC endpoint must support `debug_executionWitness`. The output is a JSON file per block.
A range excludes its end block.

```shell
./target/release/sbv-cli dump --rpc http://localhost:8545 --block 11 --out-dir ./witness
./target/release/sbv-cli dump --rpc http://localhost:8545 --block 11..14 --out-dir ./witness
```

Scroll witnesses do not contain ancestor block hashes. The Ethereum-only `--ancestors`
option and ancestor-fetching APIs have been removed.

Run `sbv-cli run --help` or `sbv-cli dump --help` for all available options.
