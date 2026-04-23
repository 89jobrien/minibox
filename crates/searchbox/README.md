# searchbox

Code-search daemon for minibox, backed by [Zoekt](https://github.com/sourcegraph/zoekt).

Runs `searchboxd`, an async HTTP service that manages Zoekt indexing and query proxying for
repositories hosted on or accessible to the minibox VPS.

## Architecture

```text
searchboxd
├── zoektbox   — Zoekt lifecycle management (provision, deploy, health)
├── Indexer    — watches repositories and triggers reindex on push
└── QueryProxy — forwards search requests to the Zoekt webserver
```

## Running

```bash
cargo build -p searchbox --release
sudo ./target/release/searchboxd --config /etc/searchbox/config.toml
```

## Configuration

Configuration is TOML-based:

```toml
[server]
listen = "0.0.0.0:3100"

[zoekt]
bin_dir = "/opt/zoekt/bin"
data_dir = "/var/lib/zoekt"

[[repos]]
url = "git@github.com:89jobrien/minibox.git"
```

Default config path: `~/.config/searchbox/config.toml` (macOS: `~/Library/Application Support/searchbox/config.toml`).

## Features

| Feature             | Description                                        |
| ------------------- | -------------------------------------------------- |
| `integration-tests` | Enable tests that require a live Zoekt process.    |
|                     | Gate with `#[cfg(feature = "integration-tests")]`. |

## Modules

| Crate/Module | Description                                             |
| ------------ | ------------------------------------------------------- |
| `zoektbox`   | Zoekt binary management (version pinning, provisioning) |
| `searchboxd` | Binary entry point — HTTP server and request routing    |
