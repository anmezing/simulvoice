param([switch]$KeepData)
$ErrorActionPreference = "SilentlyContinue"
$dir = Split-Path -Parent $PSCommandPath
$marker = Join-Path $dir "simulvoice.data-dir"
$raw = if (Test-Path $marker) { (Get-Content $marker -Raw).Trim() } else { "data" }
$data = if ([System.IO.Path]::IsPathRooted($raw)) { $raw } else { Join-Path $dir $raw }
$inside = $data.StartsWith($dir, [System.StringComparison]::OrdinalIgnoreCase)

Remove-Item (Join-Path ([Environment]::GetFolderPath("Programs")) "SimulVoice.lnk") -Force
Remove-Item "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\SimulVoice" -Recurse -Force
Remove-Item "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\SimulVoice" -Recurse -Force

if (-not $KeepData -and (Test-Path $data)) { Remove-Item $data -Recurse -Force }

if ($KeepData -and $inside) {
    Get-ChildItem $dir -Exclude "data", "uninstall.ps1" | Remove-Item -Recurse -Force
    Write-Host "Removed program files; data kept at $data"
} else {
    Get-ChildItem $dir -Exclude "uninstall.ps1" | Remove-Item -Recurse -Force
    $cmd = '/c timeout /t 2 >nul & rd /s /q "' + $dir + '"'
    Start-Process -FilePath cmd.exe -ArgumentList $cmd -WindowStyle Hidden
    Write-Host "SimulVoice removed"
}
