# minibox toolkit — auto-loaded by toolkit-hook when you cd into this repo

export def "fmt"        [] { cargo fmt --all }
export def "check"      [] { cargo check --workspace }
export def "lint"       [] { just lint }
export def "test"       [] { just test-unit }
export def "nextest"    [] { just nextest }
export def "build"      [] { just build }
export def "build-linux" [] { just build-linux }
export def "ci"         [] { just ci }
export def "pre-commit" [] { cargo xtask pre-commit }
export def "prepush"    [] { cargo xtask prepush }
export def "coverage"   [] { just coverage }
export def "doctor"     [] { just doctor }
export def "clean"      [] { cargo clean }
export def "context"    [] { just context }

export def "help" [] {
    scope commands
    | where name =~ "^tk "
    | select name
    | sort-by name
}
