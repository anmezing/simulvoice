param([string]$Iscc = "")
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not $Iscc) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    )
    foreach ($c in $candidates) { if (Test-Path $c) { $Iscc = $c; break } }
}
if (-not $Iscc -or -not (Test-Path $Iscc)) {
    throw "ISCC.exe not found. Install Inno Setup 6 first."
}

cargo build --release

$stage = "dist\stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item "target\release\simulvoice.exe" $stage
Copy-Item "README.md" $stage
if (Test-Path "examples\simulvoice.toml") { Copy-Item "examples\simulvoice.toml" "$stage\simulvoice.toml" }
Set-Content -Path "$stage\simulvoice.data-dir" -Value "data" -Encoding ascii -NoNewline

& $Iscc "simulvoice.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
Write-Host "Installer: dist\SimulVoiceSetup-0.15.0.exe"




