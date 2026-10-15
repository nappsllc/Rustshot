# 0.6.3 checks, dot-sourced by test.ps1 and run on the portable exe:
# - Icons: the exe's 16 px icon as the shell extracts it (ExtractIconEx's
#   small entry, a DIB) is the new pixel-aligned frame
#   (packaging/icons/small/rustshot-16.png, copied by run.ps1 to
#   data\icon16-new.png) and not 0.6.2's (data\icon16-062.png, when run.ps1
#   could read it from git); the Settings window's small title-bar icon is
#   the same frame when it is 16 x 16.
# - Window titles: `rustshot settings` opens "Rustshot — Settings" and
#   `rustshot history` "Rustshot — History" (GetWindowText of the
#   rustshot_window owned by the PID started here); E on History's one
#   entry opens the Edit window titled "<file> — Rustshot", and a stroke
#   drawn in it makes it "*<file> — Rustshot".
# - Windows chrome: on screen, the Settings window's caption is Rustshot's
#   theme surface and its 1 px border (the frame's outer pixels,
#   DWMWA_EXTENDED_FRAME_BOUNDS) is the caption colour. (The colours set
#   with DWMWA_CAPTION/TEXT/BORDER_COLOR can't be read back:
#   DwmGetWindowAttribute answers E_INVALIDARG for them.)
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped. Input goes to
# the sandbox's own desktop only.

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class V63 {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, uint a, out RECT v, uint n);
  [DllImport("user32.dll")] public static extern IntPtr GetClassLongPtr(IntPtr h, int i);
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint n);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr h);
  // The visible rustshot_window windows of process pid: "hwnd<TAB>title" each.
  public static List<string> Windows(uint pid) {
    var r = new List<string>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      var c = new StringBuilder(64); GetClassName(h, c, 64);
      if (p == pid && c.ToString() == "rustshot_window" && IsWindowVisible(h)) {
        var t = new StringBuilder(512); GetWindowText(h, t, 512);
        r.Add(h.ToInt64() + "\t" + t.ToString());
      }
      return true; }, IntPtr.Zero);
    return r;
  }
  public static string Title(IntPtr h) { var t = new StringBuilder(512); GetWindowText(h, t, 512); return t.ToString(); }
}
"@

# The window of $p whose title matches $title exactly (or, with -Like, the pattern): its handle, or Zero.
function V63Window($p, $title, [switch]$Like) {
    foreach ($w in [V63]::Windows([uint32]$p.Id)) {
        $h, $t = $w -split "`t", 2
        if (($Like -and $t -like $title) -or (-not $Like -and $t -ceq $title)) { return [IntPtr][int64]$h }
    }
    [IntPtr]::Zero
}
function V63Titles($p) { (@([V63]::Windows([uint32]$p.Id)) | ForEach-Object { ($_ -split "`t", 2)[1] }) -join ' | ' }

# Pixels of two 32-bit images that differ by more than 24 in a channel (alpha
# weighted: fully transparent pixels compare equal); -1 when sizes differ.
function V63Diff([System.Drawing.Bitmap]$a, [System.Drawing.Bitmap]$b) {
    if ($a.Width -ne $b.Width -or $a.Height -ne $b.Height) { return -1 }
    $n = 0
    for ($y = 0; $y -lt $a.Height; $y++) { for ($x = 0; $x -lt $a.Width; $x++) {
        $p = $a.GetPixel($x, $y); $q = $b.GetPixel($x, $y)
        if ($p.A -eq 0 -and $q.A -eq 0) { continue }
        $d = [Math]::Max([Math]::Max([Math]::Abs($p.A - $q.A), [Math]::Abs($p.R * $p.A - $q.R * $q.A) / 255),
                         [Math]::Max([Math]::Abs($p.G * $p.A - $q.G * $q.A) / 255, [Math]::Abs($p.B * $p.A - $q.B * $q.A) / 255))
        if ($d -gt 24) { $n++ }
    } }
    $n
}

# An icon handle as a 32-bit bitmap (the caller disposes it).
function V63IconBmp([IntPtr]$h) {
    $ico = [System.Drawing.Icon]::FromHandle($h)
    try { $ico.ToBitmap() } finally { $ico.Dispose() }
}

function V63Drag($x0, $y0, $x1, $y1) {
    [void][V63]::SetCursorPos($x0, $y0); Start-Sleep -Milliseconds 150
    [V63]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)   # left down
    for ($i = 1; $i -le 12; $i++) {
        [void][V63]::SetCursorPos([int]($x0 + ($x1 - $x0) * $i / 12), [int]($y0 + ($y1 - $y0) * $i / 12)); Start-Sleep -Milliseconds 25
    }
    [V63]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)   # left up
}

