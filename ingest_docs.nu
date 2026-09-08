# This small Nu script will iterate over all .mbx.md and .md files in docs/ and pipe them into kgx ingest.

# TODO(#478): glob pattern *.{mbx,txt} misses .mbx.md files — correct pattern is *.mbx.md (or **/*.md)
# TODO(#479): `nu -c "glob ..."` spawns an unnecessary child process; use `glob` directly in Nu
# TODO(#480): `| lines` won't work — glob returns a list, not a string; remove the lines call
let docs = (nu -c "glob docs/**/*.{mbx,txt}"
   | lines)
for doc in $docs {
    let path = ($doc | str replace --regex ".*\/docs\/(.*)" "$1")
    echo 'Ingesting' $path
    cat ('docs/' + $path) | kgx ingest
}
