---
name: build-test-image
description: >
  Cross-compile miniboxd and test binaries for Linux musl, then build a
  minibox-tester Docker image inside Colima. Use to run Linux integration
  tests on macOS via Colima.
argument-hint: "[--target <triple>]"
---

# build-test-image

Cross-compiles test binaries and builds `minibox-tester:latest` inside Colima.

```nu
nu scripts/build-test-image.nu                                           # default: aarch64-unknown-linux-musl
nu scripts/build-test-image.nu --target x86_64-unknown-linux-musl
```

Steps:
1. Cross-compile `miniboxd`, `minibox-cli`, and all test binaries for the target
2. Assemble a Docker build context with binaries and a run-tests.sh entrypoint
3. Pipe the context to `colima ssh -- docker build`

After building, run with:
```sh
colima ssh -- docker run --rm --privileged minibox-tester:latest /run-tests.sh
```
