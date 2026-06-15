#!/usr/bin/env bash
# Release script for macOS and Linux.
# Builds the native binary for the current platform and publishes it as a GitHub Release.
# Version is auto-bumped (patch) only when the tag for the current version doesn't exist yet.
#
# Usage: bash release.sh [--draft] [--minor] [--force]
#   --draft  Create the release as a draft (default: published)
#   --minor  Bump the minor version instead of the patch version (default: patch)
#   --force  Skip version mismatch check (use when pushing ahead of remote)
#
# Prerequisites:
#   - gh CLI (https://cli.github.com) — authenticated via `gh auth login`
#   - git
#
# Workflow (run on each machine after pushing code):
#   1. macOS (first): bash release.sh    → bumps version, creates tag + release, uploads
#   2. Linux:          bash release.sh    → builds, uploads to existing release
#   3. Windows:        .\release.ps1     → builds, uploads to existing release

set -euo pipefail

APP="reemerge"

# ──────────────────────────────────────────────
# Validate prerequisites
# ──────────────────────────────────────────────

if ! command -v gh &>/dev/null; then
	echo "ERROR: gh CLI not found. Install it from https://cli.github.com/" >&2
	exit 1
fi

if ! gh auth status &>/dev/null; then
	echo "ERROR: gh CLI is not authenticated. Run: gh auth login" >&2
	exit 1
fi

# ──────────────────────────────────────────────
# Parse flags
# ──────────────────────────────────────────────

draft_flag=""
minor_bump=false
force_flag=false

for arg in "$@"; do
	case "$arg" in
		--draft) draft_flag="--draft" ;;
		--minor) minor_bump=true ;;
		--force) force_flag=true ;;
	esac
done

if [ -n "$draft_flag" ]; then
	echo "  (Draft mode)"
fi
if [ "$minor_bump" = true ]; then
	echo "  (Minor bump)"
fi
if [ "$force_flag" = true ]; then
	echo "  (Force mode — version mismatch check skipped)"
fi

# ──────────────────────────────────────────────
# Read current version from Cargo.toml
# ──────────────────────────────────────────────

bump_patch() {
	local current="$1"
	local major="${current%%.*}"
	local rest="${current#*.}"
	local minor="${rest%%.*}"
	local patch="${rest#*.}"
	local new_patch=$((patch + 1))
	echo "$major.$minor.$new_patch"
}

bump_minor() {
	local current="$1"
	local major="${current%%.*}"
	local rest="${current#*.}"
	local minor="${rest%%.*}"
	local new_minor=$((minor + 1))
	echo "$major.$new_minor.0"
}

version=$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)
if [ -z "$version" ]; then
	echo "ERROR: Could not find version in Cargo.toml" >&2
	exit 1
fi

os="$(uname -s)"
arch="$(uname -m)"

# ──────────────────────────────────────────────
# Determine binary name for this platform
# ──────────────────────────────────────────────

case "$os" in
	Darwin)
		case "$arch" in
			arm64|aarch64) binary_name="${APP}-macos-arm64" ;;
			x86_64)       binary_name="${APP}-macos-x64" ;;
			*)            echo "Unsupported arch: $arch" >&2; exit 1 ;;
		esac
		;;
	Linux)
		case "$arch" in
			x86_64|amd64)      binary_name="${APP}-linux-x64" ;;
			arm64|aarch64)     binary_name="${APP}-linux-arm64" ;;
			*)                 echo "Unsupported arch: $arch" >&2; exit 1 ;;
		esac
		;;
	*)
		echo "Unsupported OS: $os" >&2
		exit 1
		;;
esac

# ──────────────────────────────────────────────
# Detect code changes since last release
# ──────────────────────────────────────────────

git fetch --tags 2>/dev/null || true
latest_tag=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || echo "")

if [ -n "$latest_tag" ]; then
	# Verify local version matches the latest tag before proceeding.
	# If you run the release script without pulling first, versions will diverge.
	tag_version="${latest_tag#v}"
	if [ "$tag_version" != "$version" ]; then
		if [ "$force_flag" = true ]; then
			echo "  (Warning: local version $version differs from latest tag $tag_version, proceeding with --force)"
		else
			echo "ERROR: Local version ($version) differs from latest tag ($tag_version)." >&2
			echo "  Run 'git pull' first to sync, or use --force to override." >&2
			exit 1
		fi
	fi

	new_commits=$(git rev-list HEAD "^$latest_tag" --count 2>/dev/null || echo "0")
else
	# No prior tag → this is the first release ever
	new_commits=1
fi

tag="v$version"

