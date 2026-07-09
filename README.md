<img src="/github-reepolee.svg" style="margin-bottom:1rem; width:200px">

# reemerge

An interactive cherry-pick tool for preparing hand-selected branches for pull requests. Choose which commits — and even which files within those commits — to bring into a new PR branch.

## Features

- **Interactive branch selection** — fuzzy-select from local and remote branches
- **Commit-by-commit selection** — pick individual commits to include
- **File-level granularity** — select which files from each commit to apply
- **Diff preview** — preview changes before applying with `--diff` or interactive prompt
- **Merge commit handling** — automatically retries with `-m 1` when a merge commit is detected
- **Automatic branch creation** — creates a new PR branch from your target branch
- **Optional push** — prompts to push the new branch to origin when ready
- **Colorful terminal UI** — rich colored output with icons and progress indicators

## Installation

### Quick install

**macOS / Linux:**

```bash
curl -fsSL https://raw.githubusercontent.com/reepolee/reemerge/main/install.sh | bash
```

**Windows:**

```powershell
irm https://raw.githubusercontent.com/reepolee/reemerge/main/install.ps1 | iex
```

The script detects your OS and architecture, downloads the correct binary from the latest GitHub Release, and adds it to your PATH.

Or download a binary directly from the [latest release](https://github.com/reepolee/reemerge/releases/latest).

### Build from source

Requires [Rust](https://rustup.rs/) (edition 2024).

```bash
cargo build --release
# Binary at ./target/release/reemerge
```

**macOS / Linux:**

```bash
./build.sh
# Produces reemerge-macos-arm64 (or -x64 / -linux-x64 / -linux-arm64) and installs to ~/.local/bin/
```

**Windows:**

```powershell
.\build.ps1
# Produces reemerge-windows-x64.exe and installs to ~\bin\
```

Pass `--no-install` / `-NoInstall` to skip the local install step (useful for CI).

## Usage

Run `reemerge` inside any git repository:

```bash
reemerge
```

### Interactive workflow

1. **Select target branch** — the branch you want to PR into (default: `main`)
2. **Select source branch** — the branch with the changes (default: `develop`)
3. **Pick commits** — multi-select the commits from the source branch that aren't on the target
4. **Preview diffs** — optionally review changes before applying
5. **Select files per commit** — choose which files from each commit to include
6. **Name your PR branch** — auto-suggested as `pr/<source-branch-name>`
7. **Confirm and apply** — files are cherry-picked onto the new branch
8. **Push** — optionally push the new branch to origin

### Options

| Flag | Description |
|------|-------------|
| `--diff` | Always show diff preview for selected commits (skips the prompt) |
| `--version`, `-V` | Print the bare version number and exit |

### Example walkthrough

```bash
# Start the interactive tool
reemerge

# Output:
# ╔═══════════════════════════════════════════╗
# ║           PR Prep - Cherry Pick           ║
# ║    Prepare hand-selected branches for PRs ║
# ╚═══════════════════════════════════════════╝
#
#   ✓ Repository: /path/to/your/project
#
#   ⟳ Fetching branches...
#   ✓ Found 12 branches
#
#   (interactive menus follow...)
```

### Use case: partial cherry-pick

If you have a branch with 5 commits touching 20 files but you only want 3 specific commits (and only certain files from them), reemerge lets you:

1. Select only those 3 commits
2. For each commit, deselect files you don't want
3. Apply just the selected changes to a clean branch off `main`

This is useful for splitting a large branch into smaller, focused PRs.

## Development

This is a Rust project. To test locally without releasing:

```bash
cargo build --release
cp target/release/reemerge ~/.local/bin/   # macOS/Linux
```

### Release workflow

Releases are cut from a single machine (the Mac mini). `release.sh` cross-builds
**all six targets** and publishes them as one GitHub Release:

- macOS arm64/x64 (native `cargo build`)
- Linux x64/arm64 (`cargo zigbuild`)
- Windows x64/arm64 (`cargo xwin build`)

```bash
bash release.sh            # bump, tag, cross-build all targets, publish
bash release.sh --minor    # bump the minor version instead of patch
bash release.sh --draft    # publish the release as a draft
```

One-time setup on the Mac:

```bash
brew install zig
cargo install cargo-zigbuild cargo-xwin
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-pc-windows-msvc
```
