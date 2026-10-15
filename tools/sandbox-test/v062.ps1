# 0.6.2 checks, dot-sourced by test.ps1 and run on the portable exe:
# the tray icon is the app icon. A daemon of an isolated instance adds its
# notification-area icon (NIM_MODIFY on its hWnd and uID 1 succeeds only for
# an icon that exists), drawn by the app (its stderr has no "could not draw
# the tray icon", which it prints when it falls back to the exe's icon, nor
# "could not add the notification-area icon"); the taskbar's button for it
# (UI Automation, when the icon is not in the overflow) shows the blue tile,
# cropped to out\v062-tray-icon.png.
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA, no
# hotkeys. Only the PID started here is stopped. No input is sent.

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class V62 {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct NID {
    public uint cbSize; public IntPtr hWnd; public uint uID; public uint uFlags; public uint uCallbackMessage; public IntPtr hIcon;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string szTip;
    public uint dwState; public uint dwStateMask;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string szInfo;
    public uint uVersion;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string szInfoTitle;
    public uint dwInfoFlags; public Guid guidItem; public IntPtr hBalloonIcon;
  }
  [DllImport("shell32.dll", CharSet = CharSet.Unicode)] public static extern bool Shell_NotifyIconW(uint msg, ref NID d);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  // NIM_MODIFY (1) of the tip only: true only when the icon exists.
  public static bool IconExists(IntPtr h, string tip) {
    var d = new NID(); d.cbSize = (uint)Marshal.SizeOf(typeof(NID)); d.hWnd = h; d.uID = 1; d.uFlags = 4; d.szTip = tip;
    return Shell_NotifyIconW(1, ref d);
  }
}
"@

# The taskbar's buttons whose name starts with Rustshot (the notification icons).
function TrayButtons {
    $tb = [V62]::FindWindow('Shell_TrayWnd', $null)
    if ($tb -eq [IntPtr]::Zero) { return @() }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($tb)
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    @($all | Where-Object { "$($_.Current.Name)" -like 'Rustshot*' -and -not $_.Current.IsOffscreen -and $_.Current.BoundingRectangle.Width -gt 4 })
}

function Checks062($exe) {
    $root = "$env:TEMP\v062-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-062' }
    $base = "history = false`nrecord_hotkey = `"`"`ncapture_full_hotkey = `"`"`ncapture_window_hotkey = `"`"`nrepeat_hotkey = `"`"`ncapture_text_hotkey = `"`"`n"
    V4Config $appdata $save $base
    $before = @(TrayButtons).Count

    $err = "$out\v062-daemon-stderr.txt"
    $dm = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError $err }
    $h = [IntPtr]::Zero
    $up = WaitFor { $script:v62h = [V62]::FindWindow('rustshot_tray_sbx-062', $null); $script:v62h -ne [IntPtr]::Zero } 10
    $h = $script:v62h
    Check 'v062 tray: the daemon makes its tray window (rustshot_tray_sbx-062)' ($up -and [W]::Owner($h) -eq $dm.Id) "pid=$($dm.Id)"
    if ($up) {
        $icon = WaitFor { [V62]::IconExists($h, 'Rustshot') } 10
        Check 'v062 tray: its notification-area icon exists (NIM_MODIFY on its hWnd, uID 1)' $icon
        Start-Sleep 2
        $lines = @(Get-Content $err -ErrorAction SilentlyContinue)
        $bad = @($lines | Where-Object { $_ -match 'could not draw the tray icon|could not add the notification-area icon' })
        Check 'v062 tray: the icon is the drawn app icon, no fallback (stderr)' ($bad.Count -eq 0) "$($bad -join ' | ')"
        $btn = @(TrayButtons)
        if ($btn.Count -gt $before) {
            $r = $btn[-1].Current.BoundingRectangle
            $x = [int]$r.X; $y = [int]$r.Y; $w = [int]$r.Width; $hh = [int]$r.Height
            $bmp = New-Object System.Drawing.Bitmap $w, $hh
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
            $bmp.Save("$out\v062-tray-icon.png")
            $blue = 0; $white = 0
            for ($j = 0; $j -lt $hh; $j++) { for ($i = 0; $i -lt $w; $i++) {
                $c = $bmp.GetPixel($i, $j)
                if ($c.B -gt 170 -and ($c.B - $c.R) -gt 90) { $blue++ }
                if ($c.R -gt 230 -and $c.G -gt 230 -and $c.B -gt 230) { $white++ }
            } }
            $g.Dispose(); $bmp.Dispose()
            Check 'v062 tray: the taskbar shows the blue tile with the white glyph (out\v062-tray-icon.png)' ($blue -ge 40 -and $white -ge 4) "button '$($btn[-1].Current.Name)' ${w}x$hh at $x,$y; blue=$blue white=$white"
        } else {
            Add-Content $log "SKIP  v062 tray: no visible taskbar button for the icon (in the overflow?); taskbar buttons named Rustshot: $($btn.Count), before: $before"
        }
        Shot 'v062-desktop'
    }
    StopPid $dm
}
