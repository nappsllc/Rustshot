# 0.5.0 checks, dot-sourced by test.ps1 and run on the portable exe:
# languages (the release's lang-<tag>.txt files listed by `rustshot data`;
# the app starts in English; Settings › General's Language list opens and
# picking a language with no network says it couldn't download it and
# stays in English; `language = "de"` with no network starts in English;
# the repo's de.txt put in the cache by hand is verified and switches the
# window to German; Arabic opens too), and the overlay's new tools on a
# window of this sandbox: the ruler (U, a drag), Read QR code (Q in a
# one-shot capture over a QR picture prints its link; with none, "No QR
# code found") and Scrolling capture (Shift+S on a long text box: the box
# scrolls and the stitched picture is taller than the selection).
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped. Real key and
# mouse input goes to the sandbox's own desktop; Settings gets posted keys.
# Inputs: <T>\data\lang\*.txt (run.ps1 copies the repo's lang\), and
# optionally <T>\data\qr.png (run.ps1 -QrPng: a picture with a QR code of
# an https link, e.g. the qr_make_link_light_1x.png preview).

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class V50 {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string c, string t);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, IntPtr e);
  // The visible top-level window of class cls owned by process pid (0: any).
  public static IntPtr Find(string cls, uint pid) {
    IntPtr r = IntPtr.Zero;
    EnumWindows((h, l) => { var s = new StringBuilder(64); GetClassName(h, s, 64); uint p; GetWindowThreadProcessId(h, out p);
      if (s.ToString() == cls && IsWindowVisible(h) && (pid == 0 || p == pid)) { r = h; return false; } return true; }, IntPtr.Zero);
    return r;
  }
  // The same by class name prefix (WinForms classes have a variable suffix).
  public static IntPtr FindPrefix(string prefix, uint pid) {
    IntPtr r = IntPtr.Zero;
    EnumWindows((h, l) => { var s = new StringBuilder(128); GetClassName(h, s, 128); uint p; GetWindowThreadProcessId(h, out p);
      if (s.ToString().StartsWith(prefix) && IsWindowVisible(h) && p == pid) { r = h; return false; } return true; }, IntPtr.Zero);
    return r;
  }
  public static string Title(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
  // FNV-1a of a control's name: the id the Settings window reports (crate::ui::id_of).
  public static string Id(string name) { unchecked { ulong h = 0xcbf29ce484222325UL; foreach (byte b in Encoding.UTF8.GetBytes(name)) { h ^= b; h *= 0x100000001b3UL; } return h.ToString(); } }
  // A real left-button drag on this desktop, in steps.
  public static void Drag(int x0, int y0, int x1, int y1) {
    SetCursorPos(x0, y0); System.Threading.Thread.Sleep(150);
    mouse_event(2, 0, 0, 0, IntPtr.Zero); System.Threading.Thread.Sleep(100);
    for (int i = 1; i <= 20; i++) { SetCursorPos(x0 + (x1 - x0) * i / 20, y0 + (y1 - y0) * i / 20); System.Threading.Thread.Sleep(25); }
    System.Threading.Thread.Sleep(150); mouse_event(4, 0, 0, 0, IntPtr.Zero);
  }
}
"@

# A picture of window h as it is on screen.
function WinShot($h, $name) {
    $r = New-Object V50+RECT
    [void][V50]::GetWindowRect($h, [ref]$r)
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    if ($w -le 0 -or $hh -le 0) { return $null }
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
    $g.Dispose()
    $p = "$out\$name.png"; $bmp.Save($p); $bmp.Dispose()
    $p
}

# Pixels in a picture that are red text (the error colour of either theme).
function RedPixels($path) {
    $b = New-Object System.Drawing.Bitmap $path
    try {
        $n = 0
        for ($y = 0; $y -lt $b.Height; $y += 1) { for ($x = 0; $x -lt $b.Width; $x += 2) {
            $c = $b.GetPixel($x, $y)
            if ($c.R -ge 190 -and $c.R - $c.G -ge 70 -and $c.R - $c.B -ge 70) { $n++ }
        } }
        $n
    } finally { $b.Dispose() }
}

function FocusId($file) { if (Test-Path $file) { (Get-Content $file -Raw -ErrorAction SilentlyContinue) } else { $null } }

