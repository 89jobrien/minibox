#!/usr/bin/env bash
# Convert pprof files to Firefox Profiler JSON format
# Usage: ./bench/convert-to-firefox.sh <pprof-file>
# Output: {basename}.firefox.json in the same directory

set -e

pprof_file="$1"

if [[ -z "$pprof_file" ]]; then
    echo "Usage: convert-to-firefox.sh <pprof-file>"
    exit 1
fi

if [[ ! -f "$pprof_file" ]]; then
    echo "Error: File not found: $pprof_file"
    exit 1
fi

output="${pprof_file%.pprof}.firefox.json"

# Check if pprof tool is available
if ! command -v pprof &> /dev/null; then
    echo "Error: pprof tool not found. Install go-pprof or graphviz-related tools."
    exit 1
fi

# Use pprof CLI to convert to JSON format compatible with Firefox Profiler
pprof -json "$pprof_file" > "$output"

echo "Converted to $output"
echo "Import into Firefox Profiler: https://profiler.firefox.com/"
