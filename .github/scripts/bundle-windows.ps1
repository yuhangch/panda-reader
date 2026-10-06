# Usage: bundle-windows.ps1 <target-triple> <arch-label>
#   dist/panda-reader-<version>-windows-<arch>.zip
#   dist/panda-reader-<version>-windows-<arch>-setup.exe
$ErrorActionPreference = 'Stop'

$Target = $args[0]
$Arch = $args[1]
$Root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $Root

$Version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$VersionCore = ($Version -split '[-+]', 2)[0]
$VersionInfoVersion = "${VersionCore}.0"
$Name = "panda-reader-$Version-windows-$Arch"
$Stage = "dist/$Name"

Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage, dist | Out-Null

Copy-Item "target/$Target/release/panda-reader.exe" "$Stage/panda-reader.exe"
Copy-Item "target/$Target/release/panda-reader-updater.exe" "$Stage/panda-reader-updater.exe"
Copy-Item LICENSE "$Stage/LICENSE.txt"
Copy-Item README.md "$Stage/README.md"
Copy-Item THIRD_PARTY_NOTICES.md "$Stage/THIRD_PARTY_NOTICES.md"
Copy-Item THIRD_PARTY_LICENSES.txt "$Stage/THIRD_PARTY_LICENSES.txt"
New-Item -ItemType Directory -Force -Path "$Stage/THIRD_PARTY_ASSET_LICENSES/fonts" | Out-Null
Copy-Item apps/panda-reader/assets/TTY7-LICENSE "$Stage/THIRD_PARTY_ASSET_LICENSES/TTY7-LICENSE.txt"
Copy-Item apps/panda-reader/assets/fonts/*LICENSE* "$Stage/THIRD_PARTY_ASSET_LICENSES/fonts/"

Compress-Archive -Path "$Stage/*" -DestinationPath "dist/$Name.zip" -Force
if (-not (Test-Path "dist/$Name.zip") -or (Get-Item "dist/$Name.zip").Length -eq 0) {
    throw "Portable archive was not created"
}
if (-not (Test-Path "$Stage/panda-reader.exe") -or (Get-Item "$Stage/panda-reader.exe").Length -eq 0) {
    throw "Staged executable is missing"
}
if (-not (Test-Path "$Stage/panda-reader-updater.exe") -or (Get-Item "$Stage/panda-reader-updater.exe").Length -eq 0) {
    throw "Staged updater helper is missing"
}
$ZipCheck = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path "dist/$Name.zip").Path)
try {
    if (-not ($ZipCheck.Entries | Where-Object { $_.FullName -eq 'panda-reader.exe' })) {
        throw "Portable archive does not contain panda-reader.exe"
    }
    if (-not ($ZipCheck.Entries | Where-Object { $_.FullName -eq 'panda-reader-updater.exe' })) {
        throw "Portable archive does not contain panda-reader-updater.exe"
    }
} finally {
    $ZipCheck.Dispose()
}

$Iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $Iscc) {
    $Iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
}
if (-not (Test-Path $Iscc)) {
    throw "Inno Setup compiler (ISCC.exe) not found"
}

& $Iscc `
    "/DAppVersion=$Version" `
    "/DVersionInfoVersion=$VersionInfoVersion" `
    "/DStageDir=$((Resolve-Path $Stage).Path)" `
    "/DOutputDir=$((Resolve-Path dist).Path)" `
    "/DOutputName=$Name-setup" `
    .github/scripts/windows-installer.iss
if ($LASTEXITCODE -ne 0) {
    throw "ISCC exited with $LASTEXITCODE"
}
if (-not (Test-Path "dist/$Name-setup.exe") -or (Get-Item "dist/$Name-setup.exe").Length -eq 0) {
    throw "Windows installer was not created"
}

Write-Host "OK dist/$Name.zip"
Write-Host "OK dist/$Name-setup.exe"
