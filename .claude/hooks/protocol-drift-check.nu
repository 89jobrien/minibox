#!/usr/bin/env nu

# PreToolUse/Edit hook: warn when the canonical protocol changes.
# DaemonRequest consumers must stay in sync — see CLAUDE.md "Protocol gotchas".

let input = open --raw /dev/stdin | from json
let file_path = ($input | get -o tool_input.file_path | default "")

let log_file = ($env.HOME | path join ".mbx" "automation-runs.jsonl")
let ts = (date now | format date "%Y-%m-%dT%H:%M:%S")

if ($file_path | str ends-with "minibox-core/src/protocol.rs") {
    let msg = "WARN: editing minibox-core/src/protocol.rs — also audit server, handlers, frontends, and tests"
    print $"[protocol-drift-check] ($msg)"
    print "  minibox-core owns DaemonRequest; all consumers use that canonical definition."
    $"{\"run_id\": \"($ts)\", \"script\": \"protocol-drift\", \"status\": \"complete\", \"duration_s\": 0.0, \"output\": \"($msg)\"}\n"
    | save --append $log_file
}

exit 0
