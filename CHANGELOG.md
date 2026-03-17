# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

## [v0.0.10] - 2026-03-17

### Added

- GitHub Actions workflows for CI, release, and integration testing.
- Security-critical tests for path validation (Zip Slip prevention) and tar extraction safety.

### Fixed

- Resolved all clippy warnings blocking CI, including Linux-only lints.
- Narrowed security clippy lints to the `suspicious` group to reduce false positives.
- Fixed test module placement, unit struct defaults, and e2e process kill capture.
- Switched reqwest to `rustls-tls` for static musl cross-compilation support.
