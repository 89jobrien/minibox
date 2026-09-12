# Iterate over all Markdown files in docs/ and pipe them into kgx ingest.

let docs = (glob "docs/**/*.md")
for doc in $docs {
    print $"Ingesting ($doc)"
    open --raw $doc | kgx ingest
}
