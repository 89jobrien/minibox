# winbox

Windows container runtime stub — reserved for future implementation.

## Status

Currently a placeholder with dependencies declared but not implemented. When enabled, will provide:

- Windows Container subsystem (HCS) integration
- Process isolation / Hyper-V container support
- OCI image pulling for Windows

## See Also

- `hcs.rs` adapter skeleton in minibox (defines HCS types and APIs)
- `docs/core/FEATURE_MATRIX.mbx.md` — current Windows support status

## Future Work

1. Implement HCS (Host Compute Service) adapter in `minibox/src/adapters/hcs.rs`
2. Wire into `miniboxd` platform dispatch
3. Add Windows-specific container tests
