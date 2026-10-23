# Build a portable Windows package: exe + README + optional config.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
cargo build --release
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$dist = Join-Path $PSScriptRoot "dist\simulvoice-$version"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item "target\release\simulvoice.exe" $dist
Copy-Item "README.md" $dist
if (Test-Path "examples\simulvoice.toml") { Copy-Item "examples\simulvoice.toml" "$dist\simulvoice.toml" }
foreach ($extra in @("simulvoice.toml")) {
    if (Test-Path $extra) { Copy-Item $extra $dist }
}
$zip = "$dist.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$dist\*" -DestinationPath $zip
Write-Host "打包完成: $zip"




