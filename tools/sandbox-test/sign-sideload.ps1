<#
.SYNOPSIS
  Sign a copy of the CI's (unsigned) MSIX for the sandbox test.

  Makes a throwaway self-signed code-signing certificate (its subject is
  the manifest's Publisher, e.g. the Store's CN=<GUID> or CN=nappsllc-dev; 2 days, a random PFX password that is never written down), signs
  <In>\*-x64.msix as <In>\rustshot-<ver>-x64-sideload.msix, exports the
  public certificate as <In>\sideload.cer (test.ps1 trusts it inside the
  sandbox only), then deletes the certificate and the PFX again. With
  more than one -In folder, the package in each is signed with the same
  certificate (an older release's, for the update check; same Publisher)
  and sideload.cer goes into the first.
  Needs signtool.exe from the Windows SDK.
#>
param(
    [Parameter(Mandatory)] [string[]]$In,
    [string]$SignTool
)
$ErrorActionPreference = 'Stop'
if (-not $SignTool) {
    $SignTool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
}
if (-not $SignTool -or -not (Test-Path $SignTool)) { throw 'signtool.exe not found (Windows SDK); pass -SignTool' }
$pkgs = @(foreach ($dir in $In) {
    $m = Get-ChildItem $dir -Recurse -Filter '*-x64.msix' | Where-Object Name -notmatch 'sideload' | Select-Object -First 1
    if (-not $m) { throw "no *-x64.msix under $dir" }
    # The signed copy goes into the -In folder itself, as before.
    [pscustomobject]@{ File = $m; Signed = Join-Path $dir ($m.BaseName + '-sideload.msix') }
})
# The certificate's subject must be the manifest's Publisher.
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Publisher($file) {
    $zip = [IO.Compression.ZipFile]::OpenRead($file)
    try {
        $rd = New-Object IO.StreamReader(($zip.Entries | Where-Object FullName -eq 'AppxManifest.xml').Open())
        $p = ([xml]$rd.ReadToEnd()).Package.Identity.Publisher
        $rd.Close()
        $p
    } finally { $zip.Dispose() }
}
$publisher = Publisher $pkgs[0].File.FullName
if (-not $publisher) { throw "no Publisher in $($pkgs[0].File.Name)'s AppxManifest.xml" }
foreach ($m in $pkgs) {
    $p = Publisher $m.File.FullName
    if ($p -cne $publisher) { throw "$($m.File.Name)'s Publisher '$p' is not '$publisher'" }
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("rustshot-sign-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
$cert = $null
try {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $publisher -KeyUsage DigitalSignature -FriendlyName 'rustshot sandbox sideload' -CertStoreLocation Cert:\CurrentUser\My -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}') -NotAfter (Get-Date).AddDays(2)
    $plain = [guid]::NewGuid().ToString()
    $pw = ConvertTo-SecureString -String $plain -Force -AsPlainText
    Export-PfxCertificate -Cert $cert -FilePath "$tmp\s.pfx" -Password $pw | Out-Null
    Export-Certificate -Cert $cert -FilePath (Join-Path $In[0] 'sideload.cer') | Out-Null
    foreach ($m in $pkgs) {
        Copy-Item $m.File.FullName $m.Signed -Force
        & $SignTool sign /fd SHA256 /f "$tmp\s.pfx" /p $plain $m.Signed 2>&1 | Select-Object -Last 2
        if ($LASTEXITCODE -ne 0) { throw "signtool exit=$LASTEXITCODE" }
        "signed: $($m.Signed)"
    }
} finally {
    if ($cert) { Get-ChildItem Cert:\CurrentUser\My | Where-Object Thumbprint -eq $cert.Thumbprint | Remove-Item }
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
"certs left: " + @(Get-ChildItem Cert:\CurrentUser\My | Where-Object FriendlyName -eq 'rustshot sandbox sideload').Count
