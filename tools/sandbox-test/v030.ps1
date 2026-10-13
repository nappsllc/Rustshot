# 0.3.0 checks, dot-sourced by test.ps1 and run on the portable exe:
# the launcher (`rustshot launcher`, with and without a daemon, and a
# delayed capture through it with its countdown pill), a delayed region
# capture from the CLI, `--accept-on-select` with a region (no input),
# Open with (Ctrl+O) writing into a private %TEMP%\rustshot-open, the new
# config keys after a real Settings save (OK rewrites the whole file), and
# that the daemon takes no global Ctrl+O.
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped.
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class V3 {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  // A left click posted to the window at client point (x, y), as tests/e2e.rs does.
  public static void PostClientClick(IntPtr h, int x, int y) {
    var l = (IntPtr)((y << 16) | (x & 0xFFFF));
    PostMessage(h, 0x200, IntPtr.Zero, l); System.Threading.Thread.Sleep(150);
    PostMessage(h, 0x201, (IntPtr)1, l); System.Threading.Thread.Sleep(80);
    PostMessage(h, 0x202, IntPtr.Zero, l);
  }
  public static void PostKey(IntPtr h, uint vk) { PostMessage(h, 0x100, (IntPtr)vk, IntPtr.Zero); System.Threading.Thread.Sleep(60); PostMessage(h, 0x101, (IntPtr)vk, IntPtr.Zero); }
  [DllImport("user32.dll", SetLastError=true)] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
  public static void Click(int x, int y) { SetCursorPos(x, y); mouse_event(2, 0, 0, 0, IntPtr.Zero); mouse_event(4, 0, 0, 0, IntPtr.Zero); }
  // RegisterHotKey(mods, vk): 0 when free, else the Win32 error (1409: taken).
  public static int Probe(uint mods, uint vk) {
    if (RegisterHotKey(IntPtr.Zero, 0x5101, mods | 0x4000, vk)) { UnregisterHotKey(IntPtr.Zero, 0x5101); return 0; }
    return Marshal.GetLastWin32Error();
  }
}
"@

# Width x height of a PNG (its IHDR), or '' when it is not one.
function PngSize($path) {
    if (-not (Test-Path $path)) { return '' }
    $b = [System.IO.File]::ReadAllBytes($path)
    if ($b.Length -lt 24 -or [System.Text.Encoding]::ASCII.GetString($b, 1, 3) -ne 'PNG') { return '' }
    $w = ([int]$b[16] -shl 24) -bor ([int]$b[17] -shl 16) -bor ([int]$b[18] -shl 8) -bor $b[19]
    $h = ([int]$b[20] -shl 24) -bor ([int]$b[21] -shl 16) -bor ([int]$b[22] -shl 8) -bor $b[23]
    "${w}x${h}"
}

