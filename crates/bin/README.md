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

Use `--hardfork` to verify historical Scroll fixtures with the matching rules:

```shell
jq '.header' ./testdata/scroll/feynman/534352-19604670.json > parent.json
./target/release/sbv-cli run --hardfork feynman --parent-header parent.json \
  ./testdata/scroll/feynman/534352-19604671.json \
  ./testdata/scroll/feynman/534352-19604672.json
```

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
