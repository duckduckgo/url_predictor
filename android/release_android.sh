#!/usr/bin/env bash
set -euo pipefail

# Automates:
# 1) Set version.properties to a non-SNAPSHOT
# 2) Commit "Release X.Y.Z"
# 3) Tag "X.Y.Z" (annotated)
# 4) (Optional) Publish release version to Maven local
# 5) Push + push --tags
# 6) Bump to next -SNAPSHOT (defaults to patch+1)
# 7) Commit "Prepare next development version."
# 8) Push
#
# Usage examples:
#   ./release_android.sh --new-version 1.2.3
#   ./release_android.sh --new-version 1.2.3 --bump minor
#   ./release_android.sh --new-version 1.2.3 --next-snapshot 1.3.0-SNAPSHOT
#   ./release_android.sh --new-version 1.2.3 --maven-local
#
# Optional:
#   --version-file <path>   default: version.properties
#   --no-push               do everything except pushing
#   --dry-run               print what would happen, don’t change anything
#   --tag-prefix ""         default: "" (set to "v" if you want tags like v1.2.3)
#   --maven-local           Publish release version to Maven local and skip push
#                           (runs MAVEN_LOCAL_CMD, default: "./gradlew publishToMavenLocal")

NEW_VERSION=""
NEXT_SNAPSHOT=""
VERSION_FILE="${VERSION_FILE:-version.properties}"
BUMP_KIND="patch"     # patch|minor|major
PUSH="true"
DRY_RUN="false"
TAG_PREFIX="${TAG_PREFIX:-}"
MAVEN_LOCAL="false"
MAVEN_LOCAL_CMD="${MAVEN_LOCAL_CMD:-./gradlew :ddg-url-predictor:publishToMavenLocal -PskipSigning}"


die() { echo "Error: $*" >&2; exit 1; }
run() { if [[ "$DRY_RUN" == "true" ]]; then echo "[dry-run] $*"; else eval "$@"; fi; }

usage() {
  cat <<EOF
Usage: $(basename "$0") --new-version X.Y.Z [options]

Required:
  --new-version X.Y.Z         Release version (must NOT contain -SNAPSHOT)

Options:
  --next-snapshot A.B.C-SNAPSHOT
                              If omitted, computed by bumping --bump (default: patch)
  --bump patch|minor|major    How to compute next snapshot (default: patch)
  --version-file <path>       Path to version.properties
                              (default: ${VERSION_FILE})
  --tag-prefix <prefix>       Prefix for git tag (default: "${TAG_PREFIX}")
  --no-push                   Do everything except pushing
  --maven-local               Publish release version to Maven local and skip push
                              (uses MAVEN_LOCAL_CMD: ${MAVEN_LOCAL_CMD})
  --dry-run                   Show actions without changing anything
  -h | --help                 Show this help
EOF
}

# --- Parse args ---
while [[ $# -gt 0 ]]; do
  case "$1" in
    --new-version) NEW_VERSION="${2-}"; shift 2;;
    --next-snapshot) NEXT_SNAPSHOT="${2-}"; shift 2;;
    --bump) BUMP_KIND="${2-}"; shift 2;;
    --version-file) VERSION_FILE="${2-}"; shift 2;;
    --tag-prefix) TAG_PREFIX="${2-}"; shift 2;;
    --no-push) PUSH="false"; shift;;
    --maven-local)
      MAVEN_LOCAL="true"
      PUSH="false"   # releasing to mavenLocal implies no push
      shift
      ;;
    --dry-run) DRY_RUN="true"; shift;;
    -h|--help) usage; exit 0;;
    *) die "Unknown option: $1";;
  esac
done

[[ -n "$NEW_VERSION" ]] || { usage; die "--new-version is required"; }
[[ "$NEW_VERSION" =~ -SNAPSHOT$ ]] && die "--new-version must NOT include -SNAPSHOT"

# --- Helpers for version.properties (expects a line like VERSION_NAME=1.2.3) ---
get_version() {
  [[ -f "$VERSION_FILE" ]] || die "version file not found: $VERSION_FILE"
  grep -E '^VERSION_NAME=' "$VERSION_FILE" | head -n1 | cut -d'=' -f2- || true
}

set_version() {
  local v="$1"
  [[ -f "$VERSION_FILE" ]] || die "version file not found: $VERSION_FILE"
  if [[ "$DRY_RUN" == "true" ]]; then
    echo "[dry-run] set VERSION_NAME=$v in $VERSION_FILE"
  else
    if grep -qE '^VERSION_NAME=' "$VERSION_FILE"; then
      sed -i.bak -E "s/^VERSION_NAME=.*/VERSION_NAME=${v}/" "$VERSION_FILE"
      rm -f "${VERSION_FILE}.bak"
    else
      echo "VERSION_NAME=${v}" >> "$VERSION_FILE"
    fi
  fi
}

