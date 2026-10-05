#!/usr/bin/env bash
# Upload every file in a directory to a GitHub release, replacing same-named assets.
#
#   bash scripts/upload-assets.sh <owner/repo> <release_id> <dir>
#
# Requires GH_TOKEN. Delete-then-upload keeps re-runs idempotent (the API rejects duplicate names).
set -euo pipefail

REPO="${1:?usage: upload-assets.sh <owner/repo> <release_id> <dir>}"
RELEASE_ID="${2:?missing release id}"
DIR="${3:?missing directory}"
: "${GH_TOKEN:?GH_TOKEN is required}"

EXISTING=$(gh api "repos/$REPO/releases/$RELEASE_ID/assets")
shopt -s nullglob
for f in "$DIR"/*; do
  name=$(basename "$f")
  id=$(echo "$EXISTING" | jq -r --arg n "$name" '.[] | select(.name==$n) | .id')
  if [ -n "$id" ] && [ "$id" != "null" ]; then
    curl -s -X DELETE -H "Authorization: Bearer $GH_TOKEN" \
      "https://api.github.com/repos/$REPO/releases/assets/$id" >/dev/null
  fi
  curl -sS --fail -X POST \
    -H "Authorization: Bearer $GH_TOKEN" \
    -H "Content-Type: application/octet-stream" \
    --data-binary @"$f" \
    "https://uploads.github.com/repos/$REPO/releases/$RELEASE_ID/assets?name=$name" >/dev/null
  echo "uploaded $name -> $REPO"
done
