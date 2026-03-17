# Bench

This directory contains benchmarking utilities for minibox.

## Build

```
cargo build -p minibox-bench
```

## Run (Dry)

```
./target/debug/minibox-bench --dry-run
```

## Run (Full)

```
./target/debug/minibox-bench
```

## Output

Results are written to `bench/results/<timestamp>.json` and `bench/results/<timestamp>.txt`.
