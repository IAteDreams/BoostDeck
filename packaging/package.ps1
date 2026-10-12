# Builds the release and packages it into dist/:
#   BoostDeck-<version>-portable.zip  boostdeck.exe, README.txt, LICENSE.txt
#   BoostDeck-<version>-x64.msi       when the WiX Toolset (wix.exe, v5) is on PATH
#
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File packaging/package.ps1
# Unsigned: see README "Packaging".

param([switch]$SkipBuild)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $SkipBuild) {
    cargo build --release -p app
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$exe = Join-Path $root "target\release\boostdeck.exe"
if (-not (Test-Path $exe)) { throw "no release build at $exe" }

$dist = Join-Path $root "dist"
$stage = Join-Path $dist "BoostDeck-$version"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe $stage
(Get-Content -Raw packaging\README.txt).Replace("{VERSION}", $version) |
    Set-Content -Encoding utf8 (Join-Path $stage "README.txt")
Copy-Item LICENSE (Join-Path $stage "LICENSE.txt")

$zip = Join-Path $dist "BoostDeck-$version-portable.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
"Portable: $zip"

if (Get-Command wix -ErrorAction SilentlyContinue) {
    $msi = Join-Path $dist "BoostDeck-$version-x64.msi"
    wix build packaging\boostdeck.wxs -arch x64 -d Version=$version -d StageDir=$stage -o $msi
    if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
    Remove-Item -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($msi, ".wixpdb"))
    "Installer: $msi"
} else {
    "Installer: skipped - the WiX Toolset (wix.exe) is not on PATH. Install it with: dotnet tool install --global wix --version 5.0.2"
}
