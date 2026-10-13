# 0.4.0 checks, dot-sourced by test.ps1 and run on the portable exe:
# the tray icon off (no background process: `rustshot daemon` exits, a
# plain launch opens Settings, the one-time notice for a 0.3.0 config with
# the icon off), `rustshot screenclip <URI>` with no daemon (an on-demand
# capture that holds no global hotkeys and ends after Esc, posted to its own
# overlay), `register-screenclip on|off` (the VM only: the keys Windows
# offers in Default apps, gone again after off), the export border, image
# edits (crop, flip, rotate, canvas) from an annotations.json fixture
# rendered by History's Open with, a freeform (lasso) snip saved with a
# transparent outside, Text mode (select all, copy) and Quick Redact (solid)
# on a window with an e-mail address.
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped. Real key and
# mouse input goes to the sandbox's own desktop.

function V4Config($appdata, $save, $extra = '') {
    $esc = $save.Replace('\', '\\')
    Set-Content "$appdata\rustshot\config.toml" (@"
save_path = "$esc"
capture_hotkey = "Ctrl+Alt+Shift+F11"
quit_hotkey = "Ctrl+Alt+Shift+F10"
check_updates = false
show_thumbnail = false
screenclip_hint = false
"@ + "`n" + $extra)
}

function StopPid($p) {
    if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $null = WaitFor { $p.HasExited } 5 }
}

# ARGB of pixel (x, y) of an image file (negative x/y count from the right/bottom).
function Pixel($path, $x, $y) {
    $b = New-Object System.Drawing.Bitmap $path
    try {
        if ($x -lt 0) { $x = $b.Width + $x }
        if ($y -lt 0) { $y = $b.Height + $y }
        $c = $b.GetPixel($x, $y)
        '{0:x2}{1:x2}{2:x2}{3:x2}' -f $c.A, $c.R, $c.G, $c.B
    } finally { $b.Dispose() }
}

# Pixels that differ between two same-sized pictures (every 2nd px each
# way), and the most common colour among them in the second one.
function DiffStats($a, $b) {
    $p = New-Object System.Drawing.Bitmap $a; $q = New-Object System.Drawing.Bitmap $b
    try {
        if ($p.Width -ne $q.Width -or $p.Height -ne $q.Height) { return @{ n = -1; top = ''; share = 0 } }
        $n = 0; $hist = @{}
        for ($y = 0; $y -lt $p.Height; $y += 2) {
            for ($x = 0; $x -lt $p.Width; $x += 2) {
                $c1 = $p.GetPixel($x, $y); $c2 = $q.GetPixel($x, $y)
                if ([Math]::Abs($c1.R - $c2.R) + [Math]::Abs($c1.G - $c2.G) + [Math]::Abs($c1.B - $c2.B) -gt 24) {
                    $n++; $k = '{0:x2}{1:x2}{2:x2}' -f $c2.R, $c2.G, $c2.B; $hist[$k] = 1 + [int]$hist[$k]
                }
            }
        }
        $top = $hist.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1
        @{ n = $n; top = $(if ($top) { $top.Key } else { '' }); share = $(if ($n -gt 0 -and $top) { [Math]::Round($top.Value / $n, 2) } else { 0 }) }
    } finally { $p.Dispose(); $q.Dispose() }
}

function Newest($dir, $filter = '*.png') {
    Get-ChildItem $dir -Recurse -Filter $filter -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
}

function Checks040($exe) {
    $root = "$env:TEMP\v040-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $cfgPath = "$appdata\rustshot\config.toml"
    $inst = 'sbx-040'
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = $inst }
    $tray = { [W]::FindWindow("rustshot_tray_$inst", [NullString]::Value) -ne [IntPtr]::Zero }

    # ------------------------------------------------ tray icon off: a 0.3.0 config (show_tray = false, no marker)
    V4Config $appdata $save "show_tray = false`n"
    $d = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError "$out\v040-notice-stderr.txt" }; $null = $d.Handle
    $up = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 10
    $h = [W]::Settings()
    Start-Sleep 2
    Shot 'v040-tray-off-notice'
    $cfg = Get-Content $cfgPath -Raw
    Check 'v040 tray off: a 0.3.0 config with the icon off gets the one-time notice (Settings opens in that process, no tray)' ($up -and [W]::Owner($h) -eq $d.Id -and -not (& $tray)) "window=$up owner=$(if ($up) { [W]::Owner($h) }) pid=$($d.Id)"
    Check 'v040 tray off: the notice writes tray_off_mode_seen = true' ($cfg -match '(?m)^tray_off_mode_seen = true')
    if ($up) { [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    $gone = WaitFor { $d.HasExited } 6
    if (-not $gone -and $up) { [V3]::PostKey($h, 0x1B); Start-Sleep -Milliseconds 300; [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero); $gone = WaitFor { $d.HasExited } 5 }
    Check 'v040 tray off: closing the notice ends that process' $gone "exit=$(if ($gone) { $d.ExitCode })"
    StopPid $d

    $d = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError "$out\v040-daemon-off-stderr.txt" }; $null = $d.Handle
    $gone = WaitFor { $d.HasExited } 8
    $err = "$(Get-Content "$out\v040-daemon-off-stderr.txt" -ErrorAction SilentlyContinue)"
    Check 'v040 tray off: `rustshot daemon` (the login item) exits at once, exit 0, no tray window' ($gone -and $d.ExitCode -eq 0 -and -not (& $tray) -and [W]::Settings() -eq [IntPtr]::Zero) "exit=$(if ($gone) { $d.ExitCode }) stderr=[$err]"
    StopPid $d

    $p = WithEnv $vars { Start-Process $exe -PassThru }; $null = $p.Handle
    $up = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 10
    $h = [W]::Settings()
    Check 'v040 tray off: a plain launch opens Settings in its own process (no daemon)' ($up -and [W]::Owner($h) -eq $p.Id -and -not (& $tray)) "owner=$(if ($up) { [W]::Owner($h) }) pid=$($p.Id)"
    if ($up) { [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    $gone = WaitFor { $p.HasExited } 6
    Check 'v040 tray off: closing it ends that process (exit 0)' ($gone -and $p.ExitCode -eq 0) "exit=$(if ($gone) { $p.ExitCode })"
    StopPid $p

    $d = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon', '--force' -PassThru }; $null = $d.Handle
    $up = WaitFor { & $tray } 10
    Start-Sleep 1
    Check 'v040 tray off: `daemon --force` runs anyway' ($up -and -not $d.HasExited)
    StopPid $d

    # ------------------------------------------------ screenclip URI with no daemon: an on-demand capture
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'screenclip', 'ms-screenclip:?source=HotKey' -PassThru -RedirectStandardError "$out\v040-screenclip-stderr.txt" }; $null = $p.Handle
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
    $ov = [W]::Overlay()
    Start-Sleep 1
    Shot 'v040-screenclip-overlay'
    Check 'v040 screenclip: `rustshot screenclip "ms-screenclip:?source=HotKey"` opens a capture in that process (no daemon)' ($ovUp -and [W]::Owner($ov) -eq $p.Id) "owner=$(if ($ovUp) { [W]::Owner($ov) }) pid=$($p.Id)"
    $e = [V3]::Probe(0x0007, 0x7A)   # Ctrl+Alt+Shift+F11, the config's capture hotkey
    Check 'v040 screenclip: the on-demand run holds no global hotkeys (the capture key is free)' ($ovUp -and $e -eq 0) "err=$e"
    if ($ovUp) { [V3]::PostKey($ov, 0x1B) }   # Esc, posted to its own overlay
    $gone = WaitFor { $p.HasExited } 10
    Check 'v040 screenclip: Esc ends the capture and the process (exit 0)' ($gone -and $p.ExitCode -eq 0 -and -not [W]::OverlayVisible()) "gone=$gone exit=$(if ($gone) { $p.ExitCode }) overlay=$([W]::OverlayVisible())"
    StopPid $p

    # ------------------------------------------------ register-screenclip on / off (the sandbox's HKCU)
    $cls = 'HKCU:\Software\Classes\Rustshot.ScreenClip'
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'register-screenclip', 'on' -PassThru -Wait -RedirectStandardError "$out\v040-register-stderr.txt" }
    $cmd = (Get-ItemProperty "$cls\shell\open\command" -ErrorAction SilentlyContinue).'(default)'
    $proto = Get-ItemProperty $cls -ErrorAction SilentlyContinue
    $assoc = (Get-ItemProperty 'HKCU:\Software\Rustshot\Capabilities\URLAssociations' -ErrorAction SilentlyContinue).'ms-screenclip'
    $capName = (Get-ItemProperty 'HKCU:\Software\Rustshot\Capabilities' -ErrorAction SilentlyContinue).ApplicationName
    $regApp = (Get-ItemProperty 'HKCU:\Software\RegisteredApplications' -ErrorAction SilentlyContinue).Rustshot
    Check 'v040 register-screenclip on: exit 0' ($p.ExitCode -eq 0) "exit=$($p.ExitCode) $(Get-Content "$out\v040-register-stderr.txt" -ErrorAction SilentlyContinue)"
    Check 'v040 register-screenclip on: Rustshot.ScreenClip is a URL protocol whose command runs this exe' ($proto -and ($proto.PSObject.Properties.Name -contains 'URL Protocol') -and $cmd -and $cmd -like "*$([IO.Path]::GetFileName($exe))*") "command=[$cmd]"
    Check 'v040 register-screenclip on: Capabilities (ApplicationName Rustshot, ms-screenclip -> Rustshot.ScreenClip) and RegisteredApplications' ($capName -eq 'Rustshot' -and $assoc -eq 'Rustshot.ScreenClip' -and $regApp -eq 'Software\Rustshot\Capabilities') "app=$capName assoc=$assoc registered=$regApp"
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'register-screenclip', 'off' -PassThru -Wait }
    $left = (Test-Path $cls) -or [bool](Get-ItemProperty 'HKCU:\Software\RegisteredApplications' -ErrorAction SilentlyContinue).Rustshot -or [bool](Get-ItemProperty 'HKCU:\Software\Rustshot\Capabilities\URLAssociations' -ErrorAction SilentlyContinue).'ms-screenclip'
    Check 'v040 register-screenclip off: exit 0, the ProgID, association and RegisteredApplications value are gone' ($p.ExitCode -eq 0 -and -not $left) "exit=$($p.ExitCode) left=$left"

    # ------------------------------------------------ export border (direct capture)
    V4Config $appdata $save "border_width = 4`nborder_color = `"#ff0000`"`nhistory = false`n"
    $png = "$save\border.png"
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '400x300+100+100', '--path', $png -PassThru -Wait }
    $sz = PngSize $png
    $px = if (Test-Path $png) { Pixel $png 1 1 } else { '' }
    Check 'v040 border: border_width = 4 adds 4 px each side to `full --region 400x300` (408x308, red edge)' ($p.ExitCode -eq 0 -and $sz -eq '408x308' -and $px -eq 'ffff0000') "exit=$($p.ExitCode) size=$sz px(1,1)=$px"

    # ------------------------------------------------ image edits: annotations.json transforms, rendered by History's Open with
    V4Config $appdata $save "history = true`n"
    Remove-Item -Recurse -Force (Join-Path $local 'rustshot-data\history') -ErrorAction SilentlyContinue
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '400x300+100+100' -PassThru -Wait }
    $entry = HistEntries $local | Sort-Object { $_.FullName } | Select-Object -Last 1
    if ($p.ExitCode -eq 0 -and $entry) {
        # Crop 200x100 at (0,0), flip, a quarter turn (100x200), a 10 px magenta canvas: 120x220.
        Set-Content "$($entry.FullName)\annotations.json" '{"version":1,"crop":[0,0,200,100],"transforms":[{"op":"flip","axis":"h"},{"op":"rotate","deg":90},{"op":"canvas","pad":[10,10,10,10],"bg":"#ff00ff","radius":0,"shadow":false}],"objects":[]}' -NoNewline
        $m = Get-Content "$($entry.FullName)\meta.json" -Raw
        Set-Content "$($entry.FullName)\meta.json" ($m -replace '"edited":false', '"edited":true') -NoNewline
        $ow = "$env:TEMP\rustshot-open"
        $before = @(Get-ChildItem $ow -Filter *.png -ErrorAction SilentlyContinue).Count
        $hp = WithEnv $vars { Start-Process $exe -ArgumentList 'history' -PassThru }; $null = $hp.Handle
        $up = WaitFor { [W]::FindWindow('rustshot_window', 'Rustshot History') -ne [IntPtr]::Zero } 10
        $hw = [W]::FindWindow('rustshot_window', 'Rustshot History')
        Start-Sleep 2
        if ($up) { [void][W]::SetForegroundWindow($hw); Start-Sleep -Milliseconds 400; [W]::Chord(0x11, 0x41); Start-Sleep -Milliseconds 400; [W]::Chord(0x11, 0x4F) }   # Ctrl+A, Ctrl+O
        $wrote = WaitFor { @(Get-ChildItem $ow -Filter *.png -ErrorAction SilentlyContinue).Count -gt $before } 10
        Start-Sleep 2
        Shot 'v040-history-open-with'
        $f = Newest $ow
        $sz = if ($f) { PngSize $f.FullName } else { '' }
        $px = if ($f) { Pixel $f.FullName 2 2 } else { '' }
        Check 'v040 image edits: crop + flip + rotate 90 + 10 px canvas from annotations.json render as 120x220 with the canvas colour (History, Open with)' ($up -and $wrote -and $sz -eq '120x220' -and $px -eq 'ffff00ff') "window=$up wrote=$wrote size=$sz px(2,2)=$px"
        StopPid $hp   # the PID started above (the chooser is in it)
        $p = WithEnv $vars { Start-Process $exe -ArgumentList 'history', '--list' -PassThru -Wait -RedirectStandardOutput "$out\v040-hist-list.txt" }
        Add-Content $log "      history --list after the fixture: $((Get-Content "$out\v040-hist-list.txt" -ErrorAction SilentlyContinue) -join ' | ')"
    } else {
        Check 'v040 image edits: a History entry to edit' $false "exit=$($p.ExitCode)"
    }

    # ------------------------------------------------ freeform (lasso) snip: transparent outside
    V4Config $appdata $save "selection_mode = `"freeform`"`noverlay_hints = false`nmagnifier = false`n"
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui' -PassThru }; $null = $g.Handle
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    $before = @(Get-ChildItem $save -Recurse -Filter *.png).Count
    if ($ovUp) {
        [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
        # A triangle: (200,200) -> (600,200) -> (400,500) -> back.
        $pts = @(@(200, 200), @(600, 200), @(400, 500), @(200, 200))
        [void][V3]::SetCursorPos(200, 200); Start-Sleep -Milliseconds 100
        [V3]::mouse_event(2, 0, 0, 0, [IntPtr]::Zero)
        for ($i = 1; $i -lt $pts.Count; $i++) {
            $a = $pts[$i - 1]; $b = $pts[$i]
            for ($s = 1; $s -le 40; $s++) {
                [void][V3]::SetCursorPos([int]($a[0] + ($b[0] - $a[0]) * $s / 40), [int]($a[1] + ($b[1] - $a[1]) * $s / 40))
                Start-Sleep -Milliseconds 10
            }
        }
        [V3]::mouse_event(4, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 600
        Shot 'v040-freeform'
        [W]::Chord(0x11, 0x53)   # Ctrl+S
    }
    $saved = WaitFor { @(Get-ChildItem $save -Recurse -Filter *.png).Count -gt $before } 10
    $f = Newest $save
    $sz = if ($saved) { PngSize $f.FullName } else { '' }
    $hh = if ($sz -match 'x(\d+)$') { [int]$Matches[1] } else { 0 }
    $corner = if ($saved -and $hh -gt 4) { Pixel $f.FullName 2 ($hh - 3) } else { '' }
    $mid = if ($saved) { Pixel $f.FullName 200 60 } else { '' }
    Check 'v040 freeform: a triangle lasso saves its box (about 400x300) as a PNG with a transparent outside' ($saved -and $corner -like '00*' -and $mid -like 'ff*') "size=$sz corner=$corner inside=$mid"
    $null = WaitFor { $g.HasExited } 5
    StopPid $g

    # ------------------------------------------------ Text mode and Quick Redact on a window with an e-mail address
    $form = "$root\form.ps1"
    Set-Content $form @'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$f = New-Object System.Windows.Forms.Form
$f.FormBorderStyle = 'None'; $f.StartPosition = 'Manual'; $f.Location = New-Object System.Drawing.Point(100, 100)
$f.Size = New-Object System.Drawing.Size(760, 220); $f.BackColor = 'White'; $f.TopMost = $true
$l = New-Object System.Windows.Forms.Label
$l.Text = "Write to jane.doe@example.com`r`nServer 10.20.30.40 today"
$l.Font = New-Object System.Drawing.Font('Segoe UI', 26); $l.ForeColor = 'Black'; $l.AutoSize = $true; $l.Location = New-Object System.Drawing.Point(20, 20)
$f.Controls.Add($l)
[System.Windows.Forms.Application]::Run($f)
'@
    $fp = Start-Process powershell.exe -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $form -PassThru
    Start-Sleep 4
    V4Config $appdata $save "overlay_hints = false`nmagnifier = false`n"
    $plain = "$save\redact-plain.png"
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '760x220+100+100', '--path', $plain -PassThru -Wait }
    Check 'v040 redact: the text window is on screen (plain capture)' ($p.ExitCode -eq 0 -and (PngSize $plain) -eq '760x220') "exit=$($p.ExitCode) size=$(PngSize $plain)"
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', '760x220+100+100' -PassThru -RedirectStandardError "$out\v040-text-stderr.txt" }; $null = $g.Handle
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    $langs = '?'
    try {
        $null = [Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
        $langs = (@([Windows.Media.Ocr.OcrEngine]::AvailableRecognizerLanguages) | ForEach-Object { $_.LanguageTag }) -join ','
    } catch {}
    Add-Content $log "      OCR languages here: [$langs]"
    $text = ''
    if ($ovUp) {
        [System.Windows.Forms.Clipboard]::Clear()
        KeyPress 0x12   # Alt: lets this process hand the foreground over (the form has it)
        [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
        KeyPress 0x4F                                  # O: Text mode
        Start-Sleep 4
        Shot 'v040-text-mode'
    }
    Check 'v040 text mode: O (Copy text) keeps the overlay up (no crash, also without an OCR language)' ($ovUp -and [W]::OverlayVisible() -and -not $g.HasExited) "exited=$($g.HasExited) exit=$(if ($g.HasExited) { $g.ExitCode })"
    if (-not $langs) {
        Add-Content $log 'SKIP  v040 text mode / Quick Redact on the words: no Windows OCR language in this sandbox (it has no network to add one)'
        if ($ovUp -and -not $g.HasExited) {
            [W]::Chord(0x10, 0x52); Start-Sleep 3        # Shift+R: Quick Redact
            Shot 'v040-redact-no-ocr'
        }
        Check 'v040 redact: Shift+R keeps the overlay up without an OCR language' ($ovUp -and [W]::OverlayVisible() -and -not $g.HasExited) "exited=$($g.HasExited) exit=$(if ($g.HasExited) { $g.ExitCode })"
        if (-not $g.HasExited) { [V3]::PostKey([W]::Overlay(), 0x1B) }
    } else {
        if ($ovUp -and -not $g.HasExited) {
            [W]::Chord(0x11, 0x41); Start-Sleep -Milliseconds 400   # Ctrl+A
            [W]::Chord(0x11, 0x43); Start-Sleep -Milliseconds 800   # Ctrl+C
            try { $text = [System.Windows.Forms.Clipboard]::GetText() } catch {}
            KeyPress 0x1B; Start-Sleep -Milliseconds 500  # Esc: leave Text mode
        }
        Check 'v040 text mode: O, Ctrl+A, Ctrl+C copies the recognized words (the e-mail address)' ($text -match 'jane\.doe@example\.com') "clipboard=[$($text -replace "`r?`n", ' / ')]"
        Check 'v040 text mode: Esc leaves Text mode and keeps the capture' ($ovUp -and [W]::OverlayVisible() -and -not $g.HasExited)
        $before = @(Get-ChildItem $save -Recurse -Filter *.png).Count
        if ($ovUp -and -not $g.HasExited) {
            [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
            [W]::Chord(0x10, 0x52)   # Shift+R: Quick Redact
            Start-Sleep 5
            Shot 'v040-redact'
            [W]::Chord(0x11, 0x53)   # Ctrl+S
        }
        $saved = WaitFor { @(Get-ChildItem $save -Recurse -Filter *.png).Count -gt $before } 10
        $f = Newest $save
        $st = if ($saved) { DiffStats $plain $f.FullName } else { @{ n = -2; top = ''; share = 0 } }
        Check 'v040 redact: Shift+R covers the e-mail and the IP (the saved capture differs from the plain one)' ($saved -and $st.n -gt 300) "changed px (every 2nd)=$($st.n) file=$(if ($f) { $f.Name })"
        Check 'v040 redact: solid by default (one colour fills the changed pixels)' ($saved -and $st.n -gt 300 -and $st.share -ge 0.6) "top colour=#$($st.top) share=$($st.share)"
    }
    $e = HistEntries $local | Sort-Object { $_.FullName } | Select-Object -Last 1
    if ($e) { Add-Content $log "      redact entry objects: $(((Get-Content "$($e.FullName)\meta.json" -Raw | ConvertFrom-Json).objects) -join ',')" }
    $null = WaitFor { $g.HasExited } 5
    StopPid $g
    StopPid $fp
    $err = Get-Content "$out\v040-notice-stderr.txt", "$out\v040-screenclip-stderr.txt", "$out\v040-text-stderr.txt" -ErrorAction SilentlyContinue
    if ($err) { Add-Content $log ("      stderr: " + (($err | Select-Object -First 8) -join ' | ')) }
}
