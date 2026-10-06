# Release workflow for pqless. Run `just` to list recipes.
#
# To release: add a "## X.Y.Z - YYYY-MM-DD" entry to CHANGELOG.md, then
# run `just release X.Y.Z`. That runs bump, wait-ci and tag in turn; each can
# also be run on its own, e.g. to retry after a failure.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List recipes
default:
    @just --list --unsorted

# Format check, lint and test, as CI does
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Bump to VERSION, wait for CI, then tag and publish
release version: (bump version) wait-ci tag

# 1. Set VERSION, run checks, commit and push to main
bump version:
    #!/usr/bin/env bash
    set -euo pipefail
    v="{{ version }}"
    [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 1.2.3"; exit 1; }
    [[ "$(git branch --show-current)" == main ]] || { echo "switch to main first"; exit 1; }
    # The new CHANGELOG entry may be uncommitted; nothing else may be.
    [[ -z "$(git status --porcelain -- . ':!CHANGELOG.md')" ]] || { echo "commit or stash other changes first"; exit 1; }
    git rev-parse -q --verify "refs/tags/v$v" >/dev/null && { echo "tag v$v already exists"; exit 1; }
    grep -q "^## $v - " CHANGELOG.md || { echo "add a '## $v - $(date +%F)' entry to CHANGELOG.md first"; exit 1; }
    perl -0pi -e "s/^version = \"[^\"]*\"/version = \"$v\"/m" Cargo.toml
    {{ just_executable() }} check  # also refreshes Cargo.lock
    git add Cargo.toml Cargo.lock CHANGELOG.md
    git commit -m "chore: release $v"
    git push origin main

# 2. Wait for CI to pass on the pushed main
wait-ci:
    #!/usr/bin/env bash
    set -euo pipefail
    git fetch -q origin
    sha=$(git rev-parse HEAD)
    [[ "$(git rev-parse origin/main)" == "$sha" ]] || { echo "HEAD is not what's on origin/main; push first"; exit 1; }
    echo "Waiting for CI on ${sha:0:7}..."
    id=
    for _ in $(seq 24); do  # a pushed run can take a moment to appear
        id=$(gh run list --workflow CI --commit "$sha" --limit 1 --json databaseId --jq '.[0].databaseId // empty')
        [[ -n "$id" ]] && break
        sleep 5
    done
    [[ -n "$id" ]] || { echo "no CI run found for ${sha:0:7}"; exit 1; }
    gh run watch "$id" --compact --exit-status --interval 15

# 3. Tag the version in Cargo.toml and push it, publishing the release
tag:
    #!/usr/bin/env bash
    set -euo pipefail
    git fetch -q --tags origin
    v=$(perl -ne 'if (/^version = "([^"]+)"/) { print $1; exit }' Cargo.toml)
    tag="v$v"
    sha=$(git rev-parse HEAD)
    [[ -z "$(git status --porcelain)" ]] || { echo "working tree is not clean"; exit 1; }
    [[ "$(git rev-parse origin/main)" == "$sha" ]] || { echo "HEAD is not what's on origin/main; push first"; exit 1; }
    git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "tag $tag already exists"; exit 1; }
    grep -q "^## $v - " CHANGELOG.md || { echo "CHANGELOG.md has no entry for $v"; exit 1; }
    ci=$(gh run list --workflow CI --commit "$sha" --limit 1 --json conclusion --jq '.[0].conclusion // empty')
    [[ "$ci" == success ]] || { echo "CI has not passed on ${sha:0:7} (${ci:-no run}); try 'just wait-ci'"; exit 1; }
    git tag -a "$tag" -m "$tag"
    git push origin "$tag"
    echo "Waiting for the release build..."
    id=
    for _ in $(seq 24); do
        id=$(gh run list --workflow Release --branch "$tag" --limit 1 --json databaseId --jq '.[0].databaseId // empty')
        [[ -n "$id" ]] && break
        sleep 5
    done
    [[ -n "$id" ]] || { echo "no Release run found for $tag; check the Actions tab"; exit 1; }
    gh run watch "$id" --compact --exit-status --interval 30
    gh release view "$tag" --json url --jq .url
