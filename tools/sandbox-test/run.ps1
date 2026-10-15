<#
.SYNOPSIS
  Run Rustshot's clean-system test in Windows Sandbox. See README.md.

.EXAMPLE
  .\tools\sandbox-test\run.ps1 -RunId 123456789 -Old D:\rustshot-0.2.0-setup.exe
  .\tools\sandbox-test\run.ps1 -Artifacts D:\ci-artifacts -Old D:\rustshot-0.2.0-setup.exe -Only v030
#>
param(
    # A folder with the CI's Windows artifacts (rustshot-windows-installer\,
    # rustshot-windows-msix\, rustshot-windows-x86_64\), as `gh run download` makes them.
    [string]$Artifacts,
    # Or: a CI run id; its Windows artifacts are downloaded with gh.
    [string]$RunId,
    [string]$Repo = 'nappsllc/rustshot',
    # An older *-setup.exe for the upgrade checks (e.g. the last release's).
    [string]$Old,
    # An older release's *-x64.msix (same identity) for the MSIX update check;
    # signed with the same throwaway certificate as the new one.
    [string]$OldMsix,
    # Work folder (mapped into the sandbox as Desktop\test). Results: <Work>\out\results.txt.
    [string]$Work = (Join-Path $env:TEMP 'rustshot-sandbox'),
    # Just one group: probe (hotkey probe), rec (recording), v030 / v040 / v041 / v050 / v060 / v062 / v063 (0.3.0 / 0.4.0 / 0.4.1 / 0.5.0 / 0.6.0 / 0.6.2 / 0.6.3 checks), data (data files), msix (the MSIX).
    [ValidateSet('', 'probe', 'rec', 'v030', 'v040', 'v041', 'v050', 'v060', 'v062', 'v063', 'data', 'msix')] [string]$Only = '',
    # A picture with a QR code of an https link for the 0.5.0 Read QR check
    # (e.g. the qr_make_link_light_1x.png preview; without it that check SKIPs).
    [string]$QrPng,
    # Skip signing the MSIX (the MSIX checks then FAIL with "inputs present").
    [switch]$NoSign,
    # Prepare <Work> and the .wsb only.
    [switch]$NoLaunch
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
$in = Join-Path $Work 'in'; $oldDir = Join-Path $Work 'old'
New-Item -ItemType Directory -Force $in, $oldDir | Out-Null
Copy-Item (Join-Path $here '*.ps1') $Work -Exclude 'run.ps1', 'sign-sideload.ps1'

if ($RunId) {
    gh run download $RunId -R $Repo -D $in -n rustshot-windows-installer -n rustshot-windows-msix -n rustshot-windows-x86_64
    if ($LASTEXITCODE -ne 0) { throw 'gh run download failed' }
} elseif ($Artifacts) {
    Copy-Item (Join-Path $Artifacts '*') $in -Recurse
} else { throw 'pass -Artifacts or -RunId' }
if ($Old) { Copy-Item $Old $oldDir }
$signDirs = @($in)
if ($OldMsix) {
    $oldMsixDir = Join-Path $Work 'oldmsix'
    New-Item -ItemType Directory -Force $oldMsixDir | Out-Null
    Copy-Item $OldMsix $oldMsixDir
    $signDirs += $oldMsixDir
}
# The blur model, for the data checks' "put there by hand" case (packages carry none).
$dataDir = Join-Path $Work 'data'
New-Item -ItemType Directory -Force $dataDir | Out-Null
Copy-Item (Join-Path $here '..\..\models\selfie.bin') $dataDir
# The language files, for the 0.5.0 checks' "put there by hand" cases.
Copy-Item (Join-Path $here '..\..\lang') $dataDir -Recurse
if ($QrPng) { Copy-Item $QrPng (Join-Path $dataDir 'qr.png') }
# The 0.6.3 icon checks: the new pixel-aligned 16 px frame and 0.6.2's (from
# git; that check SKIPs without it).
Copy-Item (Join-Path $here '..\..\packaging\icons\small\rustshot-16.png') (Join-Path $dataDir 'icon16-new.png')
$old16 = Join-Path $dataDir 'icon16-062.png'
cmd /c "git -C `"$here`" show v0.6.2:packaging/icons/rustshot-16.png > `"$old16`" 2> nul"
if ($LASTEXITCODE -ne 0 -or (Get-Item $old16).Length -lt 100) { Remove-Item $old16 -ErrorAction SilentlyContinue }
if (-not $NoSign) { & (Join-Path $here 'sign-sideload.ps1') -In $signDirs }
if ($Only) { New-Item -ItemType File (Join-Path $Work "$Only-only") | Out-Null }

$wsb = Join-Path $Work 'rustshot-test.wsb'
Set-Content $wsb @"
<Configuration>
  <Networking>Disable</Networking>
  <vGPU>Disable</vGPU>
  <ClipboardRedirection>Disable</ClipboardRedirection>
  <MappedFolders>
    <MappedFolder>
      <HostFolder>$Work</HostFolder>
      <SandboxFolder>C:\Users\WDAGUtilityAccount\Desktop\test</SandboxFolder>
      <ReadOnly>false</ReadOnly>
    </MappedFolder>
  </MappedFolders>
  <LogonCommand>
    <Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\Users\WDAGUtilityAccount\Desktop\test\test.ps1</Command>
  </LogonCommand>
</Configuration>
"@
Get-ChildItem (@($signDirs) + $oldDir) -Recurse -File | ForEach-Object { "{0,10}  {1}" -f $_.Length, $_.FullName.Substring($Work.Length + 1) }
if (-not $NoLaunch) {
    Start-Process $wsb
    "sandbox started; results appear in $(Join-Path $Work 'out\results.txt') (ends with DONE)"
}