function Checks063($exe) {
    $root = "$env:TEMP\v063-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-063' }
    V4Config $appdata $save "history = true`nhistory_hotkey = `"`"`nrecord_hotkey = `"`"`ncapture_full_hotkey = `"`"`ncapture_window_hotkey = `"`"`nrepeat_hotkey = `"`"`ncapture_text_hotkey = `"`"`n"
    $em = [char]0x2014

    # ------------------------------------------------ the exe's 16 px icon: the new pixel-aligned frame
    $new16 = "$T\data\icon16-new.png"; $old16 = "$T\data\icon16-062.png"
    $large = New-Object IntPtr[] 1; $small = New-Object IntPtr[] 1
    [void][V63]::ExtractIconEx($exe, 0, $large, $small, 1)
    if ($small[0] -ne [IntPtr]::Zero -and (Test-Path $new16)) {
        $got = V63IconBmp $small[0]
        $got.Save("$out\v063-icon-16.png")
        $ref = New-Object System.Drawing.Bitmap $new16
        $dn = V63Diff $got $ref
        $ref.Dispose()
        Check 'v063 icon: the exe''s 16 px icon (ExtractIconEx small) is the pixel-aligned frame small/rustshot-16.png' ($dn -eq 0) "differing px=$dn size=$($got.Width)x$($got.Height) (out\v063-icon-16.png)"
        if (Test-Path $old16) {
            $old = New-Object System.Drawing.Bitmap $old16
            $do = V63Diff $got $old
            $old.Dispose()
            Check 'v063 icon: ... and not 0.6.2''s 16 px icon' ($do -ge 20) "px differing from 0.6.2's=$do"
        } else {
            Add-Content $log "SKIP  v063 icon: no 0.6.2 16 px icon to compare with (run.ps1 reads it from git: v0.6.2:packaging/icons/rustshot-16.png)"
        }
        $got.Dispose()
    } else {
        Check 'v063 icon: ExtractIconEx returns a 16 px icon and data\icon16-new.png is there' $false "icon=$($small[0]) ref=$(Test-Path $new16)"
    }
    foreach ($h in $small[0], $large[0]) { if ($h -ne [IntPtr]::Zero) { [void][V63]::DestroyIcon($h) } }

    # ------------------------------------------------ Settings: its title, its small icon, its DWM colours
    $s = WithEnv $vars { Start-Process $exe -ArgumentList 'settings' -PassThru -RedirectStandardError "$out\v063-settings-stderr.txt" }; $null = $s.Handle
    $want = "Rustshot $em Settings"
    $up = WaitFor { (V63Window $s $want) -ne [IntPtr]::Zero } 10
    $sw = V63Window $s $want
    Check "v063 titles: ``rustshot settings`` opens a window titled ""$want""" $up "titles: $(V63Titles $s)"
    if ($up) {
        Start-Sleep 2
        # The class's small icon (GCLP_HICONSM; WM_GETICON has none: no WM_SETICON).
        $hi = [V63]::GetClassLongPtr($sw, -34)
        if ($hi -eq [IntPtr]::Zero) { $hi = [V63]::SendMessage($sw, 0x007F, [IntPtr]2, [IntPtr]::Zero) }
        if ($hi -ne [IntPtr]::Zero -and (Test-Path $new16)) {
            $b = V63IconBmp $hi
            $b.Save("$out\v063-settings-small-icon.png")
            if ($b.Width -eq 16 -and $b.Height -eq 16) {
                $ref = New-Object System.Drawing.Bitmap $new16
                $dn = V63Diff $b $ref
                $ref.Dispose()
                Check 'v063 icon: the Settings window''s small (title bar) icon is the new 16 px frame' ($dn -eq 0) "differing px=$dn (out\v063-settings-small-icon.png)"
            } else {
                Add-Content $log "SKIP  v063 icon: the Settings window's small icon is $($b.Width)x$($b.Height), not 16 px (DPI); saved as out\v063-settings-small-icon.png"
            }
            $b.Dispose()
        } else {
            Add-Content $log "SKIP  v063 icon: the Settings window has no small icon handle (GCLP_HICONSM, WM_GETICON)"
        }
        # DWMWA_EXTENDED_FRAME_BOUNDS (9): the frame as drawn, its outer pixels the border.
        $fb = New-Object V63+RECT
        $hr = [V63]::DwmGetWindowAttribute($sw, 9, [ref]$fb, 16)
        if ($hr -eq 0) {
            [void][V63]::SetForegroundWindow($sw); Start-Sleep -Milliseconds 500
            $w = $fb.R - $fb.L; $hh = $fb.B - $fb.T
            $bmp = New-Object System.Drawing.Bitmap $w, $hh
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            $g.CopyFromScreen($fb.L, $fb.T, 0, 0, $bmp.Size)
            $bmp.Save("$out\v063-settings-frame.png")
            $cap = $bmp.GetPixel([int]($w * 0.55), 8)
            $edges = @($bmp.GetPixel(0, [int]($hh / 2)), $bmp.GetPixel($w - 1, [int]($hh / 2)), $bmp.GetPixel([int]($w / 2), 0))
            $g.Dispose(); $bmp.Dispose()
            $near = { param($a, $b) [Math]::Abs($a.R - $b.R) -le 6 -and [Math]::Abs($a.G - $b.G) -le 6 -and [Math]::Abs($a.B - $b.B) -le 6 }
            # Rustshot's: the dark theme's caption 1B1C20, the light one's FAFAFC.
            $known = @([System.Drawing.Color]::FromArgb(0x1B, 0x1C, 0x20), [System.Drawing.Color]::FromArgb(0xFA, 0xFA, 0xFC))
            $isOurs = @($known | Where-Object { & $near $_ $cap }).Count -gt 0
            $hex = { param($c) '{0:X2}{1:X2}{2:X2}' -f $c.R, $c.G, $c.B }
            $bad = @($edges | Where-Object { -not (& $near $_ $cap) })
            Check 'v063 chrome: the caption is Rustshot''s theme surface and the 1 px border is the caption colour (on screen, out\v063-settings-frame.png)' ($isOurs -and $bad.Count -eq 0) "caption=$(& $hex $cap) border(left,right,top)=$(($edges | ForEach-Object { & $hex $_ }) -join ',') frame=${w}x$hh"
        } else {
            Add-Content $log ("SKIP  v063 chrome: no DWMWA_EXTENDED_FRAME_BOUNDS (hr=0x{0:X8})" -f $hr)
        }
        Shot 'v063-settings'
    }
    StopPid $s

    # ------------------------------------------------ History's title, then Edit's ("<file> — Rustshot", "*" when edited)
    $png = "$save\v063-shot.png"
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '640x400+100+100', '--path', $png -PassThru -Wait }
    Check 'v063 titles: a History entry from `full --path v063-shot.png`' ($p.ExitCode -eq 0 -and (Test-Path $png)) "exit=$($p.ExitCode)"
    $h = WithEnv $vars { Start-Process $exe -ArgumentList 'history' -PassThru -RedirectStandardError "$out\v063-history-stderr.txt" }; $null = $h.Handle
    $want = "Rustshot $em History"
    $up = WaitFor { (V63Window $h $want) -ne [IntPtr]::Zero } 10
    $hw = V63Window $h $want
    Check "v063 titles: ``rustshot history`` opens a window titled ""$want""" $up "titles: $(V63Titles $h)"
    if ($up) {
        Start-Sleep 2
        [void][V63]::SetForegroundWindow($hw); Start-Sleep -Milliseconds 400
        [W]::Chord(0x11, 0x41); Start-Sleep -Milliseconds 400                                       # Ctrl+A: the one entry
        [V63]::keybd_event(0x45, 0, 0, [IntPtr]::Zero); [V63]::keybd_event(0x45, 0, 2, [IntPtr]::Zero)   # E: Edit
        $want = "v063-shot.png $em Rustshot"
        $eup = WaitFor { (V63Window $h $want) -ne [IntPtr]::Zero } 10
        $ew = V63Window $h $want
        Check "v063 titles: E in History opens the Edit window titled ""$want""" $eup "titles: $(V63Titles $h)"
        if ($eup) {
            Start-Sleep 2
            [void][V63]::SetForegroundWindow($ew); Start-Sleep -Milliseconds 400
            $r = New-Object V63+RECT
            [void][V63]::GetWindowRect($ew, [ref]$r)
            $cx = [int](($r.L + $r.R) / 2); $cy = [int](($r.T + $r.B) / 2)
            # R: the rectangle tool (the window opens on Select), then a drag over the picture.
            [V63]::keybd_event(0x52, 0, 0, [IntPtr]::Zero); [V63]::keybd_event(0x52, 0, 2, [IntPtr]::Zero); Start-Sleep -Milliseconds 300
            V63Drag ($cx - 80) ($cy - 40) ($cx + 80) ($cy + 40)
            $starred = "*$want"
            $ed = WaitFor { (V63Window $h $starred) -ne [IntPtr]::Zero } 5
            Shot 'v063-edit'
            Check "v063 titles: a stroke in the Edit window marks it unsaved (""$starred"")" $ed "titles: $(V63Titles $h)"
        }
    }
    StopPid $h
}
