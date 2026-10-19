# 0.6.5 checks, dot-sourced by test.ps1 and run on the portable exe:
# - Settings > About has no links row: from the rail (posted Tab, End),
#   Tab goes round the About page back to the rail; the controls it
#   reaches include powernapps.net (link_powernapps) and none of the old
#   links (link_0 .. link_5: Website, Source on GitHub, Report an issue,
#   What's new, Licenses). out\v065-about.png shows the page (and its
#   footer, "2026 PowerNapps - by Denis Platonov - Apache-2.0").
# - Several hotkeys in use by another app: this PowerShell process holds
#   4, then 8 unusual chords (RegisterHotKey, Ctrl+Alt+Shift+F1 .. F8); a
#   daemon of an isolated instance whose slots want them opens Settings
#   with the hotkey summary dialog. Tab reaches both of its buttons (q_ok,
#   q_change), and the card (the pixels the dialog changes beyond the
#   scrim: the same page after OK, darkened as the scrim does, is the
#   reference) lies inside the client area, centred, the 8-failure card
#   ("and N more") taller than the 4-failure one (out\v065-hotkeys-*.png).
#   The text's lines themselves are checked by the unit fit tests
#   (settings_ui::tests, every language at 100 and 200 %).
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped. Settings gets
# posted keys; nothing else is typed or clicked.

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class V65 {
  [DllImport("user32.dll")] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  // The client area on screen in physical px (asked on a per-monitor-aware v2 thread).
  public static int[] Client(IntPtr h) {
    IntPtr old = SetThreadDpiAwarenessContext((IntPtr)(-4));
    try { RECT r; GetClientRect(h, out r); POINT p = new POINT(); ClientToScreen(h, ref p); return new int[] { p.X, p.Y, r.R - r.L, r.B - r.T }; }
    finally { if (old != IntPtr.Zero) SetThreadDpiAwarenessContext(old); }
  }
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  // A picture of the window's client area as it draws it (PrintWindow,
  // PW_CLIENTONLY | PW_RENDERFULLCONTENT: other windows over it don't
  // matter), c[2] x c[3] physical px.
  public static System.Drawing.Bitmap Grab(IntPtr h, int[] c) {
    IntPtr old = SetThreadDpiAwarenessContext((IntPtr)(-4));
    try {
      var b = new System.Drawing.Bitmap(c[2], c[3]);
      using (var g = System.Drawing.Graphics.FromImage(b)) { IntPtr dc = g.GetHdc(); try { PrintWindow(h, dc, 3); } finally { g.ReleaseHdc(dc); } }
      return b;
    } finally { if (old != IntPtr.Zero) SetThreadDpiAwarenessContext(old); }
  }
  // The box of the pixels of a (the dialog) that are not b (the page) under
  // the dark theme's scrim (black at 120/255; tolerance t), connected to
  // the middle of the picture (the card is centred), measured along the
  // middle row and column.
  public static int[] Card(System.Drawing.Bitmap a, System.Drawing.Bitmap b, int t) {
    int w = a.Width, h = a.Height;
    var ra = new System.Drawing.Rectangle(0, 0, w, h);
    var da = a.LockBits(ra, System.Drawing.Imaging.ImageLockMode.ReadOnly, System.Drawing.Imaging.PixelFormat.Format32bppArgb);
    var db = b.LockBits(ra, System.Drawing.Imaging.ImageLockMode.ReadOnly, System.Drawing.Imaging.PixelFormat.Format32bppArgb);
    bool[] off = new bool[w * h];
    try {
      int n = da.Stride * h; byte[] pa = new byte[n], pb = new byte[n];
      Marshal.Copy(da.Scan0, pa, 0, n); Marshal.Copy(db.Scan0, pb, 0, n);
      for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
        int i = y * da.Stride + x * 4;
        for (int c = 0; c < 3; c++) { int want = pb[i + c] * 135 / 255; if (Math.Abs(pa[i + c] - want) > t) off[y * w + x] = true; }
      }
    } finally { a.UnlockBits(da); b.UnlockBits(db); }
    int x0 = int.MaxValue, y0 = int.MaxValue, x1 = -1, y1 = -1;
    int start = (h / 2) * w + w / 2;
    if (!off[start]) return new int[] { -1, -1, -1, -1 };
    var seen = new bool[w * h]; var q = new System.Collections.Generic.Queue<int>(); q.Enqueue(start); seen[start] = true;
    while (q.Count > 0) {
      int p = q.Dequeue(), x = p % w, y = p / w;
      int[] nb = { x > 0 ? p - 1 : -1, x < w - 1 ? p + 1 : -1, y > 0 ? p - w : -1, y < h - 1 ? p + w : -1 };
      foreach (int m in nb) if (m >= 0 && off[m] && !seen[m]) { seen[m] = true; q.Enqueue(m); }
    }
    // The card is a rectangle: along rows and columns through its middle
    // its extent is the narrowest one (page pixels the shadow happens to
    // touch, like a focus ring, only ever widen a row).
    for (int d = -30; d <= 30; d += 5) {
      int y = h / 2 + d, x = w / 2 + d, a0 = -1, a1 = -1, b0 = -1, b1 = -1;
      for (int i = 0; i < w; i++) if (seen[y * w + i]) { if (a0 < 0) a0 = i; a1 = i; }
      for (int j = 0; j < h; j++) if (seen[j * w + x]) { if (b0 < 0) b0 = j; b1 = j; }
      if (a0 < 0 || b0 < 0) return new int[] { -1, -1, -1, -1 };
      x0 = x0 == int.MaxValue ? a0 : Math.Max(x0, a0); x1 = x1 < 0 ? a1 : Math.Min(x1, a1);
      y0 = y0 == int.MaxValue ? b0 : Math.Max(y0, b0); y1 = y1 < 0 ? b1 : Math.Min(y1, b1);
    }
    return new int[] { x0, y0, x1, y1 };
  }
}
"@

