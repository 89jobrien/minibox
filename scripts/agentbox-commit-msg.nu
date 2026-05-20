#!/usr/bin/env nu

def main [
    --stage (-a)    # Stage all changes before generating
    --commit (-c)   # Commit with the generated message
    --yes (-y)      # Skip confirmation and commit immediately
    --prod          # Use GPT-5.4 models instead of 4.1
] {
    let bin = $"($env.PWD)/agentbox/bin/mbx-commit-msg"
    if not ($bin | path exists) {
        print "building mbx-commit-msg..."
        ^go build -C agentbox -o bin/mbx-commit-msg ./cmd/mbx-commit-msg
    }
    let args = []
    let args = if $stage { $args | append "-a" } else { $args }
    let args = if $commit { $args | append "-c" } else { $args }
    let args = if $yes { $args | append "-y" } else { $args }

    let openai_model = if $prod { "gpt-5.4-nano" } else { "gpt-4.1-nano" }
    let dotenv_key = (^op read --account my.1password.com "op://byxmw65w7idxsk3i6qbohfiuty/nihl7o2bojy53zy4aqtr7txyqi/password")

    with-env {
        OPENAI_MODEL: $openai_model
        DOTENV_PRIVATE_KEY: $dotenv_key
    } {
        ^dotenvx run $"--env-file=($env.HOME)/dev/.env" -- $bin ...$args
    }
}
