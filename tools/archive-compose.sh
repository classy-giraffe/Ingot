#!/bin/bash
# Archive the pinned Fedora Rawhide compose into the local, dnf-usable
# repository (tools/compose-mirror.py). The build's --local mode
# rebuilds the slot artifact from this archive alone (no network
# access to the compose), which is the reproducibility criterion of
# ticket #15.
#
# Usage: tools/archive-compose.sh
set -euo pipefail

cd "$(dirname "$0")/.."

compose_id=$(python3 -c 'import json;print(json.load(open("tools/pins.json"))["compose"]["id"])')
archive_dir=$(python3 -c 'import json;print(json.load(open("tools/pins.json"))["compose"]["archive_dir"])')

echo "archiving $compose_id -> $archive_dir"
python3 tools/compose-mirror.py --compose-id "$compose_id" --destdir "$archive_dir"

tree="$archive_dir/compose/Everything/x86_64/os"
[ -f "$tree/repodata/repomd.xml" ] || { echo "archive incomplete: no repomd.xml" >&2; exit 1; }
[ -f "$archive_dir/compose/manifest.json" ] \
    || { echo "archive incomplete: no manifest.json" >&2; exit 1; }
echo "archive complete: $tree"
