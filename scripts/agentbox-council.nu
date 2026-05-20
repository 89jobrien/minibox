#!/usr/bin/env nu

def main [
    --base: string = "main"     # Base branch to diff against
    --mode: string = "core"     # Review mode: core (3 roles) or extensive (5 roles)
    --no-synthesis              # Skip synthesis step
    --prod                      # Use GPT-5.4 models instead of 4.1
] {
    let bin = $"($env.PWD)/agentbox/bin/agentbox"
    if not ($bin | path exists) {
        print "building agentbox..."
        ^go build -C agentbox -o bin/agentbox ./cmd/agentbox
    }
    let args = [council --base $base --mode $mode]
    let args = if $no_synthesis { $args | append "--no-synthesis" } else { $args }

    let openai_model = if $prod { "gpt-5.4-mini" } else { "gpt-4.1-mini" }
    let dotenv_key = (^op read --account my.1password.com "op://byxmw65w7idxsk3i6qbohfiuty/nihl7o2bojy53zy4aqtr7txyqi/password")

    with-env {
        OPENAI_MODEL: $openai_model
        DOTENV_PRIVATE_KEY: $dotenv_key
    } {
        ^dotenvx run $"--env-file=($env.HOME)/dev/.env" -- $bin ...$args
    }
}
