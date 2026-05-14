#!/usr/bin/env bash
# Cut a tagged GitHub release for Mnemonic.
#
# Usage:
#   ./scripts/release.sh                 # version auto-detected from Cargo.toml
#   ./scripts/release.sh NOTES.md        # body of the draft release
#
# Flow:
#   1. Preflight: clean tree, on main, in sync with origin, versions agree in
#      Cargo.toml + tauri.conf.json, gh authenticated, no existing tag.
#   2. Run workspace tests.
#   3. Tag v<version> locally and push.
#   4. Build the signed + notarized DMG via scripts/bundle.sh.
#   5. Create a DRAFT GitHub release with the DMG attached and open it in your
#      browser to review notes before publishing. The script never publishes;
#      hitting "Publish release" in the UI is the last manual step.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

NOTES_FILE="${1:-}"

red()    { printf '\033[31m%s\033[0m\n' "$*" >&2; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
step()   { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die()    { red "ERROR: $*"; exit 1; }

# 1. Preflight ----------------------------------------------------------------
step "Preflight"

command -v gh   >/dev/null || die "gh CLI not installed — \`brew install gh\`"
command -v jq   >/dev/null || die "jq not installed — \`brew install jq\`"
gh auth status  >/dev/null 2>&1 || die "gh not authenticated — run \`gh auth login\`"

[ -n "$(git status --porcelain --untracked-files=no)" ] \
  && die "working tree has uncommitted changes — commit or stash first"

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$CURRENT_BRANCH" = "main" ] || die "must be on main (currently on $CURRENT_BRANCH)"

git fetch origin --quiet
LOCAL="$(git rev-parse HEAD)"
REMOTE="$(git rev-parse origin/main)"
[ "$LOCAL" = "$REMOTE" ] || die "local main and origin/main have diverged — push or pull first"

CARGO_VERSION="$(grep -E '^version = ' Cargo.toml | head -1 | sed -E 's/version = "(.+)"/\1/')"
TAURI_VERSION="$(jq -r '.version' app/tauri.conf.json)"
[ "$CARGO_VERSION" = "$TAURI_VERSION" ] \
  || die "version mismatch: Cargo.toml=$CARGO_VERSION  tauri.conf.json=$TAURI_VERSION"

TAG="v$CARGO_VERSION"
green "Releasing $TAG"

if git rev-parse --verify --quiet "refs/tags/$TAG" >/dev/null; then
  die "tag $TAG already exists locally — bump the version or delete the tag"
fi
if git ls-remote --tags origin "refs/tags/$TAG" | grep -q "$TAG"; then
  die "tag $TAG already exists on origin — bump the version"
fi

if gh release view "$TAG" >/dev/null 2>&1; then
  die "GitHub release $TAG already exists — bump the version"
fi

if [ -n "$NOTES_FILE" ] && [ ! -f "$NOTES_FILE" ]; then
  die "notes file $NOTES_FILE not found"
fi

if [ ! -f .env ]; then
  yellow "WARNING: no .env at repo root — DMG will be UNSIGNED. Continue? (y/N)"
  read -r ans
  [ "$ans" = "y" ] || die "aborted"
fi

# 2. Tests --------------------------------------------------------------------
step "cargo test --workspace"
cargo test --workspace --quiet -- --test-threads=1

# 3. Tag ----------------------------------------------------------------------
step "Tagging $TAG and pushing"
git tag -a "$TAG" -m "Mnemonic $TAG"
git push origin "$TAG"

# 4. Build --------------------------------------------------------------------
step "Building DMG (scripts/bundle.sh)"
./scripts/bundle.sh

DMG="$(find "target/aarch64-apple-darwin/release/bundle/dmg" \
        -maxdepth 1 -name "Mnemonic_${CARGO_VERSION}_aarch64.dmg" | head -1)"
[ -f "$DMG" ] || die "expected DMG not found: Mnemonic_${CARGO_VERSION}_aarch64.dmg"

SHA="$(shasum -a 256 "$DMG" | awk '{print $1}')"
green "DMG ready: $DMG"
green "SHA256:  $SHA"

# 5. Draft release ------------------------------------------------------------
step "Creating draft GitHub release"

NOTES_BODY="$(mktemp -t mnemonic-release-notes).md"
if [ -n "$NOTES_FILE" ]; then
  cp "$NOTES_FILE" "$NOTES_BODY"
else
  cat > "$NOTES_BODY" <<EOF
## Mnemonic $TAG

<!-- Replace this placeholder with release notes before publishing. -->

### What changed

- ...

### Install

\`\`\`bash
brew tap EduardMaghakyan/tap
brew install --cask mnemonic
\`\`\`

Or download \`Mnemonic_${CARGO_VERSION}_aarch64.dmg\` below.
EOF
  yellow "No notes file passed; using placeholder at $NOTES_BODY"
fi

# Append Verification section unless the notes file already has one. The
# hash is only known after bundle.sh runs, so authoring it by hand is
# impossible — this keeps the human-written notes free of build artefacts.
if ! grep -q '^### Verification' "$NOTES_BODY"; then
  cat >> "$NOTES_BODY" <<EOF

### Verification

\`\`\`
SHA256: $SHA
\`\`\`

\`\`\`bash
shasum -a 256 Mnemonic_${CARGO_VERSION}_aarch64.dmg
codesign --verify --deep --strict --verbose=2 /Applications/Mnemonic.app
spctl --assess --type execute /Applications/Mnemonic.app
\`\`\`
EOF
fi

gh release create "$TAG" "$DMG" \
  --draft \
  --title "Mnemonic $TAG" \
  --notes-file "$NOTES_BODY"

URL="$(gh release view "$TAG" --json url --jq .url)"
green "Draft release created: $URL"
yellow "Review the notes, then click 'Publish release' in the GitHub UI."
