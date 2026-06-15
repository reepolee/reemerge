# Release script for Windows.
# Builds the native binary and publishes it as a GitHub Release.
# Version is auto-bumped (patch) only when the tag for the current version doesn't exist yet.
#
# Usage: .\release.ps1 [-Draft] [-Minor] [-Force]
#   -Draft  Create the release as a draft (default: published)
#   -Minor  Bump the minor version instead of the patch version (default: patch)
#   -Force  Skip version mismatch check (use when pushing ahead of remote)
#
# Prerequisites:
#   - gh CLI (https://cli.github.com) — authenticated via `gh auth login`
#   - git
#
# Workflow (run on each machine after pushing code):
#   1. macOS (first): bash release.sh     -> bumps version, creates tag + release, uploads
#   2. Linux:          bash release.sh     -> builds, uploads to existing release
#   3. Windows:        .\release.ps1      -> builds, uploads to existing release

param(
    [switch]$Draft,
    [switch]$Minor,
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$AppName = "reemerge"

# ──────────────────────────────────────────────
# Validate prerequisites
# ──────────────────────────────────────────────

if (-not (Get-Command "gh" -ErrorAction SilentlyContinue)) {
    Write-Error "gh CLI not found. Install it from https://cli.github.com/"
    exit 1
}

$authStatus = gh auth status 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Error "gh CLI is not authenticated. Run: gh auth login"
    exit 1
}

# ──────────────────────────────────────────────
# Read current version from Cargo.toml
# ──────────────────────────────────────────────

$cargoContent = Get-Content "Cargo.toml" -Raw
$versionMatch = [regex]::Match($cargoContent, 'version = "(\d+\.\d+\.\d+)"')
if (-not $versionMatch.Success) {
    Write-Error "Could not find version in Cargo.toml"
    exit 1
}

$version = $versionMatch.Groups[1].Value

# Detect Windows architecture
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $binaryName = "$AppName-windows-x64.exe" }
    'ARM64' { $binaryName = "$AppName-windows-arm64.exe" }
    default { Write-Error "Unsupported architecture: $arch"; exit 1 }
}

# ──────────────────────────────────────────────
# Detect code changes since last release
# ──────────────────────────────────────────────

git fetch --tags 2>$null | Out-Null

$latestTag = git describe --tags --abbrev=0 --match 'v*' 2>$null
$newCommits = 0

if ($latestTag) {
    # Verify local version matches the latest tag before proceeding.
    # If you run the release script without pulling first, versions will diverge.
    $tagVersion = $latestTag.TrimStart('v')
    if ($tagVersion -ne $version) {
        if ($Force) {
            Write-Host "  (Warning: local version $version differs from latest tag $tagVersion, proceeding with -Force)"
        } else {
            Write-Error "Local version ($version) differs from latest tag ($tagVersion). Run 'git pull' first to sync, or use -Force to override."
            exit 1
        }
    }

    $newCommits = [int](git rev-list HEAD "^$latestTag" --count 2>$null)
} else {
    # No prior tag → this is the first release ever
    $newCommits = 1
}

$tag = "v$version"
$doBump = $false

