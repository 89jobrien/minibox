# macbox

macOS daemon implementation using Colima (limactl + nerdctl) for container management.

## Architecture

Adapters for Colima VM lifecycle:
- `ColimaRegistry` — Image pulling via nerdctl
- `ColimaRuntime` — Container creation/lifecycle via nerdctl
- `ColimaFilesystem` — Overlay mount management
- `ColimaLimiter` — Resource limit enforcement (memory, CPU)

All operations shell out to limactl/nerdctl CLIs running inside the Colima VM.

## Limitations

- Requires Colima VM to be running
- No direct namespace isolation (runs inside Colima's Linux VM)
- Performance overhead from VM boundary crossing

## Setup

```bash
colima start              # Start Colima VM
colima limactl shell      # Enter VM for debugging
```
