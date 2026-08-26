# Structured verifier errors

`sbv-core` now preserves typed sources and phase-specific context in
`StatelessValidationError`:

- sparse-state construction errors contain the attempted pre-state root and the original
  `alloy_rlp::Error`;
- execution errors contain the failing block number and original `BlockExecutionError`;
- Scroll queue metadata reads use a dedicated `L2MessageQueueInfoFailed` variant containing the
  block number and original `ProviderError`.

This is a public API change. Downstream exhaustive matches must update the former unit-like
`SparseStateCreationFailed` and tuple-like `StatelessExecutionFailed(String)` patterns, and
Scroll-enabled consumers must handle the new queue-info variant.