if ($newCommits -gt 0) {
    # Code has changed since last release → compute next version
    $parts = $version -split '\.'
    if ($Minor) {
        $newVersion = "$($parts[0]).$([int]$parts[1] + 1).0"
        $bumpType = "minor"
    } else {
        $newVersion = "$($parts[0]).$($parts[1]).$([int]$parts[2] + 1)"
        $bumpType = "patch"
    }
    $newTag = "v$newVersion"

    # If the bumped tag already exists on remote, another machine already bumped.
    # Skip the bump and just upload our binary to the existing release.
    $tagExists = git ls-remote --tags origin "refs/tags/$newTag" 2>$null | Select-String -Pattern "refs/tags/$newTag" -SimpleMatch
    if ($tagExists) {
        Write-Host "═══ reemerge release $newVersion for Windows ═══"
        Write-Host "  (Tag $newTag already exists on remote. Uploading binary only.)"
        $version = $newVersion
        $tag = $newTag
        $doBump = $false
    } else {
        Write-Host "═══ reemerge release $newVersion for Windows ═══"
        Write-Host "  (Bumping $bumpType from $version -> $newVersion, $newCommits commits since $latestTag)"

        $cargoContent = Get-Content "Cargo.toml" -Raw
        $cargoContent = $cargoContent -replace 'version = "\d+\.\d+\.\d+"', "version = `"$newVersion`""
        Set-Content "Cargo.toml" -Value $cargoContent

        $version = $newVersion
        $tag = $newTag
        $doBump = $true
    }
} else {
    # No code changes → just upload the binary
    Write-Host "═══ reemerge release $version for Windows ═══"
    Write-Host "  (No new commits since $latestTag. Uploading binary only.)"
    $doBump = $false
}

# ──────────────────────────────────────────────
# Build
# ──────────────────────────────────────────────

Write-Host "`n→ Building $binaryName..."
cargo build --release
Copy-Item ".\target\release\$AppName.exe" ".\$binaryName"

# ──────────────────────────────────────────────
# Commit version bump (first machine only)
# ──────────────────────────────────────────────

if ($doBump) {
    Write-Host "`n→ Committing version bump..."
    git add "Cargo.toml"
    git commit -m "Bump version to $version"
    Write-Host "  Committed: Bump version to $version"
}

# ──────────────────────────────────────────────
# Create and push git tag (first machine only)
# ──────────────────────────────────────────────

if ($doBump) {
    Write-Host "`n→ Tagging $tag..."

    $tagLocal = git rev-parse $tag 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  Tag $tag already exists locally."
    } else {
        git tag $tag
        Write-Host "  Created tag $tag locally."
    }

    Write-Host "  Pushing tag $tag to origin..."
    git push origin $tag
    Write-Host "  Pushing version bump commit..."
    git push origin HEAD
}

# ──────────────────────────────────────────────
# Create or upload to GitHub Release
# ──────────────────────────────────────────────

Write-Host "`n→ Publishing release $tag..."

$assetPath = ".\$binaryName"
$assetName = $binaryName
$releaseArgs = @()

$releaseExists = gh release view $tag 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Host "  Release $tag already exists. Uploading asset..."
    gh release upload $tag "$assetPath#$assetName" --clobber
} else {
    Write-Host "  Creating release $tag..."
    $releaseArgs = @(
        "release", "create", $tag,
        "$assetPath#$assetName",
        "--title", $tag,
        "--notes", "Release $tag"
    )
    if ($Draft) {
        $releaseArgs += "--draft"
        Write-Host "  (Draft mode)"
    }
    gh @releaseArgs
}

# ──────────────────────────────────────────────
# Clean up binary artifact
# ──────────────────────────────────────────────

Write-Host ""
Remove-Item ".\$binaryName" -Force -ErrorAction SilentlyContinue
Remove-Item "Cargo.lock" -Force -ErrorAction SilentlyContinue
Write-Host "  Cleaned up $binaryName and Cargo.lock"

# ──────────────────────────────────────────────
# Install locally (to PATH)
# ──────────────────────────────────────────────

Write-Host "`n→ Installing locally..."
$InstallDir = Join-Path $HOME "bin"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item ".\target\release\$AppName.exe" (Join-Path $InstallDir "$AppName.exe") -Force

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$Paths = $UserPath -split ";"
if ($Paths -notcontains $InstallDir) {
    $NewPath = if ([string]::IsNullOrWhiteSpace($UserPath)) {
        $InstallDir
    } else {
        "$UserPath;$InstallDir"
    }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    Write-Host "  Added $InstallDir to user PATH"
    Write-Host "  Restart terminal to use $AppName"
}

Write-Host "  Installed to $(Join-Path $InstallDir "$AppName.exe")"

# ──────────────────────────────────────────────
# Done
# ──────────────────────────────────────────────

$remoteUrl = git remote get-url origin
$repoPath = $remoteUrl -replace '.*github.com[/:]', '' -replace '\.git$', ''
Write-Host "`n✅ Done! Released $binaryName → $tag"
Write-Host "   View at: https://github.com/$repoPath/releases/tag/$tag"
