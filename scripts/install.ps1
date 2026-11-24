#!/usr/bin/env pwsh
$ErrorActionPreference = "Stop"

$Repo = "sandy-sachin7/shard"
$Binary = "shard"

# Align target selection with .github/workflows/release.yml (Windows ships x86_64-windows.zip).
# Windows on ARM64 executes x86_64 binaries via built-in x64 emulation; for native ARM64, use `cargo install shard-cli`.
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -match "ARM64") {
    Write-Warning "Native ARM64 Windows archives are not published yet; installing x86_64-windows (runs via Windows x64 emulation). For a native ARM64 build, run 'cargo install shard-cli'."
}
$Target = "x86_64-windows"

# Fetch latest release
Write-Host "Fetching latest release..."
$Latest = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
if (-not $Latest) {
    Write-Error "Failed to detect latest release"
    exit 1
}

$Asset = "${Binary}-${Latest}-${Target}.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Latest/$Asset"
$ChecksumUrl = "$DownloadUrl.sha256"

# Isolated temporary working directory cleaned up reliably in `finally`
$WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) "shard-install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null

try {
    $AssetPath = Join-Path $WorkDir $Asset
    $ExtractDir = Join-Path $WorkDir "extract"

    Write-Host "Downloading $Binary $Latest ($Target)..."
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $AssetPath

    Write-Host "Verifying SHA-256 checksum..."
    $ChecksumLine = (Invoke-RestMethod $ChecksumUrl).Trim()
    $ExpectedHash = ($ChecksumLine -split '\s+')[0].ToLower()
    $ActualHash = (Get-FileHash $AssetPath -Algorithm SHA256).Hash.ToLower()

    if ($ExpectedHash -ne $ActualHash) {
        throw "Checksum mismatch! Expected: $ExpectedHash, Actual: $ActualHash"
    }

    Write-Host "Extracting..."
    Expand-Archive -Path $AssetPath -DestinationPath $ExtractDir -Force

    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\$Binary"
    Write-Host "Installing to $InstallDir..."
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $BinaryFile = Get-ChildItem -Path $ExtractDir -Filter "$Binary.exe" -Recurse | Select-Object -First 1
    if (-not $BinaryFile) {
        throw "Could not locate $Binary.exe inside downloaded archive"
    }
    Copy-Item -LiteralPath $BinaryFile.FullName -Destination (Join-Path $InstallDir "$Binary.exe") -Force

    # Add to User PATH if not already present
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $PathEntries = if ($UserPath) { $UserPath -split ';' } else { @() }
    if ($PathEntries -notcontains $InstallDir) {
        $NewPath = if ([string]::IsNullOrWhiteSpace($UserPath)) { $InstallDir } else { "$UserPath;$InstallDir" }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    }

    Write-Host ""
    Write-Host "✓ $Binary $Latest installed to $InstallDir"
    Write-Host "Restart your terminal and run 'shard --help' to get started."
}
finally {
    Remove-Item -LiteralPath $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
}
