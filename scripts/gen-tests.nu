#!/usr/bin/env nu
# Scaffold unit tests for a new minibox domain trait adapter.
# Example: nu scripts/gen-tests.nu BridgeNetworking

def main [
    trait: string             # Domain trait name (e.g. BridgeNetworking)
    --output: string = ""     # Output file path (default: Claude decides)
] {
    let args = [$trait]
    let args = if ($output | is-not-empty) {
        $args | append ["--output" $output]
    } else {
        $args
    }

    ^uv run ($env.FILE_PWD | path join "gen-tests.py") ...$args
}
