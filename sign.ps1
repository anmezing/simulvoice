param(
    [string]$CertPath = "",
    [string]$Password = "",
    [string]$Thumbprint = "",
    [string]$Timestamp = "http://timestamp.digicert.com"
)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not $CertPath -and -not $Thumbprint) {
    throw "Provide -CertPath (PFX) or -Thumbprint (certificate installed in CurrentUser\\My)."
}

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match "\\x64\\" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signtool) { throw "signtool.exe not found. Install the Windows SDK." }

$files = @("target\release\simulvoice.exe")
$files += Get-ChildItem "dist" -Filter "SimulVoiceSetup-*.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName
foreach ($file in $files) {
    if (-not (Test-Path $file)) { continue }
    if ($Thumbprint) {
        $args = @("sign", "/sha1", $Thumbprint, "/tr", $Timestamp, "/td", "sha256", "/fd", "sha256")
    } else {
        $args = @("sign", "/f", $CertPath, "/tr", $Timestamp, "/td", "sha256", "/fd", "sha256")
        if ($Password) { $args += @("/p", $Password) }
    }
    $args += $file
    & $signtool.FullName @args
    if ($LASTEXITCODE -ne 0) { throw "signing failed: $file" }
    Write-Host "Signed: $file"
}