function V3Config($appdata, $save, $extra = '') {
    $esc = $save.Replace('\', '\\')
    Set-Content "$appdata\rustshot\config.toml" (@"
save_path = "$esc"
capture_hotkey = "Ctrl+Alt+Shift+F11"
quit_hotkey = "Ctrl+Alt+Shift+F10"
check_updates = false
show_thumbnail = false
"@ + "`n" + $extra)
}

function KeyPress([byte]$vk) { [W]::keybd_event($vk, 0, 0, [IntPtr]::Zero); [W]::keybd_event($vk, 0, 2, [IntPtr]::Zero) }
function Launcher() { [W]::FindWindow('rustshot_window', 'Rustshot Capture') }
function NoticeVisible() {
    $h = [W]::FindWindow('rustshot_notice', [NullString]::Value)
    $h -ne [IntPtr]::Zero -and [V3]::IsWindowVisible($h)
}

function Checks030($exe) {
    $root = "$env:TEMP\v030-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $cfgPath = "$appdata\rustshot\config.toml"
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-030' }
    V3Config $appdata $save

    # ------------------------------------------------ CLI: a delayed region capture (headless)
    $png = "$save\delayed.png"
    $t0 = Get-Date
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '400x300+100+100', '--delay', '3000', '--path', $png -PassThru -Wait }
    $ms = [int]((Get-Date) - $t0).TotalMilliseconds
    $sz = PngSize $png
    Check 'v030 cli: `full --region 400x300+100+100 --delay 3000` waits >= 3 s, saves 400x300' ($p.ExitCode -eq 0 -and $ms -ge 3000 -and $sz -eq '400x300') "exit=$($p.ExitCode) took=$ms ms size=$sz"

    # ------------------------------------------------ CLI: --accept-on-select with a region exports at once
    $png = "$save\aos.png"
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', '300x200+50+50', '--accept-on-select', '--path', $png -PassThru }
    $done = WaitFor { $g.HasExited } 15
    $sz = PngSize $png
    Check 'v030 cli: `gui --region 300x200+50+50 --accept-on-select --path` exports with no input' ($done -and $g.ExitCode -eq 0 -and $sz -eq '300x200') "exited=$done exit=$(if ($done) { $g.ExitCode }) size=$sz"
    if (-not $done) { Stop-Process -Id $g.Id -Force -ErrorAction SilentlyContinue }
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--accept-on-select' -PassThru -Wait -RedirectStandardError "$out\v030-aos-err.txt" }
    $err = "$(Get-Content "$out\v030-aos-err.txt" -ErrorAction SilentlyContinue)"
    Check 'v030 cli: `full --accept-on-select` without a region is refused' ($p.ExitCode -ne 0 -and $err -match 'needs a region') "exit=$($p.ExitCode) $err"

    # ------------------------------------------------ launcher without a daemon: its own window
    $l = WithEnv $vars { Start-Process $exe -ArgumentList 'launcher' -PassThru }
    $up = WaitFor { (Launcher) -ne [IntPtr]::Zero } 10
    $h = Launcher
    Check 'v030 launcher: `rustshot launcher` (no daemon) opens "Rustshot Capture" in its process' ($up -and [W]::Owner($h) -eq $l.Id) "owner=$(if ($up) { [W]::Owner($h) }) pid=$($l.Id)"
    Start-Sleep 1
    Shot 'v030-launcher'
    if ($up) { [void][W]::SetForegroundWindow($h); Start-Sleep -Milliseconds 300; KeyPress 0x1B }   # Esc: cancel
    $gone = WaitFor { $l.HasExited } 6
    Check 'v030 launcher: Esc closes it and ends that process (exit 0), nothing captured' ($gone -and $l.ExitCode -eq 0 -and -not [W]::OverlayVisible()) "exit=$(if ($gone) { $l.ExitCode })"
    if (-not $gone) { Stop-Process -Id $l.Id -Force -ErrorAction SilentlyContinue }

    # ------------------------------------------------ launcher in the daemon: Screen, 3 s, Save
    V3Config $appdata $save "launcher_source = `"screen`"`nlauncher_delay = 3`nlauncher_after = `"save`"`n"
    $dm = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError "$out\v030-daemon-stderr.txt" }
    $dUp = WaitFor { [W]::FindWindow('rustshot_tray_sbx-030', [NullString]::Value) -ne [IntPtr]::Zero } 10
    Check 'v030 launcher: isolated daemon up' ($dUp -and -not $dm.HasExited) "pid=$($dm.Id)"
    Start-Sleep 1
    # The daemon takes no global Ctrl+O (Open with is an overlay/History key).
    $e = [V3]::Probe(0x0002, [uint32][char]'O')
    Check 'v030 hotkey: Ctrl+O is not a global hotkey (free while the daemon runs)' ($e -eq 0) "err=$e"
    $s = WithEnv $vars { Start-Process $exe -ArgumentList 'launcher' -PassThru -Wait }
    $up = WaitFor { (Launcher) -ne [IntPtr]::Zero } 8
    $h = Launcher
    Check 'v030 launcher: `rustshot launcher` hands over to the daemon (exit 0, window in the daemon)' ($s.ExitCode -eq 0 -and $up -and [W]::Owner($h) -eq $dm.Id) "exit=$($s.ExitCode) owner=$(if ($up) { [W]::Owner($h) }) daemon=$($dm.Id)"
    $before = @(Get-ChildItem $save -Recurse -Filter *.png).Count
    if ($up) { [void][W]::SetForegroundWindow($h); Start-Sleep -Milliseconds 300; KeyPress 0x0D }   # Enter: Capture
    $t0 = Get-Date
    $pill = WaitFor { NoticeVisible } 3
    if ($pill) { Shot 'v030-countdown' }
    Check 'v030 launcher: Enter closes it and shows the countdown pill' ($pill -and (Launcher) -eq [IntPtr]::Zero)
    $saved = WaitFor { @(Get-ChildItem $save -Recurse -Filter *.png).Count -gt $before } 15
    $ms = [int]((Get-Date) - $t0).TotalMilliseconds
    $f = Get-ChildItem $save -Recurse -Filter *.png | Sort-Object LastWriteTime | Select-Object -Last 1
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    Check 'v030 launcher: Screen / 3 s / Save saves the screen after >= 3 s' ($saved -and $ms -ge 2900 -and $ms -le 12000) "took=$ms ms file=$(if ($f) { $f.Name }) size=$(if ($f) { PngSize $f.FullName }) screen=$($b.Width)x$($b.Height)"
    # (the "Saved" notice that follows uses the same window class)
    Check 'v030 launcher: the daemon keeps running, the pill (and the Saved notice) go' ((WaitFor { -not (NoticeVisible) } 15) -and -not $dm.HasExited)
    $cfg = Get-Content $cfgPath -Raw
    Check 'v030 launcher: the choice is remembered (launcher_* in config.toml)' ($cfg -match '(?m)^launcher_source = "screen"' -and $cfg -match '(?m)^launcher_delay = 3\b' -and $cfg -match '(?m)^launcher_after = "save"')
    Stop-Process -Id $dm.Id -Force -ErrorAction SilentlyContinue   # the PID started above
    $null = WaitFor { $dm.HasExited } 5

    # ------------------------------------------------ Open with (Ctrl+O) writes into a private temp folder
    $ow = "$env:TEMP\rustshot-open"
    $existed = Test-Path $ow
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', '400x300+100+100' -PassThru -RedirectStandardError "$out\v030-openwith-stderr.txt" }
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 8
    Start-Sleep 1
    if ($ovUp) { [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300; [W]::Chord(0x11, 0x4F) }   # Ctrl+O
    $wrote = WaitFor { @(Get-ChildItem $ow -Filter *.png -ErrorAction SilentlyContinue).Count -ge 1 } 10
    Start-Sleep 2
    Shot 'v030-open-with'
    $f = Get-ChildItem $ow -Filter *.png -ErrorAction SilentlyContinue | Select-Object -First 1
    Check 'v030 open with: Ctrl+O writes the 400x300 capture into %TEMP%\rustshot-open' ($ovUp -and -not $existed -and $wrote -and (PngSize $f.FullName) -eq '400x300') "overlay=$ovUp existed=$existed file=$(if ($f) { $f.Name + ' ' + (PngSize $f.FullName) })"
    if (Test-Path $ow) {
        $acl = Get-Acl $ow
        $me = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
        # Only the user, SYSTEM and Administrators (the owner when the user is an admin).
        $ok = @($me, 'NT AUTHORITY\SYSTEM', 'BUILTIN\Administrators')
        $open = @($acl.Access | Where-Object { $_.AccessControlType -eq 'Allow' -and $ok -notcontains "$($_.IdentityReference)" })
        $ids = ($acl.Access | ForEach-Object { "$($_.IdentityReference):$($_.FileSystemRights)" }) -join '; '
        Check 'v030 open with: the folder is private (access only for the user, SYSTEM, Administrators)' ($open.Count -eq 0 -and $ok -contains $acl.Owner) "owner=$($acl.Owner) me=$me acl=[$ids]"
    }
    Stop-Process -Id $g.Id -Force -ErrorAction SilentlyContinue   # the PID started above (the chooser is in it)
    $null = WaitFor { $g.HasExited } 5

    # ------------------------------------------------ config keys after a Settings save
    # Non-default values for the 0.3.0 keys; Settings OK after one change
    # (Play shutter sound) rewrites the whole file through to_toml.
    V3Config $appdata $save @"
magnifier = false
magnifier_square = true
accept_on_select = "save"
overlay_hints = false
launcher_source = "window"
launcher_delay = 7
launcher_after = "pin"
"@
    # In an isolated daemon (as the tray's Settings...), with posted clicks
    # as tests/e2e.rs does. A click also focuses the toggle (Enter would
    # flip it back), so OK is clicked too. Client points at 100 % (680 px
    # wide): the rail's Capture (60, 93), then Capture > After capture >
    # Play shutter sound (641, 304), OK (461, 497) (`sandbox_click_points`
    # in src/settings_ui/tests.rs checks the first two).
    $dm = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru }
    $null = WaitFor { [W]::FindWindow('rustshot_tray_sbx-030', [NullString]::Value) -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    $s = WithEnv $vars { Start-Process $exe -ArgumentList 'settings' -PassThru -Wait }
    $up = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 10
    Start-Sleep 2
    $h = [W]::Settings()
    if ($up) {
        $c = New-Object V3+RECT
        [void][V3]::GetClientRect($h, [ref]$c)
        $sc = $c.R / 680.0
        [V3]::PostClientClick($h, [int](60 * $sc), [int](93 * $sc))
        Start-Sleep -Milliseconds 300
        [V3]::PostClientClick($h, [int](641 * $sc), [int](304 * $sc))
        Start-Sleep -Milliseconds 500
        Shot 'v030-settings-toggled'
        [V3]::PostClientClick($h, [int](461 * $sc), [int](497 * $sc))   # OK
        if (-not (WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 3)) { Shot 'v030-settings-modal'; [V3]::PostKey($h, 0x0D) }   # the rewrite question's Save
    }
    $closed = WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 6
    $cfg = Get-Content $cfgPath -Raw
    Copy-Item $cfgPath "$out\v030-config-after-settings.toml"
    $want = @('magnifier = false', 'magnifier_square = true', 'accept_on_select = "save"', 'overlay_hints = false', 'launcher_source = "window"', 'launcher_delay = 7', 'launcher_after = "pin"')
    $miss = @($want | Where-Object { $cfg -notmatch ('(?m)^' + [regex]::Escape($_) + '\s*$') })
    $rewritten = $cfg -match '(?m)^shutter_sound = true' -and $cfg -match '(?m)^theme = '
    Check 'v030 config: Settings OK rewrote the file (shutter_sound = true, every key written)' ($s.ExitCode -eq 0 -and $up -and $closed -and $rewritten) "window=$up closed=$closed"
    Check 'v030 config: magnifier, magnifier_square, accept_on_select, overlay_hints and launcher_* survive the Settings save' ($rewritten -and $miss.Count -eq 0) "missing: $($miss -join ' | ')"
    Check 'v030 config: the daemon keeps running after the save' (-not $dm.HasExited)
    Stop-Process -Id $dm.Id -Force -ErrorAction SilentlyContinue   # the PID started above
    $null = WaitFor { $dm.HasExited } 5
    $err = Get-Content "$out\v030-daemon-stderr.txt", "$out\v030-openwith-stderr.txt" -ErrorAction SilentlyContinue
    if ($err) { Add-Content $log ("      stderr: " + (($err | Select-Object -First 6) -join ' | ')) }
}
