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

```shell
./target/release/sbv-cli run witness.json
```

Use `--hardfork` to verify historical Scroll fixtures with the matching rules:

```shell
./target/release/sbv-cli run --hardfork feynman ./testdata/scroll/feynman/*.json
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
