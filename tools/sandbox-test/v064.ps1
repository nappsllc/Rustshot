# 0.6.4 checks, dot-sourced by test.ps1 and run on the portable exe:
# - Windows hidden from screenshots: a window of this PowerShell process
#   with SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE), topmost at a
#   known place and filled magenta, is in a full-screen `rustshot full
#   --path` (theme dark) as the placeholder (the dark theme's flat panel
#   1F2126 / 23252B near its edges, the label's ink in its middle); with
#   `mark_hidden_windows = false` the same capture shows what is behind
#   it (no panel, no magenta). SKIP when SetWindowDisplayAffinity fails.
# - The Settings window's client area is 680 x 552 logical px
#   (GetClientRect on a per-monitor-aware thread, over GetDpiForWindow).
# - Settings' Shortcuts sections: from the rail (Tab to it, End, then Up a
#   section at a time; posted keys, RUSTSHOT_TEST_FOCUS_FILE), Tab reaches
#   a row of each one on its own page: Edit window (key_zoom_in), Overlay
#   actions (key_accept), Overlay editing (key_undo), Overlay tools
#   (key_tool_text), Recording (key_rec_mute) and Global (global3); then
#   Capture's "Mark windows hidden from screenshots" switch
#   (mark_hidden_windows). A screenshot of each page (out\v064-keys-*.png).
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped. Settings gets
# posted keys; nothing else is typed or clicked.

Add-Type @"
using System; using System.Runtime.InteropServices;
public static class V64 {
  [DllImport("user32.dll")] public static extern bool SetWindowDisplayAffinity(IntPtr h, uint a);
  [DllImport("user32.dll")] public static extern bool GetWindowDisplayAffinity(IntPtr h, out uint a);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  // The client size in physical px (asked on a per-monitor-aware v2 thread) and the window's DPI.
  public static int[] Client(IntPtr h) {
    IntPtr old = SetThreadDpiAwarenessContext((IntPtr)(-4));
    try { RECT r; GetClientRect(h, out r); return new int[] { r.R - r.L, r.B - r.T, (int)GetDpiForWindow(h) }; }
    finally { if (old != IntPtr.Zero) SetThreadDpiAwarenessContext(old); }
  }
}
"@

# Whether colour c is the dark theme's placeholder panel or its stripe (hidden.rs Style::of).
function V64Panel($c) {
    foreach ($p in @(@(0x1F, 0x21, 0x26), @(0x23, 0x25, 0x2B))) {
        if ([Math]::Abs($c.R - $p[0]) -le 3 -and [Math]::Abs($c.G - $p[1]) -le 3 -and [Math]::Abs($c.B - $p[2]) -le 3) { return $true }
    }
    $false
}
function V64Magenta($c) { $c.R -ge 220 -and $c.G -le 40 -and $c.B -ge 220 }

# Settings: Tab (posted) to the rail, then key vk on it; true when the rail had the focus.
function V64Rail($h, $focus, $vk) {
    if (-not (TabTo $h $focus 'rail' 80)) { return $false }
    [V3]::PostKey($h, $vk)
    Start-Sleep -Milliseconds 400
    $true
}