bump_semver() {
  local base="$1" kind="$2"
  # base must be X.Y.Z (no -SNAPSHOT)
  [[ "$base" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || die "Invalid semver: $base"
  local major="${BASH_REMATCH[1]}"
  local minor="${BASH_REMATCH[2]}"
  local patch="${BASH_REMATCH[3]}"

  case "$kind" in
    patch) patch=$((patch+1));;
    minor) minor=$((minor+1)); patch=0;;
    major) major=$((major+1)); minor=0; patch=0;;
    *) die "Unknown bump kind: $kind";;
  esac

  echo "${major}.${minor}.${patch}-SNAPSHOT"
}

# --- Sanity checks ---
CURRENT_VERSION="$(get_version)"
[[ -n "$CURRENT_VERSION" ]] || die "Could not read VERSION_NAME from $VERSION_FILE"

if [[ -z "$NEXT_SNAPSHOT" ]]; then
  NEXT_SNAPSHOT="$(bump_semver "$NEW_VERSION" "$BUMP_KIND")"
fi
[[ "$NEXT_SNAPSHOT" =~ -SNAPSHOT$ ]] || die "--next-snapshot must end with -SNAPSHOT (got: $NEXT_SNAPSHOT)"

# Ensure clean working tree
if [[ "$DRY_RUN" != "true" ]]; then
  if [[ -n "$(git status --porcelain)" ]]; then
    die "Working tree is dirty. Commit or stash changes before releasing."
  fi
fi

# Ensure tag doesn't already exist
TAG_NAME="${TAG_PREFIX}${NEW_VERSION}"

if [[ "$MAVEN_LOCAL" != "true" ]]; then
  # Ensure tag doesn't already exist
  run "git fetch --tags"
  if git rev-parse "$TAG_NAME" >/dev/null 2>&1; then
    die "Tag already exists: $TAG_NAME"
  fi
else
  echo "[info] Skipping tag existence check (--maven-local)"
fi

echo "Current VERSION_NAME: ${CURRENT_VERSION}"
echo "Release version     : ${NEW_VERSION}"
echo "Next snapshot       : ${NEXT_SNAPSHOT}"
echo "Version file        : ${VERSION_FILE}"
echo "Tag name            : ${TAG_NAME}"
echo "Push to remote      : ${PUSH}"
echo "Dry run             : ${DRY_RUN}"
echo "Maven local         : ${MAVEN_LOCAL}"
if [[ "$MAVEN_LOCAL" == "true" ]]; then
  echo "Maven local command : ${MAVEN_LOCAL_CMD}"
fi
echo

# --- Step 1: set release version ---
set_version "$NEW_VERSION"

# --- Step 2: commit release ---
run "git add '$VERSION_FILE'"
run "git commit -m 'Release ${NEW_VERSION}'"

# --- Step 3: tag release (annotated) ---
# # --- Step 3: tag release (annotated) ---
if [[ "$MAVEN_LOCAL" != "true" ]]; then
  run "git tag -a '${TAG_NAME}' -m '${NEW_VERSION}'"
else
  echo "[info] Skipping tag creation (--maven-local)"
fi

# --- Step 4 (optional): publish to Maven local ---
if [[ "$MAVEN_LOCAL" == "true" ]]; then
  echo "Publishing release ${NEW_VERSION} to Maven local..."
  run "${MAVEN_LOCAL_CMD}"
fi

# --- Step 5: push commit + tags ---
if [[ "$PUSH" == "true" ]]; then
  run "git push"
  run "git push --tags"
else
  echo "[info] Skipping push (--no-push or --maven-local)"
fi

# --- Step 6: bump to next snapshot ---
set_version "$NEXT_SNAPSHOT"

# --- Step 7: commit snapshot ---
run "git add '$VERSION_FILE'"
run "git commit -m 'Prepare next development version.'"

# --- Step 8: push snapshot commit ---
if [[ "$PUSH" == 'true' ]]; then
  run "git push"
else
  echo "[info] Skipping push of snapshot commit (--no-push or --maven-local)"
fi

echo "✅ Release ${NEW_VERSION} completed. Next development version: ${NEXT_SNAPSHOT}"
if [[ "$MAVEN_LOCAL" == "true" ]]; then
  echo "📦 Published ${NEW_VERSION} to Maven local (command: ${MAVEN_LOCAL_CMD})"
fi

