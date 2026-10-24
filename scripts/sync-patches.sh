#!/usr/bin/env bash
#
# Commits the patches in patches/ and pushes them to GitHub.
#
# Only patches/ is staged, so a checkout that also has work in progress on the
# source code can use this script without committing anything else.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "error: $repo_root is not a git checkout" >&2
    exit 1
fi

git add --all -- patches

if git diff --cached --quiet -- patches; then
    echo "no patch changes to sync"
    exit 0
fi

stamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
git commit -m "patches: sync user patches ($stamp)" -- patches
git push origin HEAD
