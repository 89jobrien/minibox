#!/usr/bin/env nu

def main [
    task?: string               # Task description (or pipe via stdin)
    --no-synthesis              # Skip synthesis step
    --prod                      # Use GPT-5.4 models instead of 4.1
] {
    let bin = $"($env.PWD)/agentbox/bin/agentbox"
    if not ($bin | path exists) {
        print "building agentbox..."
        ^go build -C agentbox -o bin/agentbox ./cmd/agentbox
    }
    let task_text = if ($task | is-empty) {
        $in
    } else {
        $task
    }
    let args = [meta-agent $task_text]
    let args = if $no_synthesis { $args | append "--no-synthesis" } else { $args }

    let openai_model = if $prod { "gpt-5.5" } else { "gpt-4.1-mini" }
    let dotenv_key = (^op read --account my.1password.com "op://byxmw65w7idxsk3i6qbohfiuty/nihl7o2bojy53zy4aqtr7txyqi/password")

    with-env {
        OPENAI_MODEL: $openai_model
        DOTENV_PRIVATE_KEY: $dotenv_key
    } {
        ^dotenvx run $"--env-file=($env.HOME)/dev/.env" -- $bin ...$args
    }
}
