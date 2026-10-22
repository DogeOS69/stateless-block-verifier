"""RLP-encodes the header of an empty DogeOS execution payload, for resealing a mutated one.

An `ExecutionPayloadV1` carries the header's variable fields; the rest are constants on a DogeOS
chain. The body has to be empty, so that the transactions root is the empty-trie root and no
transaction has to be re-encoded.

Field order, from alloy's `Header::encode`: parent_hash, ommers_hash, beneficiary, state_root,
transactions_root, receipts_root, logs_bloom, difficulty, number, gas_limit, gas_used, timestamp,
extra_data, mix_hash, nonce, base_fee_per_gas.

Usage: seal_header.py PAYLOAD_JSON [BASE_FEE_OVERRIDE]
Prints the RLP; keccak it to get the block hash (`cast keccak`).
"""

import json
import sys

from l1_message import integer, rlp

EMPTY_OMMER_ROOT = bytes.fromhex(
    "1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347"
)
EMPTY_TRIE_ROOT = bytes.fromhex(
    "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
)
# DogeOS headers carry difficulty one and a zero nonce; `validate_header` requires both.
DIFFICULTY = 1
NONCE = bytes(8)


def raw(value: str) -> bytes:
    return bytes.fromhex(value.removeprefix("0x"))


def main() -> None:
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    payload = json.load(open(sys.argv[1]))
    if payload["transactions"]:
        raise SystemExit("only an empty payload can be resealed without re-encoding its body")
    base_fee = int(sys.argv[2], 0) if len(sys.argv) == 3 else int(payload["baseFeePerGas"], 16)

    fields = [
        raw(payload["parentHash"]),
        EMPTY_OMMER_ROOT,
        raw(payload["feeRecipient"]),
        raw(payload["stateRoot"]),
        EMPTY_TRIE_ROOT,
        raw(payload["receiptsRoot"]),
        raw(payload["logsBloom"]),
        integer(DIFFICULTY),
        integer(int(payload["blockNumber"], 16)),
        integer(int(payload["gasLimit"], 16)),
        integer(int(payload["gasUsed"], 16)),
        integer(int(payload["timestamp"], 16)),
        raw(payload["extraData"]),
        raw(payload["prevRandao"]),
        NONCE,
        integer(base_fee),
    ]
    print("0x" + rlp(fields).hex())


if __name__ == "__main__":
    main()
