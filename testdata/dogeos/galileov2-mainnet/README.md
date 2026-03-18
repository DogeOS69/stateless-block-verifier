Live Scroll mainnet GalileoV2 regression fixture.

- Source block: `32144474`
- Chain ID: `534352` (Scroll mainnet)
- Dumped with: `sbv-cli dump --block 32144474 --rpc https://scroll.api.onfinality.io/public`
- Queue proofs: appended by the DogeOS dump path in `crates/utils/src/rpc.rs`

Expected post-state queue outputs at block `32144474`:

- `withdraw_root = 0x9d42c00e8305f065d4e8df4d073cd9424986b0dcc63becf39b718d9f61f99179`
- `next_message_index = 221555`

Notes:

- This fixture is replayed with `Hardfork::GalileoV2`.
- The current `scroll-v91.2` chainspec dependency used by this repo does not yet activate
  Galileo/GalileoV2 on Scroll mainnet, so replay with the default mainnet chainspec fails even
  though the live block is post-activation.
