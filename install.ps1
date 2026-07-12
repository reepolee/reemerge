# Install reemerge from the latest GitHub Release.
# Usage: irm https://raw.githubusercontent.com/reepolee/reemerge/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$App = "reemerge"
$Owner = "reepolee"
$Repo = "reemerge"
$InstallDir = Join-Path $HOME "bin"
$Target = Join-Path $InstallDir "$App.exe"

# ──────────────────────────────────────────────
# Detect architecture
# ──────────────────────────────────────────────

$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
	"AMD64" { $AssetName = "$App-windows-x64.exe" }
	"ARM64" { $AssetName = "$App-windows-arm64.exe" }
	"x86" {
		# A 32-bit shell on 64-bit Windows reports x86; check the native arch.
		if ($env:PROCESSOR_ARCHITEW6432 -eq "ARM64") {
			$AssetName = "$App-windows-arm64.exe"
		}
		else {
			$AssetName = "$App-windows-x64.exe"
		}
	}
	default {
		Write-Error "Unsupported architecture: $arch"
		return
	}
}

# ──────────────────────────────────────────────
# Download
# ──────────────────────────────────────────────

$DownloadUrl = "https://github.com/$Owner/$Repo/releases/latest/download/$AssetName"
$TmpFile = Join-Path ([System.IO.Path]::GetTempPath()) $AssetName

Write-Host "-> Downloading $AssetName..."
try {
	Invoke-WebRequest -Uri $DownloadUrl -OutFile $TmpFile -UseBasicParsing
}
catch {
	Write-Error "Failed to download ${DownloadUrl}: $_"
	return
}

# ──────────────────────────────────────────────
# Install
# ──────────────────────────────────────────────

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Move-Item -Path $TmpFile -Destination $Target -Force

Write-Host "  Installed to $Target"

# ──────────────────────────────────────────────
# PATH check
# ──────────────────────────────────────────────

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$Paths = $UserPath -split ";"

if ($Paths -notcontains $InstallDir) {
	$NewPath = if ([string]::IsNullOrWhiteSpace($UserPath)) {
		$InstallDir
	}
	else {
		"$UserPath;$InstallDir"
	}

	[Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
	Write-Host "  Added $InstallDir to user PATH"
}
else {
	Write-Host "  $InstallDir already in PATH"
}

# ──────────────────────────────────────────────
# Verify
# ──────────────────────────────────────────────

Write-Host ""
Write-Host "reemerge installed!"
& $Target --version

Write-Host ""
Write-Host "Restart your terminal (or open a new one) to use reemerge."