function Checks064($exe) {
    $root = "$env:TEMP\v064-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $focus = "$root\focus.txt"
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-064'; RUSTSHOT_TEST_FOCUS_FILE = $focus }
    $base = "theme = `"dark`"`nhistory = false`nhistory_hotkey = `"`"`nrecord_hotkey = `"`"`ncapture_full_hotkey = `"`"`ncapture_window_hotkey = `"`"`nrepeat_hotkey = `"`"`ncapture_text_hotkey = `"`"`n"

    # ------------------------------------------------ a window hidden from capture, in a full capture from the CLI
    $f = New-Object System.Windows.Forms.Form
    $f.FormBorderStyle = 'None'; $f.ShowInTaskbar = $false; $f.TopMost = $true
    $f.StartPosition = 'Manual'; $f.Location = New-Object System.Drawing.Point(240, 200); $f.Size = New-Object System.Drawing.Size(480, 300)
    $f.BackColor = [System.Drawing.Color]::FromArgb(255, 0, 255)
    $f.Show(); [System.Windows.Forms.Application]::DoEvents()
    $hf = $f.Handle
    $set = [V64]::SetWindowDisplayAffinity($hf, 0x11)   # WDA_EXCLUDEFROMCAPTURE
    $aff = [uint32]0; [void][V64]::GetWindowDisplayAffinity($hf, [ref]$aff)
    for ($i = 0; $i -lt 20; $i++) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 25 }
    if (-not $set -or $aff -ne 0x11) {
        Add-Content $log ("SKIP  v064 hidden: SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE) failed here (ok=$set affinity=0x{0:X})" -f $aff)
    } else {
        $vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
        foreach ($case in @(@{ tag = 'on'; extra = ''; marked = $true }, @{ tag = 'off'; extra = "mark_hidden_windows = false`n"; marked = $false })) {
            V4Config $appdata $save ($base + $case.extra)
            $png = "$save\v064-hidden-$($case.tag).png"
            $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--path', $png -PassThru -RedirectStandardError "$out\v064-hidden-$($case.tag)-stderr.txt" }; $null = $p.Handle
            # Our window keeps answering while rustshot runs (a hung one is ghosted).
            $end = (Get-Date).AddSeconds(20)
            while (-not $p.HasExited -and (Get-Date) -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 }
            StopPid $p
            $ok = $p.ExitCode -eq 0 -and (Test-Path $png)
            Check "v064 hidden ($($case.tag)): ``rustshot full --path`` saves the screen" $ok "exit=$($p.ExitCode)"
            if (-not $ok) { continue }
            Copy-Item $png "$out\v064-hidden-$($case.tag).png"
            $b = New-Object System.Drawing.Bitmap $png
            try {
                # The PNG is physical px; this (DPI-unaware) process's coordinates are logical.
                $k = $b.Width / [double]$vs.Width
                $toX = { param($x) [int](($f.Left - $vs.Left + $x) * $k) }; $toY = { param($y) [int](($f.Top - $vs.Top + $y) * $k) }
                $w = $f.Width; $hh = $f.Height
                $flat = @(@(12, 12), @(($w - 12), 12), @(12, ($hh - 12)), @(($w - 12), ($hh - 12)), @(20, [int]($hh / 2)))
                $px = @($flat | ForEach-Object { $b.GetPixel((& $toX $_[0]), (& $toY $_[1])) })
                $nPanel = @($px | Where-Object { V64Panel $_ }).Count
                $nMag = @($px | Where-Object { V64Magenta $_ }).Count
                $hex = ($px | ForEach-Object { '{0:X2}{1:X2}{2:X2}' -f $_.R, $_.G, $_.B }) -join ','
                if ($case.marked) {
                    $ink = 0
                    for ($y = [int]($hh / 2) - 40; $y -lt [int]($hh / 2) + 40; $y += 2) { for ($x = [int]($w / 2) - 60; $x -lt [int]($w / 2) + 60; $x += 2) {
                        if (-not (V64Panel $b.GetPixel((& $toX $x), (& $toY $y)))) { $ink++ }
                    } }
                    Check 'v064 hidden: a WDA_EXCLUDEFROMCAPTURE window is the "Hidden from screenshots" placeholder in the saved full capture (panel at its edges, the label in its middle)' ($nPanel -eq $flat.Count -and $ink -gt 50) "panel=$nPanel/$($flat.Count) [$hex] label ink=$ink (every 2nd px) scale=$k out\v064-hidden-on.png"
                } else {
                    Check 'v064 hidden: with mark_hidden_windows = false the capture shows what is behind it (no panel, not the window''s magenta)' ($nPanel -lt $flat.Count -and $nMag -eq 0) "panel=$nPanel magenta=$nMag [$hex] out\v064-hidden-off.png"
                }
            } finally { $b.Dispose() }
        }
    }
    $f.Close(); $f.Dispose()

    # ------------------------------------------------ Settings: 680 x 552 logical, the six Shortcuts sections, Capture's new switch
    V4Config $appdata $save $base
    $s = OpenSettings $exe $vars 'v064'
    Check 'v064 settings: `rustshot settings` opens its window' ($s.h -ne [IntPtr]::Zero)
    if ($s.h -ne [IntPtr]::Zero) {
        $c = [V64]::Client($s.h)
        $k = $c[2] / 96.0
        $lw = [Math]::Round($c[0] / $k); $lh = [Math]::Round($c[1] / $k)
        Check 'v064 settings: the window''s client area is 680 x 552 logical px (552 tall, was 520)' ($lw -eq 680 -and $lh -eq 552) "client=$($c[0])x$($c[1]) px at $($c[2]) dpi = ${lw}x$lh"
        # From About up, one section at a time: (rail name, a row only on that page).
        [void](V64Rail $s.h $focus 0x23)   # End: About
        foreach ($sec in @(@('Edit window', 'key_zoom_in'), @('Overlay actions', 'key_accept'), @('Overlay editing', 'key_undo'),
                           @('Overlay tools', 'key_tool_text'), @('Recording', 'key_rec_mute'), @('Global', 'global3'))) {
            $railed = V64Rail $s.h $focus 0x26   # Up
            $reached = $railed -and (TabTo $s.h $focus $sec[1] 60)
            $slug = ($sec[0] -replace ' ', '-').ToLower()
            [void](WinShot $s.h "v064-keys-$slug")
            Check "v064 shortcuts: the Shortcuts > $($sec[0]) section has its rows (Tab reaches $($sec[1]) there)" $reached "rail=$railed focus=$(FocusId $focus) want=$([V50]::Id($sec[1])) (out\v064-keys-$slug.png)"
        }
        # Home: General, Down: Capture.
        [void](V64Rail $s.h $focus 0x24)
        $railed = V64Rail $s.h $focus 0x28
        $reached = $railed -and (TabTo $s.h $focus 'mark_hidden_windows' 60)
        [void](WinShot $s.h 'v064-capture')
        Check 'v064 settings: Capture has "Mark windows hidden from screenshots" (Tab reaches mark_hidden_windows)' $reached "rail=$railed focus=$(FocusId $focus) (out\v064-capture.png)"
    }
    CloseSettings $s
}
