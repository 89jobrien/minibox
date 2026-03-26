# minibox-bench

Benchmark harness measuring protocol codec performance and container trait overhead.

## Benchmarks

### Protocol Codec
Nanosecond-scale measurements for encoding/decoding minibox protocol messages:
- Container request/response serialization
- Image manifest JSON parsing
- Stream output message packing

Results saved to `bench/results/bench.jsonl` (append-only history) and `latest.json` (canonical snapshot for devloop).

### Trait Overhead
Adapter pattern call overhead — measuring the cost of dynamic dispatch through domain traits:
- `ContainerRuntime` trait calls
- `ImageRegistry` adapter lookups
- `FilesystemProvider` vtable traversal

## Running

```bash
cargo build -p minibox-bench --release
./target/release/minibox-bench --suite codec
./target/release/minibox-bench --suite adapter
```

Or via xtask:

```bash
cargo xtask bench           # local
cargo xtask bench-vps       # on VPS with auto-fetch
```
