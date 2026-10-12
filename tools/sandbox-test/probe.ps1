# Win+Shift+<key> RegisterHotKey probe on the clean system, dot-sourced by test.ps1.
# Writes one line per key: FREE / TAKEN (with the Win32 error).
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class HK {
  [DllImport("user32.dll", SetLastError=true)] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
  public static int Probe(uint vk) {
    if (RegisterHotKey(IntPtr.Zero, 0x5100, 0x0008 | 0x0004 | 0x4000, vk)) { UnregisterHotKey(IntPtr.Zero, 0x5100); return 0; }
    return Marshal.GetLastWin32Error();
  }
}
"@
function HotkeyProbe($log) {
    # Let explorer finish registering its own hotkeys first.
    $null = WaitFor { @(Get-Process explorer -ErrorAction SilentlyContinue).Count -gt 0 } 60
    Start-Sleep 20
    $keys = [char[]]'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
    $taken = @()
    foreach ($k in $keys) {
        $e = [HK]::Probe([uint32][char]$k)
        if ($e -ne 0) { $taken += "$k($e)" }
    }
    Add-Content $log "      probe Win+Shift taken: $($taken -join ' ')"
    foreach ($k in 'X','F','E','R','T','H','Z','W','A','Q','G','J','Y') {
        $e = [HK]::Probe([uint32][char]$k)
        Check "probe: Win+Shift+$k registers on a clean system" ($e -eq 0) "err=$e"
    }
}
