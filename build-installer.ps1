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
if (Test-Path "target\release\onnxruntime_marian.dll") { Copy-Item "target\release\onnxruntime_marian.dll" $stage }
Copy-Item "README.md" $stage
Copy-Item "README.en.md" $stage
Copy-Item "LICENSE" "$stage\LICENSE.txt"
# simulvoice.exe is a GUI-subsystem app: "&" returns before it finishes, so wait for it explicitly
$p = Start-Process "target\release\simulvoice.exe" -ArgumentList "write-icon", "$stage\simulvoice.ico" -Wait -PassThru -NoNewWindow
if ($p.ExitCode -ne 0 -or -not (Test-Path "$stage\simulvoice.ico")) { throw "write-icon failed (exit $($p.ExitCode))" }
Set-Content -Path "$stage\simulvoice.data-dir" -Value "data" -Encoding ascii -NoNewline

& $Iscc "simulvoice.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
Write-Host "Installer: dist\SimulVoiceSetup-0.46.0.exe"


































