param(
    [string]$InstallDir = "",
    [string]$DataDir = "",
    [switch]$Machine,
    [switch]$NoShortcut,
    [switch]$NoRegistry
)
$ErrorActionPreference = "Stop"
$src = $PSScriptRoot

if (-not $InstallDir) {
    if ($Machine) { $InstallDir = Join-Path $env:ProgramFiles "SimulVoice" }
    else { $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\SimulVoice" }
}
$dataIsCustom = $true
if (-not $DataDir) { $DataDir = Join-Path $InstallDir "data"; $dataIsCustom = $false }

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -Path (Join-Path $src "simulvoice.exe") -Destination $InstallDir -Force
Copy-Item -Path (Join-Path $src "README.md") -Destination $InstallDir -Force
foreach ($name in @("simulvoice.toml", "keys.toml")) {
    $p = Join-Path $src $name
    if (Test-Path $p) { Copy-Item -Path $p -Destination $InstallDir -Force }
}
Copy-Item -Path (Join-Path $src "uninstall.ps1") -Destination $InstallDir -Force

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
if ($dataIsCustom) {
    Set-Content -Path (Join-Path $InstallDir "simulvoice.data-dir") -Value $DataDir -Encoding ascii
} else {
    Set-Content -Path (Join-Path $InstallDir "simulvoice.data-dir") -Value "data" -Encoding ascii
}

if (-not $NoShortcut) {
    $ws = New-Object -ComObject WScript.Shell
    $lnk = Join-Path ([Environment]::GetFolderPath("Programs")) "SimulVoice.lnk"
    $sc = $ws.CreateShortcut($lnk)
    $sc.TargetPath = Join-Path $InstallDir "simulvoice.exe"
    $sc.WorkingDirectory = $InstallDir
    $sc.Description = "SimulVoice"
    $sc.Save()
}

if (-not $NoRegistry) {
    $root = if ($Machine) { "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall" } else { "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" }
    $key = Join-Path $root "SimulVoice"
    New-Item -Path $key -Force | Out-Null
    Set-ItemProperty -Path $key -Name DisplayName -Value "SimulVoice"
    Set-ItemProperty -Path $key -Name DisplayVersion -Value "0.12.0"
    Set-ItemProperty -Path $key -Name InstallLocation -Value $InstallDir
    Set-ItemProperty -Path $key -Name DisplayIcon -Value (Join-Path $InstallDir "simulvoice.exe")
    $uninstallCmd = 'powershell -ExecutionPolicy Bypass -File "{0}"' -f (Join-Path $InstallDir 'uninstall.ps1')
    Set-ItemProperty -Path $key -Name UninstallString -Value $uninstallCmd
    Set-ItemProperty -Path $key -Name NoModify -Value 1
    Set-ItemProperty -Path $key -Name NoRepair -Value 1
}

Write-Host "Installed to: $InstallDir"
Write-Host "Data directory: $DataDir"
if (-not $NoShortcut) { Write-Host "Start menu shortcut created" }