# Tab (posted) until control `name` reports the focus; true when reached.
function TabTo($h, $file, $name, $max = 40) {
    $want = [V50]::Id($name)
    for ($i = 0; $i -lt $max; $i++) {
        if ((FocusId $file) -eq $want) { return $true }
        $was = FocusId $file
        [V3]::PostKey($h, 0x09)
        $null = WaitFor { (FocusId $file) -ne $was } 3
    }
    (FocusId $file) -eq $want
}

# `rustshot settings` in its own process with these variables; its window (or zero).
function OpenSettings($exe, $vars, $tag) {
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'settings' -PassThru -RedirectStandardError "$out\v050-$tag-stderr.txt" }; $null = $p.Handle
    $null = WaitFor { [V50]::Find('rustshot_window', [uint32]$p.Id) -ne [IntPtr]::Zero } 10
    Start-Sleep 2
    @{ p = $p; h = [V50]::Find('rustshot_window', [uint32]$p.Id) }
}

function CloseSettings($s) {
    if ($s.h -ne [IntPtr]::Zero) { [void][W]::PostMessage($s.h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    $null = WaitFor { $s.p.HasExited } 5
    StopPid $s.p
}

function Checks050($exe) {
    $root = "$env:TEMP\v050-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $focus = "$root\focus.txt"
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-050'; RUSTSHOT_TEST_FOCUS_FILE = $focus }
    $base = "history = false`noverlay_hints = false`nmagnifier = false`nrecord_hotkey = `"`"`ncapture_full_hotkey = `"`"`ncapture_window_hotkey = `"`"`nrepeat_hotkey = `"`"`ncapture_text_hotkey = `"`"`n"
    # (A GUI-subsystem exe: its --version line does not reach a pipe here; the version resource says the same.)
    $ver = ((Get-Item $exe).VersionInfo.ProductVersion -replace '^(\d+\.\d+\.\d+).*$', '$1')
    $cache = "$local\rustshot-data"
    $repoLang = "$T\data\lang"

    # ------------------------------------------------ `rustshot data`: every language file of the release, none here
    V4Config $appdata $save $base
    $o = WithEnv $vars { & $exe data 2>&1 | Out-String }
    $langLines = @($o -split "`r?`n" | Where-Object { $_ -match '^lang-[A-Za-z0-9-]+\.txt: not here; download https://github\.com/nappsllc/rustshot/releases/download/v' + [regex]::Escape($ver) + '/lang-[A-Za-z0-9-]+\.txt and save it as ' })
    $repoCount = @(Get-ChildItem $repoLang -Filter *.txt -ErrorAction SilentlyContinue).Count
    Check "v050 data: ``rustshot data`` lists every language as lang-<tag>.txt of the v$ver release ($repoCount files in the repo)" ($langLines.Count -ge 35 -and ($repoCount -eq 0 -or $langLines.Count -eq $repoCount)) "listed=$($langLines.Count)"
    Add-Content $log "      data (first lines): $((($o -split "`r?`n") | Select-Object -First 3) -join ' | ')"

    # ------------------------------------------------ English at start (system language: this sandbox's)
    $sys = (Get-WinUserLanguageList)[0].LanguageTag
    $s = OpenSettings $exe $vars 'en'
    $title = if ($s.h -ne [IntPtr]::Zero) { [V50]::Title($s.h) } else { '' }
    Check "v050 language: with no language set (system $sys) Settings opens in English" ($title -eq "Rustshot $([char]0x2014) Settings") "title=[$title]"
    $enShot = if ($s.h -ne [IntPtr]::Zero) { WinShot $s.h 'v050-settings-en' }
    $redEn = if ($enShot) { RedPixels $enShot } else { -1 }

    # ------------------------------------------------ Settings › General: the Language list opens; a pick offline fails cleanly
    $reached = $s.h -ne [IntPtr]::Zero -and (TabTo $s.h $focus 'language')
    Check 'v050 language: Settings › General has the Language list (Tab reaches the "language" control)' $reached "focus=$(FocusId $focus) want=$([V50]::Id('language'))"
    if ($reached) {
        [V3]::PostKey($s.h, 0x0D); Start-Sleep 1   # Enter: open the list
        $open = WinShot $s.h 'v050-settings-language-list'
        $d = if ($open -and $enShot) { DiffStats $enShot $open } else { @{ n = -1 } }
        Check 'v050 language: Enter opens the language list (out\v050-settings-language-list.png)' ($d.n -gt 500) "changed px (every 2nd)=$($d.n)"
        [V3]::PostKey($s.h, 0x1B); Start-Sleep 1   # Esc: close it
        # Down on the closed list picks the next language (System default → the first one): its file is fetched, and there is no network.
        [V3]::PostKey($s.h, 0x28); [V3]::PostKey($s.h, 0x28)
        $failed = WaitFor { $sh = WinShot $s.h 'v050-settings-pick-offline'; $sh -and (RedPixels $sh) -gt $redEn + 60 } 40
        $red = RedPixels "$out\v050-settings-pick-offline.png"
        Check 'v050 language: picking a language offline says it couldn''t download it, in red, with Retry (out\v050-settings-pick-offline.png)' $failed "red px=$red (English start: $redEn)"
        $title = [V50]::Title($s.h)
        Check 'v050 language: ... and the window stays up and in English' (-not $s.p.HasExited -and $title -eq "Rustshot $([char]0x2014) Settings") "title=[$title] exited=$($s.p.HasExited)"
        $parts = @(Get-ChildItem $cache -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*.part' -or $_.DirectoryName -like '*\lang' })
        Check 'v050 language: no language file and no partial file after the failed download' ($parts.Count -eq 0) "files=$(($parts | ForEach-Object Name) -join ',')"
    }
    CloseSettings $s

    # ------------------------------------------------ language = "de" with no network: English, cleanly
    V4Config $appdata $save ($base + "language = `"de`"`n")
    $s = OpenSettings $exe $vars 'de-offline'
    $title = if ($s.h -ne [IntPtr]::Zero) { [V50]::Title($s.h) } else { '' }
    $sh = if ($s.h -ne [IntPtr]::Zero) { WinShot $s.h 'v050-settings-de-offline' }
    Start-Sleep 3
    Check 'v050 language: language = "de" with no network starts in English and stays up' ($title -eq "Rustshot $([char]0x2014) Settings" -and -not $s.p.HasExited) "title=[$title] exited=$($s.p.HasExited)"
    CloseSettings $s
    $o = WithEnv $vars { & $exe data --fetch 2>&1 | Out-String }; $rc = $LASTEXITCODE
    $deLine = ($o -split "`r?`n" | Where-Object { $_ -like 'lang-de.txt:*' }) -join ''
    Check 'v050 language: offline `data --fetch` says lang-de.txt couldn''t download (exit 1), no partial file' ($rc -eq 1 -and $deLine -match "couldn't download" -and @(Get-ChildItem $cache -Recurse -Filter *.part -ErrorAction SilentlyContinue).Count -eq 0) "exit=$rc $deLine"

    # ------------------------------------------------ the repo's de.txt in the cache by hand: verified, and Settings is German
    if (Test-Path "$repoLang\de.txt") {
        New-Item -ItemType Directory -Force "$cache\lang" | Out-Null
        Copy-Item "$repoLang\de.txt" "$cache\lang\de.txt"
        $o = WithEnv $vars { & $exe data 2>&1 | Out-String }
        $deLine = ($o -split "`r?`n" | Where-Object { $_ -like 'lang-de.txt:*' }) -join ''
        Check 'v050 language: the repo''s de.txt put in the cache is "SHA-256 verified" (the bytes the release publishes)' ($deLine -match 'SHA-256 verified') $deLine
        $s = OpenSettings $exe $vars 'de'
        $title = if ($s.h -ne [IntPtr]::Zero) { [V50]::Title($s.h) } else { '' }
        if ($s.h -ne [IntPtr]::Zero) { $null = WinShot $s.h 'v050-settings-de' }
        Check 'v050 language: with it, language = "de" opens Settings in German (out\v050-settings-de.png)' ($title -eq "Rustshot $([char]0x2014) Einstellungen") "title=[$title]"
        CloseSettings $s
        # Right to left: Arabic, its file put there the same way.
        if (Test-Path "$repoLang\ar.txt") {
            Copy-Item "$repoLang\ar.txt" "$cache\lang\ar.txt"
            V4Config $appdata $save ($base + "language = `"ar`"`n")
            $s = OpenSettings $exe $vars 'ar'
            $title = if ($s.h -ne [IntPtr]::Zero) { [V50]::Title($s.h) } else { '' }
            if ($s.h -ne [IntPtr]::Zero) { $null = WinShot $s.h 'v050-settings-ar' }
            $want = ((Get-Content "$repoLang\ar.txt" -Encoding UTF8 | Where-Object { $_ -like 'settings.window_title =*' }) -replace '^settings\.window_title = ', '').Trim()
            Check 'v050 language: Arabic (right to left) opens Settings in Arabic (out\v050-settings-ar.png, eye check: mirrored)' ($title -eq $want -and -not $s.p.HasExited) "title=[$title]"
            CloseSettings $s
        }
    } else {
        Add-Content $log 'SKIP  v050 language: hand-placed de.txt / ar.txt (no <T>\data\lang)'
    }
    Remove-Item -Recurse -Force "$cache\lang" -ErrorAction SilentlyContinue
    V4Config $appdata $save $base

    # ------------------------------------------------ ruler (U): a drag in the overlay draws a measured line
    $plain = "$save\v050-plain.png"
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '600x400+100+100', '--path', $plain -PassThru -Wait }
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', '600x400+100+100' -PassThru -RedirectStandardError "$out\v050-ruler-stderr.txt" }; $null = $g.Handle
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    if ($ovUp) {
        KeyPress 0x12
        [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
        Shot 'v050-ruler-before'
        KeyPress 0x55; Start-Sleep -Milliseconds 400       # U: ruler
        [V50]::Drag(200, 300, 600, 300); Start-Sleep 1
        Shot 'v050-ruler'
        $d = DiffStats "$out\v050-ruler-before.png" "$out\v050-ruler.png"
        Check 'v050 ruler: U and a drag draw a measured line in the overlay (out\v050-ruler.png)' ($d.n -gt 100 -and -not $g.HasExited) "changed px (every 2nd)=$($d.n)"
        KeyPress 0x1B; Start-Sleep -Milliseconds 300; KeyPress 0x1B
    } else {
        Check 'v050 ruler: the overlay opens' $false
    }
    $null = WaitFor { $g.HasExited } 5
    StopPid $g

    # ------------------------------------------------ Read QR code (Q): a code on screen gives a toast with buttons
    if (Test-Path "$T\data\qr.png") {
        $form = "$root\qrform.ps1"
        Set-Content $form @"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
`$img = [System.Drawing.Image]::FromFile('$T\data\qr.png')
`$f = New-Object System.Windows.Forms.Form
`$f.FormBorderStyle = 'None'; `$f.StartPosition = 'Manual'; `$f.Location = New-Object System.Drawing.Point(100, 100)
`$f.ClientSize = `$img.Size; `$f.TopMost = `$true; `$f.BackColor = 'White'
`$pb = New-Object System.Windows.Forms.PictureBox; `$pb.Dock = 'Fill'; `$pb.Image = `$img
`$f.Controls.Add(`$pb)
[System.Windows.Forms.Application]::Run(`$f)
"@
        $fp = Start-Process powershell.exe -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $form -PassThru
        Start-Sleep 4
        $sz = (New-Object System.Drawing.Bitmap "$T\data\qr.png"); $qw = $sz.Width; $qh = $sz.Height; $sz.Dispose()
        $res = @{}
        # A one-shot capture (`gui`) prints what it read: the code's text on stdout, "No QR code found" on stderr.
        foreach ($case in @('code', 'none')) {
            if ($case -eq 'none') { StopPid $fp; Start-Sleep 1 }
            $so = "$out\v050-qr-$case-stdout.txt"; $se = "$out\v050-qr-$case-stderr.txt"
            $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', "${qw}x${qh}+100+100" -PassThru -RedirectStandardOutput $so -RedirectStandardError $se }; $null = $g.Handle
            $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
            Start-Sleep 1
            if ($ovUp) {
                KeyPress 0x12
                [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
                KeyPress 0x51                                     # Q: Read QR code
            }
            $done = WaitFor { $g.HasExited } 20
            StopPid $g
            $res[$case] = @{ ov = $ovUp; done = $done; exit = $(if ($done) { $g.ExitCode }); so = ((Get-Content $so -Raw -ErrorAction SilentlyContinue) + '').Trim(); se = ((Get-Content $se -Raw -ErrorAction SilentlyContinue) + '').Trim() }
        }
        Check 'v050 qr: Q on the QR window reads the code (its link on stdout)' ($res.code.so -eq 'https://github.com/rustshot/rustshot') "overlay=$($res.code.ov) exited=$($res.code.done) exit=$($res.code.exit) stdout=[$($res.code.so)] stderr=[$($res.code.se)]"
        Check 'v050 qr: Q with no code on screen says "No QR code found"' ($res.none.so -eq '' -and $res.none.se -match 'No QR code found') "overlay=$($res.none.ov) exit=$($res.none.exit) stdout=[$($res.none.so)] stderr=[$($res.none.se)]"
        StopPid $fp
    } else {
        Add-Content $log 'SKIP  v050 qr: no <T>\data\qr.png (run.ps1 -QrPng)'
    }

    # ------------------------------------------------ Scrolling capture (Shift+S) on a long text box
    $form = "$root\scrollform.ps1"
    Set-Content $form @'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$f = New-Object System.Windows.Forms.Form
$f.FormBorderStyle = 'None'; $f.StartPosition = 'Manual'; $f.Location = New-Object System.Drawing.Point(100, 100)
$f.ClientSize = New-Object System.Drawing.Size(560, 420); $f.TopMost = $true
$t = New-Object System.Windows.Forms.TextBox
$t.Multiline = $true; $t.ScrollBars = 'Vertical'; $t.Dock = 'Fill'; $t.ReadOnly = $true
$t.Font = New-Object System.Drawing.Font('Consolas', 14)
$t.Text = ((1..160) | ForEach-Object { "line {0:000}  the quick brown fox jumps over the lazy dog {1}" -f $_, ('#' * ($_ % 23)) }) -join "`r`n"
$t.Name = 'longtext'
$f.Controls.Add($t)
[System.Windows.Forms.Application]::Run($f)
'@
    $fp = Start-Process powershell.exe -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $form -PassThru
    Start-Sleep 4
    # EM_GETFIRSTVISIBLELINE of the form's text box (its only child); -1 when not found.
    $firstLine = {
        param($pid0)
        $form = [V50]::FindPrefix('WindowsForms10.Window', [uint32]$pid0)
        if ($form -eq [IntPtr]::Zero) { return -1 }
        $c = [V50]::FindWindowEx($form, [IntPtr]::Zero, [NullString]::Value, [NullString]::Value)
        if ($c -eq [IntPtr]::Zero) { return -1 }
        [int][V50]::SendMessage($c, 0x00CE, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    # The text box at its top (WM_VSCROLL SB_TOP), then where it is.
    $tb = [V50]::FindPrefix('WindowsForms10.Window', [uint32]$fp.Id)
    if ($tb -ne [IntPtr]::Zero) { $tb = [V50]::FindWindowEx($tb, [IntPtr]::Zero, [NullString]::Value, [NullString]::Value) }
    if ($tb -ne [IntPtr]::Zero) { [void][V50]::SendMessage($tb, 0x0115, [IntPtr]6, [IntPtr]::Zero); Start-Sleep -Milliseconds 500 }
    $line0 = & $firstLine $fp.Id
    $before = @(Get-ChildItem $save -Recurse -Filter *.png | Where-Object Name -ne 'v050-plain.png').Count
    $g = WithEnv $vars { Start-Process $exe -ArgumentList 'gui', '--region', '480x300+120+160' -PassThru -RedirectStandardError "$out\v050-scroll-stderr.txt" }; $null = $g.Handle
    $ovUp = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    if ($ovUp) {
        KeyPress 0x12
        [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
        [W]::Chord(0x10, 0x53)                                 # Shift+S: Scrolling capture
        Start-Sleep 2
        Shot 'v050-scroll-running'
    }
    # With the history off the stitched picture is saved.
    $saved = WaitFor { @(Get-ChildItem $save -Recurse -Filter *.png | Where-Object Name -ne 'v050-plain.png').Count -gt $before } 90
    $line1 = & $firstLine $fp.Id
    $f = Get-ChildItem $save -Recurse -Filter *.png | Where-Object Name -ne 'v050-plain.png' | Sort-Object LastWriteTime | Select-Object -Last 1
    $sz = if ($f) { PngSize $f.FullName } else { '' }
    $hgt = if ($sz) { [int]($sz -split 'x')[1] } else { 0 }
    if ($f) { Copy-Item $f.FullName "$out\v050-scroll.png" }
    Check 'v050 scroll: Shift+S scrolls the text box under the selection' ($line0 -ge 0 -and $line1 -gt $line0) "first visible line $line0 -> $line1"
    Check 'v050 scroll: the stitched picture is saved, 480 wide and taller than the 300 px selection (out\v050-scroll.png)' ($saved -and $sz -match '^480x' -and $hgt -gt 600) "size=$sz"
    $null = WaitFor { $g.HasExited } 10
    StopPid $g
    StopPid $fp
    $err = Get-Content "$out\v050-*-stderr.txt" -ErrorAction SilentlyContinue
    if ($err) { Add-Content $log ("      stderr: " + (($err | Select-Object -First 10) -join ' | ')) }
}
