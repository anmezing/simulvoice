param([switch]$SkipInstaller)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

cargo build --release
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$dist = Join-Path $PSScriptRoot "dist\portable"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item "target\release\simulvoice.exe" $dist -Force
Copy-Item "README.md" $dist -Force
if (Test-Path "examples\simulvoice.toml") { Copy-Item "examples\simulvoice.toml" "$dist\simulvoice.toml" -Force }
$zip = Join-Path $PSScriptRoot "dist\simulvoice-$version-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$dist\*" -DestinationPath $zip -Force
Write-Host "Portable: $zip"

if (-not $SkipInstaller) {
    try {
        & (Join-Path $PSScriptRoot "build-installer.ps1")
    } catch {
        Write-Warning "Installer build skipped: $_"
    }
}
