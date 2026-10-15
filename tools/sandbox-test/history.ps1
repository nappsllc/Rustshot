# History checks (0.2.0), dot-sourced by test.ps1.
# An isolated instance of the portable exe (own RUSTSHOT_INSTANCE, APPDATA
# and LOCALAPPDATA, unusual hotkeys): a direct capture and a daemon capture
# each leave an entry folder under <LOCALAPPDATA>\rustshot-data\history with
# original.png, thumb.png and meta.json; `rustshot history` and the History
# hotkey open the History window; with history = false nothing new is kept.

function HistEntries($root) {
    @(Get-ChildItem "$root\rustshot-data\history" -Directory -ErrorAction SilentlyContinue |
        ForEach-Object { Get-ChildItem $_.FullName -Directory } |
        Where-Object { Test-Path "$($_.FullName)\meta.json" })
}

function WithEnv($vars, [scriptblock]$body) {
    $old = @{}
    foreach ($k in $vars.Keys) { $old[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $vars[$k]) }
    try { & $body } finally { foreach ($k in $old.Keys) { [Environment]::SetEnvironmentVariable($k, $old[$k]) } }
}

function HistConfig($appdata, $save, $on) {
    $esc = $save.Replace('\', '\\')
    Set-Content "$appdata\rustshot\config.toml" @"
save_path = "$esc"
capture_hotkey = "Ctrl+Alt+Shift+F11"
quit_hotkey = "Ctrl+Alt+Shift+F10"
history_hotkey = "Ctrl+Alt+Shift+F8"
check_updates = false
show_thumbnail = false
history = $(if ($on) { 'true' } else { 'false' })
"@
}

function HistoryChecks($exe) {
    $root = "$env:TEMP\hist-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    HistConfig $appdata $save $true
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-hist' }

    # A direct capture is kept.
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full' -PassThru -Wait -RedirectStandardError "$out\hist-full-stderr.txt" }
    $e = HistEntries $local
    Check 'history: `rustshot full` exits 0' ($p.ExitCode -eq 0) "exit=$($p.ExitCode)"
    Check 'history: one entry under the isolated LOCALAPPDATA\rustshot\history' ($e.Count -eq 1) "$($e.FullName -join ', ')"
    if ($e.Count -ge 1) {
        $d = $e[0].FullName
        $names = (Get-ChildItem $d -File).Name -join ','
        Check 'history: entry has original.png, thumb.png, meta.json' ((Test-Path "$d\original.png") -and (Test-Path "$d\thumb.png") -and (Test-Path "$d\meta.json")) "$($d.Substring($local.Length)) : $names"
        $m = $null
        try { $m = Get-Content "$d\meta.json" -Raw | ConvertFrom-Json } catch {}
        Check 'history: meta.json is a screenshot with its size and saved path' ($m -and $m.kind -eq 'screenshot' -and $m.w -gt 0 -and $m.h -gt 0 -and $m.saved -and (Test-Path $m.saved)) "kind=$($m.kind) $($m.w)x$($m.h) saved=$($m.saved)"
        $png = [System.IO.File]::ReadAllBytes("$d\thumb.png")[1..3]
        Check 'history: thumb.png is a PNG' ([System.Text.Encoding]::ASCII.GetString($png) -eq 'PNG')
        Copy-Item "$d\meta.json" "$out\hist-meta.json"
    }

    # `rustshot history` opens the History window (no daemon: in its own process).
    $h = WithEnv $vars { Start-Process $exe -ArgumentList 'history' -PassThru }
    $up = WaitFor { [W]::FindWindow('rustshot_window', "Rustshot $([char]0x2014) History") -ne [IntPtr]::Zero } 10
    $hw = [W]::FindWindow('rustshot_window', "Rustshot $([char]0x2014) History")
    Check 'history: `rustshot history` opens the History window' ($up -and [W]::Owner($hw) -eq $h.Id) "owner=$(if ($up) { [W]::Owner($hw) }) pid=$($h.Id)"
    Start-Sleep 2
    Shot 'history-window'
    if ($up) { [void][W]::PostMessage($hw, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    Check 'history: closing the History window ends that process (exit 0)' ((WaitFor { $h.HasExited } 5) -and $h.ExitCode -eq 0) "exit=$(if ($h.HasExited) { $h.ExitCode })"
    if (-not $h.HasExited) { Stop-Process -Id $h.Id -Force -ErrorAction SilentlyContinue }

    # A daemon capture (Ctrl+S in the overlay) is kept; the History hotkey opens the window in the daemon.
    $dm = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError "$out\hist-daemon-stderr.txt" }
    $dUp = WaitFor { [W]::FindWindow('rustshot_tray_sbx-hist', [NullString]::Value) -ne [IntPtr]::Zero } 10
    Check 'history: isolated daemon up' ($dUp -and -not $dm.HasExited) "pid=$($dm.Id)"
    Start-Sleep 1
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', '400x300+100+100' -PassThru }
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 8
    Start-Sleep 1
    if ($ovUp) { [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300; [W]::Chord(0x11, 0x53) }   # Ctrl+S
    $saved = WaitFor { (HistEntries $local).Count -eq 2 } 10
    $e2 = HistEntries $local
    Check 'history: Ctrl+S in the daemon overlay keeps a second entry' $saved "entries=$($e2.Count) overlay=$ovUp"
    if ($e2.Count -ge 2) {
        $m2 = Get-Content "$(($e2 | Sort-Object Name | Select-Object -Last 1).FullName)\meta.json" -Raw | ConvertFrom-Json
        Check 'history: the overlay entry is the 400x300 selection' ($m2.w -eq 400 -and $m2.h -eq 300) "$($m2.w)x$($m2.h)"
    }
    $null = WaitFor { $g.HasExited } 5
    if (-not $g.HasExited) { Stop-Process -Id $g.Id -Force -ErrorAction SilentlyContinue }
    [R]::Hyper(0x77)   # Ctrl+Alt+Shift+F8: history_hotkey
    $up = WaitFor { [W]::FindWindow('rustshot_window', "Rustshot $([char]0x2014) History") -ne [IntPtr]::Zero } 8
    $hw = [W]::FindWindow('rustshot_window', "Rustshot $([char]0x2014) History")
    Check 'history: the History hotkey opens the window in the daemon' ($up -and [W]::Owner($hw) -eq $dm.Id) "owner=$(if ($up) { [W]::Owner($hw) }) daemon=$($dm.Id)"
    Start-Sleep 2
    Shot 'history-window-daemon'
    if ($up) { [void][W]::PostMessage($hw, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    Check 'history: closing it keeps the daemon running' ((WaitFor { [W]::FindWindow('rustshot_window', "Rustshot $([char]0x2014) History") -eq [IntPtr]::Zero } 5) -and -not $dm.HasExited)
    Stop-Process -Id $dm.Id -Force -ErrorAction SilentlyContinue   # the PID started above
    $null = WaitFor { $dm.HasExited } 5

    # history = false: a capture is saved, nothing new is kept.
    HistConfig $appdata $save $false
    $before = (HistEntries $local).Count
    $files = @(Get-ChildItem $save -Recurse -File).Count
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full' -PassThru -Wait }
    Start-Sleep 1
    $after = (HistEntries $local).Count
    $files2 = @(Get-ChildItem $save -Recurse -File).Count
    Check 'history off: the capture is saved' ($p.ExitCode -eq 0 -and $files2 -eq $files + 1) "exit=$($p.ExitCode) files $files -> $files2"
    Check 'history off: no new entry' ($after -eq $before) "entries $before -> $after"
    # `rustshot history --list`: one tab-separated line per entry, newest first.
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'history', '--list' -PassThru -Wait -RedirectStandardOutput "$out\hist-list.txt" }
    $lines = @(Get-Content "$out\hist-list.txt" -ErrorAction SilentlyContinue | Where-Object { $_ })
    $ok = $lines.Count -eq $after -and @($lines | Where-Object { $f = $_ -split "`t"; $f.Count -eq 5 -and $f[0] -match '^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d$' -and $f[1] -eq 'screenshot' -and $f[2] -match '^\d+x\d+$' -and (Test-Path $f[4]) }).Count -eq $after
    Check 'history: `rustshot history --list` prints every entry (date, kind, size, file)' ($p.ExitCode -eq 0 -and $ok) "exit=$($p.ExitCode) lines=$($lines.Count) entries=$after first=[$($lines | Select-Object -First 1)]"
    $err = Get-Content "$out\hist-daemon-stderr.txt", "$out\hist-full-stderr.txt" -ErrorAction SilentlyContinue
    if ($err) { Add-Content $log ("      stderr: " + (($err | Select-Object -First 6) -join ' | ')) }
}
