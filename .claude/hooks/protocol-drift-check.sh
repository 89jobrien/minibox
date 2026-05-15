#!/usr/bin/env nu

# PreToolUse/Edit hook: warn when one protocol.rs is edited without the other.
# The two DaemonRequest definitions must stay in sync — see CLAUDE.md "Protocol gotchas".

let input = open --raw /dev/stdin | from json
let file_path = ($input | get -i tool_input.file_path | default "")

let log_file = ($env.HOME | path join ".mbx" "automation-runs.jsonl")
let ts = (date now | format date "%Y-%m-%dT%H:%M:%S")

if ($file_path | str ends-with "minibox-core/src/protocol.rs") {
    let msg = "WARN: editing minibox-core/src/protocol.rs — also update crates/mbx/src/protocol.rs"
    print $"[protocol-drift-check] ($msg)"
    print "  Both files define DaemonRequest independently and must stay in sync."
    $"{\"run_id\": \"($ts)\", \"script\": \"protocol-drift\", \"status\": \"complete\", \"duration_s\": 0.0, \"output\": \"($msg)\"}\n"
    | save --append $log_file
} else if ($file_path | str ends-with "mbx/src/protocol.rs") {
    let msg = "WARN: editing mbx/src/protocol.rs — also update crates/minibox-core/src/protocol.rs"
    print $"[protocol-drift-check] ($msg)"
    print "  Both files define DaemonRequest independently and must stay in sync."
    $"{\"run_id\": \"($ts)\", \"script\": \"protocol-drift\", \"status\": \"complete\", \"duration_s\": 0.0, \"output\": \"($msg)\"}\n"
    | save --append $log_file
}

exit 0
