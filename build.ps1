# Build script for Windows.
# Builds the native binary and installs it locally.
#
# Usage: .\build.ps1 [-NoInstall]
#   -NoInstall  Skip local install (just build the binary)

param(
    [switch]$NoInstall
)

$ErrorActionPreference = "Stop"
$AppName = "reemerge"

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

# ──────────────────────────────────────────────
# Detect Windows architecture
# ──────────────────────────────────────────────

$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $binaryName = "$AppName-windows-x64.exe" }
    'ARM64' { $binaryName = "$AppName-windows-arm64.exe" }
    default { Write-Error "Unsupported architecture: $arch"; exit 1 }
}

# ──────────────────────────────────────────────
# Build
# ──────────────────────────────────────────────

Write-Host "═══ reemerge v$version build for Windows ($arch) ═══"

Write-Host "`n→ Building $binaryName..."
cargo build --release

Copy-Item ".\target\release\$AppName.exe" ".\$binaryName"
Write-Host "  Created $binaryName"

# ──────────────────────────────────────────────
# Verify
# ──────────────────────────────────────────────

Write-Host "`n→ Verifying binary..."
& ".\$binaryName" --version
Write-Host "  ✓ $binaryName is ready"

# ──────────────────────────────────────────────
# Install locally (skip with -NoInstall)
# ──────────────────────────────────────────────

if (-not $NoInstall) {
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
}

# ──────────────────────────────────────────────
# Done
# ──────────────────────────────────────────────

Write-Host "`n✅ Done! Built reemerge v$version ($binaryName)"
