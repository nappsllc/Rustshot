# rustshot clean-system test, runs inside Windows Sandbox at logon.
# Inputs:  <T>\in\  (CI artifacts + sideload.msix/.cer), <T>\old\ (an older *-setup.exe)
# Outputs: <T>\out\
$ErrorActionPreference = 'Continue'
$T   = 'C:\Users\WDAGUtilityAccount\Desktop\test'
$out = "$T\out"
New-Item -ItemType Directory -Force $out | Out-Null
$log = "$out\results.txt"
Set-Content $log "rustshot sandbox test $(Get-Date -Format s)"
function Check($name, $ok, $detail = '') {
    $line = "{0}  {1}  {2}" -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $detail
    Add-Content $log $line
}
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string c, string t);
  public static bool OverlayVisible() {
    bool v = false;
    EnumWindows((h, l) => { var s = new StringBuilder(64); GetClassName(h, s, 64);
      if (s.ToString() == "rustshot_overlay" && IsWindowVisible(h)) v = true; return true; }, IntPtr.Zero);
    return v;
  }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  public static bool TrayWindow() { return FindWindow("rustshot_tray", null) != IntPtr.Zero; }
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
  // The Settings window: wind_win's "rustshot_window" class, title "Rustshot \u2014 Settings".
  public static IntPtr Settings() { return FindWindow("rustshot_window", "Rustshot \u2014 Settings"); }
  public static uint Owner(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
  public static IntPtr Overlay() {
    IntPtr r = IntPtr.Zero;
    EnumWindows((h, l) => { var s = new StringBuilder(64); GetClassName(h, s, 64);
      if (s.ToString() == "rustshot_overlay" && IsWindowVisible(h)) { r = h; return false; } return true; }, IntPtr.Zero);
    return r;
  }
  public static void Chord(byte mod, byte key) {   // real key presses (GetKeyState sees them)
    keybd_event(mod, 0, 0, IntPtr.Zero); keybd_event(key, 0, 0, IntPtr.Zero);
    keybd_event(key, 0, 2, IntPtr.Zero); keybd_event(mod, 0, 2, IntPtr.Zero);
  }
}
"@
function Shot($name) {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save("$out\$name.png"); $g.Dispose(); $bmp.Dispose()
}
function WaitFor([scriptblock]$cond, $sec = 8) {
    $end = (Get-Date).AddSeconds($sec)
    while ((Get-Date) -lt $end) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 250 }
    return $false
}
function KillAll { Get-Process rustshot -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep 1 }

