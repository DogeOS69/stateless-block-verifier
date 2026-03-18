This directory holds negative queue-read regressions for DogeOS Scroll witnesses.

`raw-20239240.json` was copied from:
`/home/dgh/dev/DogeOS69/scroll-zkvm-prover/crates/integration/testdata/galileov2/witnesses/20239240.json`

That raw prover fixture replays the block, but it does not include the extra authenticated
`L2MessageQueue` account and storage proof nodes that DogeOS appends during witness dump.

Expected behavior in this repo:
- execution must not panic
- the verifier must fail explicitly
- the error must explain that DogeOS Scroll witnesses need queue proofs for slots `0x0` and `0x1`
