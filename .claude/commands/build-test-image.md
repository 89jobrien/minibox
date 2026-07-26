---
name: build-test-image
description: >
  Cross-compile miniboxd and test binaries for Linux musl, then build a
  minibox-tester Docker image inside Colima. Use to run Linux integration
  tests on macOS via Colima.
argument-hint: "[--target <triple>]"
allowed-tools: [Bash]
---

Cross-compile test binaries for Linux musl and build `minibox-tester:latest` in Colima.
Parse `$ARGUMENTS` for: `--target <triple>` (default: `aarch64-unknown-linux-musl`).

1. **Cross-compile** with env vars `CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc`
   and `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc`:
   ```
   cargo build --target <target> -p miniboxd
   cargo build --target <target> -p minibox-cli
   cargo test --no-run --target <target> -p miniboxd --test cgroup_tests
   cargo test --no-run --target <target> -p miniboxd --test system_tests
   cargo test --no-run --target <target> -p miniboxd --test integration_tests
   cargo test --no-run --target <target> -p miniboxd --test sandbox_tests
   ```

2. **Gather binaries** from `~/.minibox/cache/target/<target>/debug/`:
   - `miniboxd`, `minibox` (in bin dir)
   - Test binaries: find `<name>-<hex>` in `deps/` by prefix, take newest

3. **Assemble build context** in `mktemp -d`:
   - Copy binaries to `usr/local/bin/`
   - Write `run-tests.sh` (runs each test suite with `MINIBOX_TEST_BIN_DIR=/usr/local/bin`)
   - Write `Dockerfile`: `FROM alpine:3.21`, COPY usr, COPY run-tests.sh, RUN chmod

4. **Build in Colima**:
   ```
   COPYFILE_DISABLE=1 tar --no-xattrs -c -C <ctx> . | colima ssh -- docker build -t minibox-tester:latest -
   ```

5. Clean up temp dir. Print: `minibox-tester:latest ready in Colima`

Run with: `colima ssh -- docker run --rm --privileged minibox-tester:latest /run-tests.sh`
