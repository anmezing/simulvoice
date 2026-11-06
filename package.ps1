param([switch]$SkipInstaller, [switch]$DirectML)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Ensure-Cmake {
    if ($env:CMAKE -and (Test-Path $env:CMAKE)) { return }
    $cmd = Get-Command cmake -ErrorAction SilentlyContinue
    if ($cmd) { $env:CMAKE = $cmd.Source; return }
    $py = python -c 'import cmake,os;print(os.path.join(cmake.CMAKE_BIN_DIR,"cmake.exe"))' 2>$null
    if ($py -and (Test-Path $py)) { $env:CMAKE = $py }
}
function Ensure-Libclang {
    if ($env:LIBCLANG_PATH -and (Test-Path (Join-Path $env:LIBCLANG_PATH 'libclang.dll'))) { return }
    $clang = Get-Command clang -ErrorAction SilentlyContinue
    if ($clang) {
        $dir = Split-Path $clang.Source -Parent
        if (Test-Path (Join-Path $dir 'libclang.dll')) { $env:LIBCLANG_PATH = $dir; return }
    }
    foreach ($dir in @('C:\Program Files\LLVM\bin', 'D:\Program Files\LLVM\bin')) {
        if (Test-Path (Join-Path $dir 'libclang.dll')) { $env:LIBCLANG_PATH = $dir; return }
    }
}
function Ensure-MarianOrt {
    $dst = "target\release\onnxruntime_marian.dll"
    if (Test-Path $dst) { return }
    $dll = "$env:TEMP\ort131\pkg\runtimes\win-x64\native\onnxruntime.dll"
    if (-not (Test-Path $dll)) {
        $zip = "$env:TEMP\ort131\ort.nupkg"
        New-Item -ItemType Directory -Force "$env:TEMP\ort131" | Out-Null
        if (-not (Test-Path $zip)) {
            Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.31.0/microsoft.ml.onnxruntime.1.31.0.nupkg" -OutFile $zip -TimeoutSec 900
        }
        Expand-Archive $zip -DestinationPath "$env:TEMP\ort131\pkg" -Force
    }
    New-Item -ItemType Directory -Force "target\release" | Out-Null
    Copy-Item $dll $dst -Force
}
Ensure-Cmake
Ensure-Libclang
Ensure-MarianOrt

if ($DirectML) {
    if (-not $env:SHERPA_ONNX_LIB_DIR) {
        $candidate = 'C:\Users\Administrator\sherpa-dml\dist'
        if (Test-Path (Join-Path $candidate 'sherpa-onnx-c-api.lib')) {
            $env:SHERPA_ONNX_LIB_DIR = $candidate
        } else {
            throw "DirectML build requires SHERPA_ONNX_LIB_DIR (shared sherpa-onnx with DirectML)."
        }
    }
    cargo build --release --no-default-features --features directml
} else {
    cargo build --release
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = if ($DirectML) { "portable-directml" } else { "portable" }
$dist = Join-Path $PSScriptRoot "dist\$name"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item "target\release\simulvoice.exe" $dist -Force
Copy-Item "target\release\onnxruntime_marian.dll" $dist -Force
Copy-Item "README.md" $dist -Force
Copy-Item "README.en.md" $dist -Force
Copy-Item "LICENSE" "$dist\LICENSE.txt" -Force
if ($DirectML) {
    Get-ChildItem "target\release" -Filter "*.dll" | Copy-Item -Destination $dist -Force
}
$suffix = if ($DirectML) { "-directml" } else { "" }
$zip = Join-Path $PSScriptRoot "dist\simulvoice-$version$suffix-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$dist\*" -DestinationPath $zip -Force
Write-Host "Portable: $zip"

# Keep symbols per version (not shipped): needed to read crash dumps users send back
$sym = Join-Path $PSScriptRoot "dist\symbols\$version$suffix"
New-Item -ItemType Directory -Force -Path $sym | Out-Null
Copy-Item "target\release\simulvoice.pdb" $sym -Force
Copy-Item "target\release\simulvoice.exe" $sym -Force
Write-Host "Symbols: $sym"

if (-not $SkipInstaller -and -not $DirectML) {
    try {
        & (Join-Path $PSScriptRoot "build-installer.ps1")
    } catch {
        Write-Warning "Installer build skipped: $_"
    }
}

