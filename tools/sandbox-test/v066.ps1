# 0.6.6 checks, dot-sourced by test.ps1 and called from msix.ps1 on the
# installed sideload-signed MSIX (the Store package fix: no hidden app):
# - Before066 (with -OldMsix, on the older release's package before the
#   update): its applications and startup tasks in the log, then its
#   "RustshotDaemon" startup task turned off (StartupTask.Disable() in the
#   package, v066-task.ps1 through Invoke-CommandInDesktopPackage).
# - Checks066: the installed manifest has exactly the Rustshot and
#   RustshotSettings applications, none hidden from Start
#   (AppListEntry="none": Partner Center refuses such a "headless" app),
#   and Start lists exactly "Rustshot" and "Rustshot Settings" for the
#   package; the startup task "Rustshot hotkeys" (TaskId RustshotDaemon,
#   Enabled="true") is on the Rustshot application, which has no
#   parameters; the StartupTask API in the package finds the task: after
#   the update still off as Before066 left it (the user's choice carries
#   over), then on again, or on after a fresh install; a sign-in-like
#   start (no Rustshot running, the Rustshot application activated with
#   no arguments, as the startup task starts it) runs the daemon (tray
#   window, no capture, no arguments on its command line); with it up,
#   Start's Rustshot Settings opens Settings and Rustshot captures.
# Inside the sandbox only (KillAll as in the other MSIX checks).

function Apps066($man) {
    @($man.Package.Applications.Application)
}

function Task066($pfn, $app, $mode, $tag) {
    $f = "$out\v066-task-$tag.txt"
    Remove-Item $f -ErrorAction SilentlyContinue
    $ps = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
    try {
        Invoke-CommandInDesktopPackage -PackageFamilyName $pfn -AppId $app -Command $ps `
            -Args "-NoProfile -ExecutionPolicy Bypass -File `"$T\v066-task.ps1`" -Mode $mode -Out `"$f`"" -ErrorAction Stop
    } catch {
        return "error Invoke-CommandInDesktopPackage: $($_.Exception.Message)"
    }
    if (WaitFor { (Test-Path $f) -and ((Get-Content $f -Raw -ErrorAction SilentlyContinue) -match 'END') } 40) {
        (Get-Content $f | Where-Object { $_ -ne 'END' }) -join ' | '
    } else { 'error no output' }
}

# Where Windows keeps a package's startup task state (for the log).
function StartupReg066($pfn) {
    $k = "HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData\$pfn"
    if (-not (Test-Path $k)) { return 'no SystemAppData key' }
    $rows = foreach ($sub in @(Get-Item $k) + @(Get-ChildItem $k -Recurse)) {
        foreach ($p in $sub.Property) { "$($sub.PSChildName)\$p=$($sub.GetValue($p))" }
    }
    $rows -join '; '
}

$script:before066 = $null
function Before066($oldPkg) {
    $man = Get-AppxPackageManifest $oldPkg
    $ids = (Apps066 $man | ForEach-Object Id) -join ','
    Add-Content $log "      v066 old package $($oldPkg.Version): applications $ids"
    Add-Content $log "      v066 old startup tasks from Rustshot: $(Task066 $oldPkg.PackageFamilyName 'Rustshot' 'list' 'old-main')"
    # The API answers in the application that declares the task (0.6.5's
    # hidden RustshotDaemon).
    $app = (Apps066 $man | Where-Object { $_.Extensions.Extension | Where-Object Category -eq 'windows.startupTask' } | Select-Object -First 1).Id
    $t0 = Task066 $oldPkg.PackageFamilyName $app 'list' 'old'
    Add-Content $log "      v066 old startup tasks: $t0"
    $t1 = Task066 $oldPkg.PackageFamilyName $app 'disable' 'old-disable'
    Add-Content $log "      v066 old startup tasks after Disable(): $t1"
    Add-Content $log "      v066 old startup state (registry): $(StartupReg066 $oldPkg.PackageFamilyName)"
    $script:before066 = $t1
}

