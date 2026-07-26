#!/usr/bin/env nu
# Build all e2e test images from their Dockerfiles.
#
# Each subdirectory under tests/e2e/images/ that contains a Dockerfile
# is treated as one image build context. The image is tagged as
# `minibox-e2e/<dir-name>:latest`.
#
# Usage:
#   nu tests/e2e/images/build.nu
#   nu tests/e2e/images/build.nu --dry-run
#   nu tests/e2e/images/build.nu --image alpine-echo

def main [
    --dry-run       # Print commands without executing
    --image: string # Build only this image (by directory name)
] {
    let script_dir = ($env.CURRENT_FILE | path dirname)
    let images_dir = $script_dir

    let contexts = (
        ls $images_dir
        | where type == dir
        | get name
        | each { |d| {name: ($d | path basename), path: $d} }
        | where { |it| ($it.path | path join "Dockerfile" | path exists) }
    )

    let targets = if ($image | is-empty) {
        $contexts
    } else {
        $contexts | where name == $image
    }

    if ($targets | length) == 0 {
        if ($image | is-empty) {
            print "No image directories with Dockerfiles found."
        } else {
            print $"Image directory not found or has no Dockerfile: ($image)"
        }
        exit 1
    }

    let results = $targets | each { |ctx|
        let tag = $"minibox-e2e/($ctx.name):latest"
        # The minibox CLI build command: minibox build -t <tag> <context>
        # Adjust if the binary name or flags differ in your environment.
        let cmd = ["cargo" "run" "-p" "minibox-cli" "--" "build" "-t" $tag $ctx.path]
        print $"Building ($ctx.name) -> ($tag)"
        if $dry_run {
            print $"  (dry-run) ($cmd | str join ' ')"
            {name: $ctx.name, tag: $tag, status: "dry-run"}
        } else {
            let result = do { run-external ...$cmd } | complete
            if $result.exit_code == 0 {
                print $"  OK: ($ctx.name)"
                {name: $ctx.name, tag: $tag, status: "ok"}
            } else {
                print $"  FAILED: ($ctx.name)"
                print $result.stderr
                {name: $ctx.name, tag: $tag, status: "failed"}
            }
        }
    }

    print ""
    print "Build summary:"
    $results | each { |r| print $"  ($r.name): ($r.status)" }

    let failures = $results | where status == "failed"
    if ($failures | length) > 0 {
        exit 1
    }
}
