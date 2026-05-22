#!/usr/bin/env pwsh
$ErrorActionPreference = "Stop"

$Repo = "sandy-sachin7/shard"
$Binary = "shard"

# Detect architecture
$Arch = (Get-CimInstance Win32_ComputerSystem).SystemType
if ($Arch -match "ARM64") {
    $Target = "aarch64-windows"
} else {
    $Target = "x86_64-windows"
}

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

$TempDir = [System.IO.Path]::GetTempPath()
$AssetPath = Join-Path $TempDir $Asset

Write-Host "Downloading $Binary $Latest ($Target)..."
Invoke-WebRequest -Uri $DownloadUrl -OutFile $AssetPath

Write-Host "Verifying checksum..."
$ExpectedHash = (Invoke-RestMethod $ChecksumUrl).Split(' ')[0]
$ActualHash = (Get-FileHash $AssetPath -Algorithm SHA256).Hash.ToLower()

if ($ExpectedHash -ne $ActualHash) {
    Write-Error "Checksum mismatch! Expected: $ExpectedHash, Actual: $ActualHash"
    exit 1
}

Write-Host "Extracting..."
$ExtractDir = Join-Path $TempDir "${Binary}-${Latest}-${Target}"
Expand-Archive -Path $AssetPath -DestinationPath $ExtractDir -Force

$InstallDir = [Environment]::GetFolderPath("ProgramFiles")
$InstallPath = Join-Path $InstallDir $Binary

Write-Host "Installing to $InstallPath..."
if (-not (Test-Path $InstallPath)) {
    New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null
}
Copy-Item (Join-Path $ExtractDir "$Binary.exe") $InstallPath -Force

# Add to PATH if not already
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallPath*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallPath", "User")
}

Remove-Item $AssetPath -Force
Remove-Item $ExtractDir -Recurse -Force

Write-Host ""
Write-Host "✓ $Binary $Latest installed to $InstallPath"
Write-Host "Run 'shard --help' to get started."