# Recording (video Task 9): RecordingChecks, run on the portable exe below.
. "$T\rec.ps1"
# Win+Shift+<key> availability on the clean system (default hotkey choice).
. "$T\probe.ps1"
# The Apps & features entry (0.2.0): UninstKeyChecks.
. "$T\uninst.ps1"
# History (0.2.0): HistoryChecks, run on the portable exe below.
. "$T\history.ps1"
# 0.3.0 (launcher, delayed capture, accept on select, Open with, new keys): Checks030.
. "$T\v030.ps1"
# 0.4.0 (tray icon off, on-demand screenclip, register-screenclip, border,
# image edits, freeform, Text mode, Quick Redact): Checks040.
. "$T\v040.ps1"
# 0.4.1 (the exe's small icons as the shell extracts them, the in-repo PNG
# encoder read back by GDI+ and WIC): Checks041.
. "$T\v041.ps1"
# Start menu / desktop shortcut names over 0.1.0's lowercase ones.
. "$T\shortcuts.ps1"
# Data files downloaded on demand (0.5.0): ChecksData, NoDataFolders.
. "$T\data.ps1"
# 0.5.0 (languages offline and from hand-placed files, Settings' Language
# list, ruler, Read QR code, Scrolling capture): Checks050.
. "$T\v050.ps1"
# 0.6.0 (Settings > About opens the powernapps.net short link; Apache-2.0): Checks060.
. "$T\v060.ps1"
# 0.6.2 (the tray icon is the drawn app icon, no fallback): Checks062.
. "$T\v062.ps1"
# 0.6.3 (the pixel-aligned 16 px icon, window titles, the unsaved marker,
# DWM caption / text / border colours): Checks063.
. "$T\v063.ps1"
# 0.6.4 (a window hidden from capture gets the placeholder, Settings 552 px
# tall, the six Shortcuts sections): Checks064.
. "$T\v064.ps1"
# The sideload-signed MSIX (and an update over the older release's): MsixChecks.
. "$T\msix.ps1"
HotkeyProbe $log
if (Test-Path "$T\probe-only") { Add-Content $log 'DONE'; return }
function PortableExe {
    $p = Get-ChildItem "$T\in" -Recurse -Filter '*-windows-x86_64.exe' | Select-Object -First 1
    if (-not $p) { $p = Get-ChildItem "$T\in\rustshot-windows-x86_64" -Filter '*.exe' | Select-Object -First 1 }
    $p
}
# <T>\v030-only: just the 0.3.0 checks.
if (Test-Path "$T\v030-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks030 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v040-only: just the 0.4.0 checks.
if (Test-Path "$T\v040-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks040 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v041-only: just the 0.4.1 checks.
if (Test-Path "$T\v041-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks041 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v050-only: just the 0.5.0 checks.
if (Test-Path "$T\v050-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks050 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v060-only: just the 0.6.0 checks.
if (Test-Path "$T\v060-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks060 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v062-only: just the 0.6.2 checks.
if (Test-Path "$T\v062-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks062 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v063-only: just the 0.6.3 checks.
if (Test-Path "$T\v063-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks063 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\v064-only: just the 0.6.4 checks.
if (Test-Path "$T\v064-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    Checks064 "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\data-only: just the data-file checks.
if (Test-Path "$T\data-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    ChecksData "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}
# <T>\msix-only: just the MSIX checks.
if (Test-Path "$T\msix-only") {
    MsixChecks
    Add-Content $log 'DONE'
    return
}
# <T>\rec-only: just the recording checks.
if (Test-Path "$T\rec-only") {
    $portable = PortableExe
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    Add-Content $log "      portable exe: $($portable.FullName) $((Get-Item $portable.FullName).Length) B"
    RecordingChecks "$env:TEMP\rustshot.exe"
    Add-Content $log 'DONE'
    return
}

# ---------------------------------------------------------------- NSIS installer
$setup = Get-ChildItem "$T\in" -Recurse -Filter '*-setup.exe' | Select-Object -First 1
# What 0.1.0 left: lowercase Start menu folder and shortcuts, a desktop one.
OldShortcuts "$env:LOCALAPPDATA\rustshot\rustshot.exe"
$p = Start-Process $setup.FullName -ArgumentList '/S' -Wait -PassThru
Check 'nsis: silent install exits 0' ($p.ExitCode -eq 0) "exit=$($p.ExitCode) ($($setup.Name))"
$inst = "$env:LOCALAPPDATA\rustshot"
$exe = "$inst\rustshot.exe"
Check 'nsis: rustshot.exe installed' (Test-Path $exe) $exe
Check 'nsis: LICENSE + NOTICE + THIRD_PARTY_NOTICES.md installed' ((Test-Path "$inst\LICENSE") -and (Test-Path "$inst\NOTICE") -and (Test-Path "$inst\THIRD_PARTY_NOTICES.md"))
Check 'nsis: LICENSE is the Apache License 2.0, NOTICE names Rustshot' (((Get-Content "$inst\LICENSE" -Raw -ErrorAction SilentlyContinue) -match 'Apache License') -and ((Get-Content "$inst\NOTICE" -Raw -ErrorAction SilentlyContinue) -match 'Rustshot'))
NoDataFolders $inst 'nsis'
$lnk = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Rustshot\Rustshot.lnk"
Check 'nsis: Start menu shortcut' (Test-Path $lnk)
ShortcutChecks $exe
$run = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).rustshot
Check 'nsis: start-with-Windows Run value' ([bool]$run) "$run"
$un = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\rustshot' -ErrorAction SilentlyContinue
Check 'nsis: Apps & features entry' ([bool]$un) "version=$($un.DisplayVersion)"
$newVersion = if ($setup.Name -match 'rustshot-(\d+\.\d+\.\d+)-setup') { $Matches[1] } else { '?' }
UninstKeyChecks 'nsis:' $inst $newVersion
$vi = (Get-Item $exe).VersionInfo
Check 'exe: version resource' ($vi.FileDescription -eq 'rustshot') "$($vi.FileDescription) $($vi.ProductVersion) $($vi.CompanyName)"
[System.Drawing.Icon]::ExtractAssociatedIcon($exe).ToBitmap().Save("$out\exe-icon.png")
Check 'exe: icon extracted to out\exe-icon.png' (Test-Path "$out\exe-icon.png")

# ---------------------------------------------------------------- first launch = daemon, no capture
$a = Start-Process $exe -PassThru -RedirectStandardError "$out\daemon-stderr.txt"
$trayUp = WaitFor { [W]::TrayWindow() }
Check 'launch (no args): daemon keeps running' (-not $a.HasExited)
Check 'launch (no args): tray window present' $trayUp
Start-Sleep 1
Check 'launch (no args): NO capture overlay' (-not [W]::OverlayVisible())
# The daemon's default globals: held by it now (1409 = already registered).
foreach ($k in 'X','F','E','G','Z','H') {
    $e = [HK]::Probe([uint32][char]$k)
    Check "hotkey: daemon holds Win+Shift+$k" ($e -eq 1409) "probe err=$e"
}
Shot 'nsis-1-after-launch'
# The new defaults work as real key presses: Win+Shift+G (Record) and
# Win+Shift+Z (Copy text) each open the overlay; Esc closes it.
function WinShift([byte]$key) {
    foreach ($m in 0x5B, 0x10) { [W]::keybd_event($m, 0, 0, [IntPtr]::Zero) }
    [W]::keybd_event($key, 0, 0, [IntPtr]::Zero); [W]::keybd_event($key, 0, 2, [IntPtr]::Zero)
    foreach ($m in 0x10, 0x5B) { [W]::keybd_event($m, 0, 2, [IntPtr]::Zero) }
}
foreach ($pair in @(@('G', 'Record'), @('Z', 'Copy text'))) {
    WinShift ([byte][char]$pair[0])
    $ov = WaitFor { [W]::OverlayVisible() } 6
    Check "hotkey: Win+Shift+$($pair[0]) ($($pair[1])) opens the overlay in the installed daemon" $ov
    if ($ov) { Shot "hotkey-$($pair[0])"; [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300; [W]::keybd_event(0x1B, 0, 0, [IntPtr]::Zero); [W]::keybd_event(0x1B, 0, 2, [IntPtr]::Zero) }
    $null = WaitFor { -not [W]::OverlayVisible() } 5
    Start-Sleep 1
}

# ---------------------------------------------------------------- second launch = capture
$b = Start-Process $exe -PassThru -Wait
Check 'second launch: exits 0' ($b.ExitCode -eq 0) "exit=$($b.ExitCode)"
$ov = WaitFor { [W]::OverlayVisible() } 5
Check 'second launch: capture overlay shown in the daemon' $ov
Start-Sleep 1
Shot 'nsis-2-overlay'
Check 'second launch: still one rustshot process' (@(Get-Process rustshot -ErrorAction SilentlyContinue).Count -eq 1)
KillAll
# Which default hotkeys the daemon could not register (its own warnings).
$warn = @(Get-Content "$out\daemon-stderr.txt" -ErrorAction SilentlyContinue | Where-Object { $_ -match 'could not register hotkey' })
$failed = @($warn | ForEach-Object { if ($_ -match 'hotkey "([^"]+)"') { $Matches[1] } })
Add-Content $log "      daemon could not register: $(if ($failed) { $failed -join ' ' } else { '(none)' })"
Check 'hotkey: the daemon registers every default global hotkey (no warnings)' ($warn.Count -eq 0) "$($failed -join ' ')"
$newHash = (Get-FileHash $exe).Hash

# ---------------------------------------------------------------- Settings window (daemon)
$d = Start-Process $exe -PassThru
$null = WaitFor { [W]::TrayWindow() }
Start-Sleep 1
$s = Start-Process $exe -ArgumentList 'settings' -PassThru -Wait
Check 'settings: `rustshot settings` hands over to the daemon (exit 0)' ($s.ExitCode -eq 0) "exit=$($s.ExitCode)"
$sw = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 8
$h = [W]::Settings()
Check 'settings: "Rustshot Settings" window opens' $sw
Check 'settings: window belongs to the daemon' ($sw -and [W]::Owner($h) -eq $d.Id) "owner=$(if ($sw) { [W]::Owner($h) }) daemon=$($d.Id)"
Start-Sleep 1
Shot 'settings-window'
if ($sw) { [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }   # WM_CLOSE
Check 'settings: closes, daemon keeps running' ((WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 5) -and -not $d.HasExited)
KillAll

# ---------------------------------------------------------------- saving into the dated folder (defaults)
$day = Join-Path ([Environment]::GetFolderPath('MyPictures')) ("rustshot\" + (Get-Date -Format 'yyyy-MM-dd'))
$f = Start-Process $exe -ArgumentList 'full' -PassThru -Wait
$files = @(Get-ChildItem $day -Filter *.png -ErrorAction SilentlyContinue)
Check 'save: `rustshot full` (no --path) exits 0' ($f.ExitCode -eq 0) "exit=$($f.ExitCode)"
Check 'save: lands in Pictures\rustshot\<date>\ as PNG' ($files.Count -eq 1) "$day : $($files.Name -join ', ')"
# Ctrl+S in the editor (real key presses) saves without a dialog into the same folder.
$g = Start-Process $exe -ArgumentList 'gui','--region','400x300+100+100' -PassThru
$ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 8
Start-Sleep 1
if ($ovUp) { [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300; [W]::Chord(0x11, 0x53) }   # Ctrl+S
$gExit = WaitFor { $g.HasExited } 10
$files = @(Get-ChildItem $day -Filter *.png -ErrorAction SilentlyContinue)
Check 'save: Ctrl+S in the editor exits 0 without a dialog' ($gExit -and $g.ExitCode -eq 0) "exited=$gExit exit=$(if ($gExit) { $g.ExitCode })"
Check 'save: Ctrl+S file in the dated folder (collision suffix)' ($files.Count -eq 2) "$($files.Name -join ', ')"
if (-not $gExit) { Stop-Process -Id $g.Id -Force -ErrorAction SilentlyContinue }
$bmp = Join-Path $day 'cli.bmp'
$c = Start-Process $exe -ArgumentList 'full','--path',$bmp -PassThru -Wait
$hdr = if (Test-Path $bmp) { [System.IO.File]::ReadAllBytes($bmp)[0..1] } else { @() }
Check 'save: --path x.bmp writes a BMP' ($c.ExitCode -eq 0 -and ($hdr -join ',') -eq '66,77') "exit=$($c.ExitCode)"
KillAll

# ---------------------------------------------------------------- uninstall
Start-Process "$inst\uninstall.exe" -ArgumentList '/S' -Wait
$gone = WaitFor { -not (Test-Path $exe) } 15
Check 'uninstall: exe removed' $gone
Check 'uninstall: Run value removed' (-not (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).rustshot)
Check 'uninstall: Start menu folder removed' (-not (Test-Path (Split-Path $lnk)))
ShortcutsGone
Check 'uninstall: Apps & features entry removed' (-not (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\rustshot'))
Check 'uninstall: install folder removed (History is not in it)' (-not (Test-Path $inst)) "$(if (Test-Path $inst) { (Get-ChildItem $inst -Recurse -Name) -join ', ' })"
$histRoot = "$env:LOCALAPPDATA\rustshot-data\history"
function InstEntries { @(Get-ChildItem $histRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object { Get-ChildItem $_.FullName -Directory } | Where-Object { Test-Path "$($_.FullName)\meta.json" }) }
$kept = InstEntries
Check 'history (installed): the CLI captures above left entries' ($kept.Count -ge 2) "entries=$($kept.Count)"
Check 'uninstall: History is left in place (user data)' ($kept.Count -ge 2)
Add-Content $log "      config.toml present after uninstall: $(Test-Path "$env:APPDATA\rustshot\config.toml")"

# ---------------------------------------------------------------- installer update: old build -> this build
# What the in-app update does for an installer install: it starts the new
# setup.exe "/S /RELAUNCH" and the daemon exits; NSIS waits for the old
# daemon's tray window, installs, and starts "rustshot daemon --after-update".
$old = Get-ChildItem "$T\old" -Filter '*-setup.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($old) {
    $p = Start-Process $old.FullName -ArgumentList '/S' -Wait -PassThru
    $oldHash = (Get-FileHash $exe -ErrorAction SilentlyContinue).Hash
    Check 'upgrade: old build installed' ($p.ExitCode -eq 0 -and $oldHash -and $oldHash -ne $newHash) "($($old.Name))"
    $o = Start-Process $exe -PassThru
    $null = WaitFor { [W]::TrayWindow() }
    Start-Sleep 1
    # A setting changed under the old build (Capture window on Win+Shift+Q).
    $cfgPath = "$env:APPDATA\rustshot\config.toml"
    New-Item -ItemType Directory -Force (Split-Path $cfgPath) | Out-Null
    $cfg = if (Test-Path $cfgPath) { Get-Content $cfgPath -Raw } else { '' }
    if ($cfg -match '(?m)^capture_window_hotkey\s*=') { $cfg = $cfg -replace '(?m)^capture_window_hotkey\s*=.*$', 'capture_window_hotkey = "Shift+Meta+Q"' }
    else { $cfg += "`ncapture_window_hotkey = `"Shift+Meta+Q`"`n" }
    Set-Content $cfgPath $cfg -NoNewline
    $histBefore = (InstEntries).Count
    $u = Start-Process $setup.FullName -ArgumentList '/S','/RELAUNCH' -PassThru
    Start-Sleep 1
    Stop-Process -Id $o.Id -Force     # the old daemon exits after starting the installer
    $done = WaitFor { $u.HasExited } 30
    Check 'upgrade: setup /S /RELAUNCH exits 0' ($done -and $u.ExitCode -eq 0) "exit=$(if ($done) { $u.ExitCode })"
    Check 'upgrade: new exe in place' ((Get-FileHash $exe).Hash -eq $newHash)
    $up = WaitFor { [W]::TrayWindow() } 10
    $np = Get-CimInstance Win32_Process -Filter "Name='rustshot.exe'" | Select-Object -First 1
    Check 'upgrade: new daemon relaunched (tray window)' ($up -and $np -and $np.ProcessId -ne $o.Id) "$($np.CommandLine)"
    Check 'upgrade: Run value and Start menu shortcut kept' ([bool](Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).rustshot -and (Test-Path $lnk))
    UpgradeShortcutChecks $setup.FullName $exe
    # "/S /RELAUNCH" is what the in-app update runs: the 0.1.x entry is rewritten.
    UninstKeyChecks 'upgrade (in-app update path):' $inst $newVersion
    Start-Sleep 2
    Check 'upgrade: config.toml keeps the old build''s setting' ((Get-Content $cfgPath -Raw) -match 'capture_window_hotkey = "Shift\+Meta\+Q"')
    $e = [HK]::Probe([uint32][char]'Q')
    Check 'upgrade: the new daemon uses it (holds Win+Shift+Q)' ($e -eq 1409) "probe err=$e"
    $g = [HK]::Probe([uint32][char]'G'); $z = [HK]::Probe([uint32][char]'Z')
    Check 'upgrade: a config without record/copy-text keys gets Win+Shift+G and Win+Shift+Z' ($g -eq 1409 -and $z -eq 1409) "G err=$g Z err=$z"
    $histAfter = (InstEntries).Count
    Check 'upgrade: History entries survive the old install and the update' ($histBefore -ge 2 -and $histAfter -eq $histBefore) "before=$histBefore after=$histAfter"
    $f = Start-Process $exe -ArgumentList 'full' -PassThru -Wait
    $histNew = (InstEntries).Count
    Check 'upgrade: an old config without history keys keeps History on (new entry)' ($f.ExitCode -eq 0 -and $histNew -eq $histAfter + 1) "exit=$($f.ExitCode) $histAfter -> $histNew"
    KillAll
    Start-Process "$inst\uninstall.exe" -ArgumentList '/S' -Wait
    $null = WaitFor { -not (Test-Path $exe) } 15
} else {
    Check 'upgrade: old setup present in <T>\old' $false
}

# ---------------------------------------------------------------- MSIX (sideload-signed copy)
MsixChecks

# ---------------------------------------------------------------- portable exe: tray stays up for a visual check
$portable = Get-ChildItem "$T\in" -Recurse -Filter '*-windows-x86_64.exe' | Select-Object -First 1
if (-not $portable) { $portable = Get-ChildItem "$T\in\rustshot-windows-x86_64" -Filter '*.exe' | Select-Object -First 1 }
if ($portable) {
    Copy-Item $portable.FullName "$env:TEMP\rustshot.exe"
    # No daemon running: `rustshot settings` shows the window in its own process.
    $s = Start-Process "$env:TEMP\rustshot.exe" -ArgumentList 'settings' -PassThru
    $sw = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 8
    $h = [W]::Settings()
    Check 'portable: `rustshot settings` without a daemon opens the window itself' ($sw -and [W]::Owner($h) -eq $s.Id)
    if ($sw) { [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    Check 'portable: closing it ends that process (exit 0)' ((WaitFor { $s.HasExited } 5) -and $s.ExitCode -eq 0) "exit=$($s.ExitCode)"
    Add-Content $log "SKIP  portable: self-update from a local fixture (the release exe has no test hook: RUSTSHOT_UPDATE_FEED is debug-only, and the sandbox has no network; covered on the host by update_install::tests::live_portable_update)"
    HistoryChecks "$env:TEMP\rustshot.exe"
    Checks030 "$env:TEMP\rustshot.exe"
    Checks040 "$env:TEMP\rustshot.exe"
    Checks041 "$env:TEMP\rustshot.exe"
    ChecksData "$env:TEMP\rustshot.exe"
    Checks050 "$env:TEMP\rustshot.exe"
    Checks060 "$env:TEMP\rustshot.exe"
    Checks062 "$env:TEMP\rustshot.exe"
    Checks063 "$env:TEMP\rustshot.exe"
    Checks064 "$env:TEMP\rustshot.exe"
    RecordingChecks "$env:TEMP\rustshot.exe"
    Start-Process "$env:TEMP\rustshot.exe"
    Check 'portable: daemon started for visual tray check' (WaitFor { [W]::TrayWindow() })
}
Add-Content $log 'DONE'