if [ "$new_commits" -gt 0 ]; then
	# Code has changed since last release → compute next version
	if [ "${minor_bump:-false}" = true ]; then
		new_version=$(bump_minor "$version")
		bump_type="minor"
	else
		new_version=$(bump_patch "$version")
		bump_type="patch"
	fi
	new_tag="v$new_version"

	# If the bumped tag already exists on remote, another machine already bumped.
	# Skip the bump and just upload our binary to the existing release.
	if git ls-remote --tags origin "refs/tags/$new_tag" 2>/dev/null | grep -q .; then
		echo "═══ reemerge release $new_version for $os ($arch) ═══"
		echo "  (Tag $new_tag already exists on remote. Uploading binary only.)"
		version="$new_version"
		tag="$new_tag"
		do_bump=false
	else
		echo "═══ reemerge release $new_version for $os ($arch) ═══"
		echo "  (Bumping $bump_type from $version → $new_version, $new_commits commits since $latest_tag)"

		# Update Cargo.toml
		sed -i '' "s/version = \"$version\"/version = \"$new_version\"/" Cargo.toml 2>/dev/null || \
		sed -i "s/version = \"$version\"/version = \"$new_version\"/" Cargo.toml

		version="$new_version"
		tag="$new_tag"
		do_bump=true
	fi
else
	# No code changes → just upload the binary
	echo "═══ reemerge release $version for $os ($arch) ═══"
	echo "  (No new commits since $latest_tag. Uploading binary only.)"
	do_bump=false
fi


# ──────────────────────────────────────────────
# Run tests
# ──────────────────────────────────────────────

echo ""
echo "→ Running tests..."
cargo test

# ──────────────────────────────────────────────
# Build
# ──────────────────────────────────────────────

echo ""
echo "→ Building $binary_name..."
cargo build --release
cp "./target/release/$APP" "./$binary_name"
file "./$binary_name"

# ──────────────────────────────────────────────
# Commit version bump (first machine only)
# ──────────────────────────────────────────────

if [ "$do_bump" = true ]; then
	echo ""
	echo "→ Committing version bump..."
	git add Cargo.toml
	git commit -m "Bump version to $version"
	echo "  Committed: Bump version to $version"
fi

# ──────────────────────────────────────────────
# Create and push git tag (first machine only)
# ──────────────────────────────────────────────

if [ "$do_bump" = true ]; then
	echo ""
	echo "→ Tagging $tag..."

	if git rev-parse "$tag" >/dev/null 2>&1; then
		echo "  Tag $tag already exists locally."
	else
		git tag "$tag"
		echo "  Created tag $tag locally."
	fi

	echo "  Pushing tag $tag to origin..."
	git push origin "$tag"
	echo "  Pushing version bump commit..."
	git push origin HEAD
fi

# ──────────────────────────────────────────────
# Create or upload to GitHub Release
# ──────────────────────────────────────────────

echo ""
echo "→ Publishing release $tag..."

asset_path="./$binary_name"
asset_name="$binary_name"

if gh release view "$tag" >/dev/null 2>&1; then
	echo "  Release $tag already exists. Uploading asset..."
	gh release upload "$tag" "$asset_path#$asset_name" --clobber
else
	echo "  Creating release $tag..."
	gh release create "$tag" \
		"$asset_path#$asset_name" \
		--title "$tag" \
		--notes "Release $tag" \
		$draft_flag
fi

# ──────────────────────────────────────────────
# Clean up binary artifact
# ──────────────────────────────────────────────

echo ""
rm -f "./$binary_name"
rm -f Cargo.lock
echo "  Cleaned up $binary_name and Cargo.lock"

# ──────────────────────────────────────────────
# Install locally (to PATH)
# ──────────────────────────────────────────────

echo ""
echo "→ Installing locally..."
install_dir="$HOME/.local/bin"
mkdir -p "$install_dir"
cp "./target/release/$APP" "$install_dir/$APP"
chmod +x "$install_dir/$APP"

if ! echo ":$PATH:" | grep -q ":$install_dir:"; then
	shell_rc=""
	if [ -n "${ZSH_VERSION:-}" ]; then
		shell_rc="$HOME/.zshrc"
	elif [ -n "${BASH_VERSION:-}" ]; then
		shell_rc="$HOME/.bashrc"
	else
		shell_rc="$HOME/.profile"
	fi

	if ! grep -Fq "$install_dir" "$shell_rc" 2>/dev/null; then
		{
			echo
			echo "export PATH=\"$install_dir:\$PATH\""
		} >> "$shell_rc"
		echo "  Added $install_dir to PATH in $shell_rc"
	fi
	echo "  Restart shell or run: export PATH=\"$install_dir:\$PATH\""
fi

echo "  Installed to $install_dir/$APP"

# ──────────────────────────────────────────────
# Done
# ──────────────────────────────────────────────

echo ""
echo "✅ Done! Released $binary_name → $tag"
echo "   View at: https://github.com/$(git remote get-url origin | sed -E 's|.*github.com[/:]||; s|\.git$||')/releases/tag/$tag"