# Every control Tab reaches on the page from the rail, until the rail again (ids).
function V65Round($h, $focus, $max = 40) {
    $rail = [V50]::Id('rail')
    $seen = @()
    for ($i = 0; $i -lt $max; $i++) {
        $was = FocusId $focus
        [V3]::PostKey($h, 0x09)
        $null = WaitFor { (FocusId $focus) -ne $was } 3
        $now = FocusId $focus
        if ($now -eq $rail -or $now -eq $was) { break }
        $seen += $now
    }
    , $seen
}

function V65Config($appdata, $save, $chords) {
    $esc = $save.Replace('\', '\\')
    $slots = @('capture_hotkey', 'quit_hotkey', 'record_hotkey', 'capture_full_hotkey', 'capture_window_hotkey', 'repeat_hotkey', 'capture_text_hotkey', 'history_hotkey', 'read_qr_hotkey')
    $lines = @("save_path = `"$esc`"", 'theme = "dark"', 'history = false', 'check_updates = false', 'show_thumbnail = false', 'screenclip_hint = false')
    for ($i = 0; $i -lt $slots.Count; $i++) { $c = if ($i -lt $chords.Count) { $chords[$i] } else { '' }; $lines += "$($slots[$i]) = `"$c`"" }
    Set-Content "$appdata\rustshot\config.toml" ($lines -join "`n")
}

function Checks065($exe) {
    $root = "$env:TEMP\v065-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $focus = "$root\focus.txt"
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-065'; RUSTSHOT_TEST_FOCUS_FILE = $focus }

    # ------------------------------------------------ Settings > About: no links row, powernapps.net stays
    V65Config $appdata $save @('Ctrl+Alt+Shift+F11', 'Ctrl+Alt+Shift+F10')
    $s = OpenSettings $exe $vars '065-about'
    $onRail = $s.h -ne [IntPtr]::Zero -and (TabTo $s.h $focus 'rail')
    Check 'v065 about: Tab reaches the section rail' $onRail "focus=$(FocusId $focus)"
    if ($onRail) {
        [V3]::PostKey($s.h, 0x23); Start-Sleep 1   # End: About
        $null = WinShot $s.h 'v065-about'
        $ids = V65Round $s.h $focus
        $pn = [V50]::Id('link_powernapps')
        $old = @(0..5 | ForEach-Object { [V50]::Id("link_$_") })
        $hits = @($ids | Where-Object { $old -contains $_ })
        Check 'v065 about: Tab round About reaches powernapps.net (link_powernapps)' ($ids -contains $pn) "reached $($ids.Count): $($ids -join ',')"
        Check 'v065 about: ... and no links row (none of link_0 .. link_5: Website, GitHub, Report an issue, What''s new, Licenses; out\v065-about.png)' ($hits.Count -eq 0 -and $ids.Count -gt 0) "old links reached=$($hits.Count)"
    }
    CloseSettings $s

    # ------------------------------------------------ several hotkeys in use: the summary dialog inside the window
    $cards = @{}
    foreach ($n in @(4, 8)) {
        $chords = @(1..$n | ForEach-Object { "Ctrl+Alt+Shift+F$_" })
        $held = @()
        for ($i = 1; $i -le $n; $i++) { if ([V65]::RegisterHotKey([IntPtr]::Zero, 0x6500 + $i, 0x4007, [uint32](0x6F + $i))) { $held += (0x6500 + $i) } }   # MOD_ALT|CONTROL|SHIFT|NOREPEAT, VK_F1..
        try {
            if ($held.Count -ne $n) { Add-Content $log "SKIP  v065 hotkeys ($n): this process could hold only $($held.Count) of Ctrl+Alt+Shift+F1..F$n"; continue }
            V65Config $appdata $save $chords
            Remove-Item $focus -ErrorAction SilentlyContinue
            $dm = WithEnv $vars { Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError "$out\v065-hotkeys-$n-stderr.txt" }; $null = $dm.Handle
            $null = WaitFor { [V50]::Find('rustshot_window', [uint32]$dm.Id) -ne [IntPtr]::Zero } 15
            Start-Sleep 2
            $h = [V50]::Find('rustshot_window', [uint32]$dm.Id)
            Check "v065 hotkeys ($n): a daemon whose $n shortcuts another app holds opens Settings with the hotkey dialog" ($h -ne [IntPtr]::Zero) "pid=$($dm.Id)"
            if ($h -ne [IntPtr]::Zero) {
                $c = [V65]::Client($h)
                $k = $c[3] / 552.0
                $a = [V65]::Grab($h, $c); $a.Save("$out\v065-hotkeys-$n.png")
                $okId = [V50]::Id('q_ok'); $chId = [V50]::Id('q_change')
                $both = (TabTo $h $focus 'q_change' 6) -and (TabTo $h $focus 'q_ok' 6)
                Check "v065 hotkeys ($n): Tab reaches both of the dialog's buttons (OK, Change shortcuts)" $both "focus=$(FocusId $focus) ok=$okId change=$chId"
                # OK (focused): the page beneath, for the reference.
                [V3]::PostKey($h, 0x0D)
                Start-Sleep 1
                $b = [V65]::Grab($h, $c); $b.Save("$out\v065-hotkeys-$n-after.png")
                # (The client area has no title bar: Windows draws it.) Tolerance 10.
                $top = 0
                $box = [V65]::Card($a, $b, 10)
                $a.Dispose(); $b.Dispose()
                $inside = $box[2] -ge 0 -and $box[0] -gt 0 -and $box[1] -gt $top -and $box[2] -lt $c[2] - 1 -and $box[3] -lt $c[3] - 1
                $mt = $box[1] - $top; $mb = $c[3] - 1 - $box[3]
                $ml = $box[0]; $mr = $c[2] - 1 - $box[2]
                $centred = [Math]::Abs($mt - $mb) -le [int](40 * $k) -and [Math]::Abs($ml - $mr) -le [int](40 * $k)
                $cards[$n] = $box[3] - $box[1]
                Check "v065 hotkeys ($n): the dialog's card lies inside the window's client area, centred both ways (out\v065-hotkeys-$n.png)" ($inside -and $centred) "card=($($box[0]),$($box[1]))-($($box[2]),$($box[3])) client=$($c[2])x$($c[3]) margins top/bottom=$mt/$mb left/right=$ml/$mr"
            }
            if ($h -ne [IntPtr]::Zero) { [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
            StopPid $dm
        } finally {
            foreach ($id in $held) { [void][V65]::UnregisterHotKey([IntPtr]::Zero, $id) }
        }
    }
    if ($cards.ContainsKey(4) -and $cards.ContainsKey(8)) {
        Check 'v065 hotkeys: the 8-failure card ("and N more") is taller than the 4-failure one' ($cards[8] -gt $cards[4]) "heights 4=$($cards[4]) 8=$($cards[8]) px"
    }
}
