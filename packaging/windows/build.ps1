# Builds Oxbow for Windows and packages it into dist\: an installer (Inno Setup 6 or later) and a portable .zip.
# The release workflow runs this; it runs the same way locally:  pwsh packaging\windows\build.ps1
param(
    [string]$Out = (Join-Path $PSScriptRoot '..\..\dist')
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$version = (Select-String -Path (Join-Path $root 'crates\app\Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = Resolve-Path $Out

Push-Location $root
try {
    cargo build --release --locked -p oxbow
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally {
    Pop-Location
}
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
$release = Join-Path $targetDir 'release'

# Portable: the .exe, its licence and the README.
$staging = Join-Path ([IO.Path]::GetTempPath()) "oxbow-$version-windows-x64"
Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
New-Item -ItemType Directory $staging | Out-Null
Copy-Item (Join-Path $release 'oxbow.exe'), (Join-Path $root 'README.md') $staging
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $staging 'LICENSE.txt')
$zip = Join-Path $Out "oxbow-$version-windows-x64.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip
Remove-Item -Recurse -Force $staging

# Installer. ISCC.exe from PATH, or wherever Inno Setup (6 or later) was installed.
$installed = Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
    Get-ItemProperty -ErrorAction SilentlyContinue |
    Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -match '^Inno Setup' -and $_.PSObject.Properties['InstallLocation'] } |
    Sort-Object { [version]($_.DisplayVersion -replace '[^0-9.].*$', '') } -Descending |
    ForEach-Object { Join-Path $_.InstallLocation 'ISCC.exe' }
$iscc = @(
    (Get-Command iscc -ErrorAction SilentlyContinue).Source
    $installed
    foreach ($v in 7, 6) {
        "${env:ProgramFiles(x86)}\Inno Setup $v\ISCC.exe"
        "$env:ProgramFiles\Inno Setup $v\ISCC.exe"
        "$env:LOCALAPPDATA\Programs\Inno Setup $v\ISCC.exe"
    }
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup (ISCC.exe) was not found. Install Inno Setup 6 or later.' }
"Inno Setup: $iscc"
& $iscc /Qp "/DVersion=$version" "/DSource=$release" "/DOutputDir=$Out" (Join-Path $PSScriptRoot 'oxbow.iss')
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }

Get-ChildItem $Out -Filter "oxbow-$version-windows-*" | ForEach-Object { '{0}  {1:N1} MB' -f $_.Name, ($_.Length / 1MB) }
