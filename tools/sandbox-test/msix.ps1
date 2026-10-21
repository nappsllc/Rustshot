# MSIX checks, dot-sourced by test.ps1: the sideload-signed copy of the CI's
# package installs (over the sideload-signed older release's package when
# run.ps1 -OldMsix gave one: an in-place update to the new version), its
# manifest declares one language per lang\<tag>.txt with en-us first, the
# Start entry starts the daemon, the alias runs, a second launch captures,
# the startup task is declared, the 0.6.6 checks (v066.ps1: Before066 on the
# older package, Checks066 on the new one), the 0.6.7 checks (v067.ps1),
# and it uninstalls.
# Inputs: <T>\in\**\sideload.cer and *-sideload.msix (run.ps1 /
# sign-sideload.ps1), optionally <T>\oldmsix\*-sideload.msix, and
# <T>\data\lang\*.txt (the repo's language files).

function MsixChecks {
    $cer = Get-ChildItem "$T\in" -Recurse -Filter 'sideload.cer' | Select-Object -First 1
    $msix = Get-ChildItem "$T\in" -Recurse -Filter '*-sideload.msix' | Select-Object -First 1
    if (-not ($cer -and $msix)) {
        Check 'msix: inputs present' $false 'sideload.cer / *-sideload.msix missing'
        return
    }
    Import-Certificate -FilePath $cer.FullName -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    $old = Get-ChildItem "$T\oldmsix" -Filter '*-sideload.msix' -ErrorAction SilentlyContinue | Select-Object -First 1
    $oldPkg = $null
    if ($old) {
        $err = $null
        try { Add-AppxPackage -Path $old.FullName -ErrorAction Stop } catch { $err = $_.Exception.Message }
        $oldPkg = Get-AppxPackage *rustshot*
        Check 'msix upgrade: the older release''s package installs' ([bool]$oldPkg -and -not $err) "$($old.Name) $($oldPkg.Version) $err"
        # 0.6.6: its startup task turned off before the update (v066.ps1).
        if ($oldPkg) { Before066 $oldPkg }
    } else {
        Add-Content $log 'SKIP  msix upgrade: no older package (run.ps1 -OldMsix)'
    }
    $err = $null
    try { Add-AppxPackage -Path $msix.FullName -ErrorAction Stop } catch { $err = $_.Exception.Message }
    $pkg = Get-AppxPackage *rustshot*
    Check 'msix: signed package installs' ([bool]$pkg -and -not $err) "$($msix.Name) $err"
    if (-not $pkg) { return }
    if ($oldPkg) {
        Check 'msix upgrade: one package, updated in place to the new version' (@($pkg).Count -eq 1 -and [version]$pkg.Version -gt [version]$oldPkg.Version -and $pkg.PackageFamilyName -eq $oldPkg.PackageFamilyName) "$($oldPkg.Version) -> $($pkg.Version)"
    }
    $man = Get-AppxPackageManifest $pkg
    # The languages as the installed package declares them.
    $declared = @($man.Package.Resources.Resource | ForEach-Object { $_.Language })
    Add-Content $log "      msix languages ($($declared.Count)): $($declared -join ' ')"
    $files = @(Get-ChildItem "$T\data\lang" -Filter *.txt -ErrorAction SilentlyContinue | ForEach-Object BaseName)
    # msix.ps1's Store codes: the file's tag, except en -> en-us (case aside).
    $want = @($files | ForEach-Object { if ($_ -eq 'en') { 'en-us' } else { $_.ToLowerInvariant() } } | Sort-Object)
    $got = @($declared | ForEach-Object { $_.ToLowerInvariant() } | Sort-Object)
    Check 'msix: manifest declares a language per lang\<tag>.txt' ($files.Count -ge 35 -and (@(Compare-Object $want $got).Count -eq 0)) "files=$($files.Count) declared=$($declared.Count) diff=$((Compare-Object $want $got | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }) -join ',')"
    Check 'msix: en-us is the first (default) language' ($declared.Count -gt 0 -and $declared[0] -eq 'en-us') "$($declared | Select-Object -First 1)"
    $loc = $pkg.InstallLocation
    $lic = @('LICENSE.txt', 'NOTICE.txt', 'THIRD_PARTY_NOTICES.md') | Where-Object { -not (Test-Path (Join-Path $loc $_)) }
    $apache = (Get-Content (Join-Path $loc 'LICENSE.txt') -Raw -ErrorAction SilentlyContinue) -match 'Apache License'
    Check 'msix: LICENSE.txt (Apache-2.0) + NOTICE.txt + THIRD_PARTY_NOTICES.md installed' (@($lic).Count -eq 0 -and $apache) "missing=$($lic -join ',') apache=$apache"
    $pfn = $pkg.PackageFamilyName
    Start-Process explorer.exe "shell:AppsFolder\$pfn!Rustshot"
    $trayUp = WaitFor { [W]::TrayWindow() } 10
    Check 'msix: Start entry launches daemon (tray window)' $trayUp
    Start-Sleep 1
    Check 'msix: Start entry does NOT capture' (-not [W]::OverlayVisible())
    $cmd = (Get-CimInstance Win32_Process -Filter "Name='rustshot.exe'" | Select-Object -First 1).CommandLine
    Add-Content $log "      msix daemon command line: $cmd"
    $alias = "$env:LOCALAPPDATA\Microsoft\WindowsApps\rustshot.exe"
    Check 'msix: rustshot.exe execution alias exists' (Test-Path $alias)
    $v = & $alias --version 2>&1
    Check 'msix: alias runs (--version)' ("$v" -match '\d+\.\d+\.\d+') "$v"
    Start-Process explorer.exe "shell:AppsFolder\$pfn!Rustshot"
    $ov = WaitFor { [W]::OverlayVisible() } 6
    Check 'msix: launching again captures' $ov
    Shot 'msix-overlay'
    $st = $man.Package.Applications.Application | ForEach-Object { $_.Extensions.Extension } | Where-Object { $_.Category -eq 'windows.startupTask' }
    Check 'msix: startup task declared' ([bool]$st)
    # 0.6.6: two Start entries, no hidden app, the startup task on Rustshot
    # (kept off over the update), a sign-in-like start (v066.ps1).
    Checks066 $pkg
    # 0.6.7: ms-screenclip (Win+Shift+S, Print Screen) chosen in Default
    # apps, the tray icon off with nothing in the background, Start at
    # login through the startup task (v067.ps1).
    Checks067 $pkg
    KillAll
    Remove-AppxPackage $pkg.PackageFullName
    Check 'msix: removed' (-not (Get-AppxPackage *rustshot*))
}
