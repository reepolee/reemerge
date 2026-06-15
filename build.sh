#!/usr/bin/env bash
# Build script for macOS and Linux.
# Builds the native binary and installs it locally.
#
# Usage: bash build.sh [--no-install]
#   --no-install  Skip local install (just build the binary)

set -euo pipefail

APP="reemerge"
no_install=false

for arg in "$@"; do
	case "$arg" in
		--no-install) no_install=true ;;
	esac
done

# ──────────────────────────────────────────────
# Read current version from Cargo.toml
# ──────────────────────────────────────────────

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
# Build
# ──────────────────────────────────────────────

echo "═══ reemerge v$version build for $os ($arch) ═══"

echo ""
echo "→ Building $binary_name..."
cargo build --release

cp "./target/release/$APP" "./$binary_name"
echo "  Created $binary_name"

# ──────────────────────────────────────────────
# Verify
# ──────────────────────────────────────────────

echo ""
echo "→ Verifying binary..."
"./$binary_name" --version
echo "  ✓ $binary_name is ready"

# ──────────────────────────────────────────────
# Install locally (skip with --no-install)
# ──────────────────────────────────────────────

if [ "$no_install" = false ]; then
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
fi

# ──────────────────────────────────────────────
# Done
# ──────────────────────────────────────────────

echo ""
echo "✅ Done! Built reemerge v$version ($binary_name)"