function Checks066($pkg) {
    $man = Get-AppxPackageManifest $pkg
    $pfn = $pkg.PackageFamilyName
    $apps = Apps066 $man
    $ids = @($apps | ForEach-Object Id)
    Check 'v066 msix: exactly the Rustshot and RustshotSettings applications' (($ids -join ',') -eq 'Rustshot,RustshotSettings') ($ids -join ',')
    $hidden = @($apps | Where-Object { $_.VisualElements.AppListEntry -eq 'none' } | ForEach-Object Id)
    Check 'v066 msix: no application hidden from Start (AppListEntry="none")' ($hidden.Count -eq 0) ($hidden -join ',')
    $start = @(Get-StartApps | Where-Object { $_.AppID -like "$pfn!*" } | Sort-Object AppID)
    $names = @($start | ForEach-Object { "$($_.Name)=$($_.AppID.Split('!')[1])" })
    Check 'v066 msix: Start lists exactly "Rustshot" and "Rustshot Settings"' (($names -join ',') -eq 'Rustshot=Rustshot,Rustshot Settings=RustshotSettings') ($names -join ',')
    $main = $apps | Where-Object Id -eq 'Rustshot'
    $st = @($main.Extensions.Extension | Where-Object Category -eq 'windows.startupTask')
    $all = @($apps | ForEach-Object { $_.Extensions.Extension } | Where-Object Category -eq 'windows.startupTask')
    $task = if ($st.Count -eq 1) { $st[0].StartupTask } else { $null }
    $ok = $all.Count -eq 1 -and $task -and $task.TaskId -eq 'RustshotDaemon' -and $task.Enabled -eq 'true' -and
        $task.DisplayName -eq 'Rustshot hotkeys' -and $st[0].Executable -eq 'rustshot.exe'
    Check 'v066 msix: the startup task "Rustshot hotkeys" (RustshotDaemon, enabled) is on the Rustshot application' $ok "on Rustshot=$($st.Count) in all=$($all.Count) id=$($task.TaskId) enabled=$($task.Enabled) name=$($task.DisplayName) exe=$($st[0].Executable)"
    Check 'v066 msix: the Rustshot application has no parameters (a plain launch)' (-not $main.Parameters) "$($main.Parameters)"

    # The task as Windows sees it (the StartupTask API in the package).
    $now = Task066 $pfn 'Rustshot' 'list' 'new'
    Add-Content $log "      v066 startup tasks: $now"
    Add-Content $log "      v066 startup state (registry): $(StartupReg066 $pfn)"
    if ($script:before066 -and $script:before066 -match 'task RustshotDaemon Disabled') {
        Check 'v066 msix upgrade: the startup task is still off as the user left it' ($now -match 'task RustshotDaemon Disabled' -and $now -notmatch 'error') $now
        $on = Task066 $pfn 'Rustshot' 'enable' 'new-enable'
        Check 'v066 msix upgrade: and turns on again (RequestEnableAsync)' ($on -match 'task RustshotDaemon Enabled') $on
    } else {
        if ($script:before066) { Add-Content $log "SKIP  v066 msix upgrade: the older package's task wasn't turned off ($($script:before066))" }
        Check 'v066 msix: the StartupTask API finds RustshotDaemon, enabled' ($now -match 'task RustshotDaemon Enabled') $now
    }

    # A sign-in-like start: nothing of Rustshot runs, then the Rustshot
    # application is activated with no arguments (what the startup task does).
    KillAll
    Start-Process explorer.exe "shell:AppsFolder\$pfn!Rustshot"
    $up = WaitFor { [W]::TrayWindow() } 10
    Start-Sleep 1
    $procs = @(Get-CimInstance Win32_Process -Filter "Name='rustshot.exe'")
    $cmd = ($procs | Select-Object -First 1).CommandLine
    $path = ($procs | Select-Object -First 1).ExecutablePath
    $plainCmd = "$cmd".Trim() -match '^("[^"]*rustshot\.exe"|\S*rustshot\.exe)$'
    Check 'v066 msix: a sign-in-like start (no arguments) runs the daemon' ($up -and $procs.Count -eq 1 -and -not [W]::OverlayVisible()) "tray=$up processes=$($procs.Count)"
    Check 'v066 msix: the daemon is the package''s exe with no arguments' ($plainCmd -and "$path" -like "$($pkg.InstallLocation)*") "cmd=$cmd path=$path"
    # With the daemon up: Start's Rustshot Settings opens Settings.
    Start-Process explorer.exe "shell:AppsFolder\$pfn!RustshotSettings"
    $sw = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 10
    $h = [W]::Settings()
    $owner = if ($sw) { [W]::Owner($h) } else { 0 }
    Check 'v066 msix: with the daemon up, Rustshot Settings opens Settings' $sw "window owner pid=$owner daemon=$(($procs | Select-Object -First 1).ProcessId)"
    if ($sw) { Shot 'v066-settings'; [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero); $null = WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 5 }
    # And Start's Rustshot captures, as before.
    Start-Process explorer.exe "shell:AppsFolder\$pfn!Rustshot"
    $ov = WaitFor { [W]::OverlayVisible() } 8
    Check 'v066 msix: with the daemon up, Rustshot from Start captures' $ov
    if ($ov) { Shot 'v066-overlay' }
    KillAll
}
