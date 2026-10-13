<#
.SYNOPSIS
  Sign a copy of the CI's (unsigned) MSIX for the sandbox test.

  Makes a throwaway self-signed code-signing certificate (its subject is
  the manifest's Publisher, e.g. the Store's CN=<GUID> or CN=nappsllc-dev; 2 days, a random PFX password that is never written down), signs
  <In>\*-x64.msix as <In>\rustshot-<ver>-x64-sideload.msix, exports the
  public certificate as <In>\sideload.cer (test.ps1 trusts it inside the
  sandbox only), then deletes the certificate and the PFX again.
  Needs signtool.exe from the Windows SDK.
#>
param(
    [Parameter(Mandatory)] [string]$In,
    [string]$SignTool
)
$ErrorActionPreference = 'Stop'
if (-not $SignTool) {
    $SignTool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
}
if (-not $SignTool -or -not (Test-Path $SignTool)) { throw 'signtool.exe not found (Windows SDK); pass -SignTool' }
$msix = Get-ChildItem $In -Recurse -Filter '*-x64.msix' | Where-Object Name -notmatch 'sideload' | Select-Object -First 1
if (-not $msix) { throw "no *-x64.msix under $In" }
$signed = Join-Path $In ($msix.BaseName + '-sideload.msix')
# The certificate's subject must be the manifest's Publisher.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($msix.FullName)
try {
    $rd = New-Object IO.StreamReader(($zip.Entries | Where-Object FullName -eq 'AppxManifest.xml').Open())
    $publisher = ([xml]$rd.ReadToEnd()).Package.Identity.Publisher
    $rd.Close()
} finally { $zip.Dispose() }
if (-not $publisher) { throw "no Publisher in $($msix.Name)'s AppxManifest.xml" }
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("rustshot-sign-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
$cert = $null
try {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $publisher -KeyUsage DigitalSignature -FriendlyName 'rustshot sandbox sideload' -CertStoreLocation Cert:\CurrentUser\My -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}') -NotAfter (Get-Date).AddDays(2)
    $plain = [guid]::NewGuid().ToString()
    $pw = ConvertTo-SecureString -String $plain -Force -AsPlainText
    Export-PfxCertificate -Cert $cert -FilePath "$tmp\s.pfx" -Password $pw | Out-Null
    Export-Certificate -Cert $cert -FilePath (Join-Path $In 'sideload.cer') | Out-Null
    Copy-Item $msix.FullName $signed -Force
    & $SignTool sign /fd SHA256 /f "$tmp\s.pfx" /p $plain $signed 2>&1 | Select-Object -Last 2
    if ($LASTEXITCODE -ne 0) { throw "signtool exit=$LASTEXITCODE" }
} finally {
    if ($cert) { Get-ChildItem Cert:\CurrentUser\My | Where-Object Thumbprint -eq $cert.Thumbprint | Remove-Item }
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
"signed: $signed"
"certs left: " + @(Get-ChildItem Cert:\CurrentUser\My | Where-Object FriendlyName -eq 'rustshot sandbox sideload').Count
