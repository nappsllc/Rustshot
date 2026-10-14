# The Apps & features (Uninstall) entry the installer writes, dot-sourced by
# test.ps1: every value, and that DisplayIcon resolves to an icon with DIB
# (BMP) images, which Settings › Installed apps needs to draw it.
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class ICO {
  public delegate bool EnumNameProc(IntPtr h, IntPtr type, IntPtr name, IntPtr l);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr LoadLibraryEx(string p, IntPtr f, uint flags);
  [DllImport("kernel32.dll")] public static extern bool FreeLibrary(IntPtr h);
  [DllImport("kernel32.dll")] public static extern bool EnumResourceNames(IntPtr h, IntPtr type, EnumNameProc f, IntPtr l);
  [DllImport("kernel32.dll")] public static extern IntPtr FindResource(IntPtr h, IntPtr name, IntPtr type);
  [DllImport("kernel32.dll")] public static extern IntPtr LoadResource(IntPtr h, IntPtr r);
  [DllImport("kernel32.dll")] public static extern IntPtr LockResource(IntPtr r);
  [DllImport("kernel32.dll")] public static extern uint SizeofResource(IntPtr h, IntPtr r);
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern uint ExtractIconEx(string f, int i, IntPtr[] large, IntPtr[] small, uint n);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr h);
  // RT_ICON images in the file: "dib png" counts.
  public static string Kinds(string path) {
    IntPtr h = LoadLibraryEx(path, IntPtr.Zero, 0x2 | 0x20);   // as a data file / image resource
    if (h == IntPtr.Zero) return "load failed";
    int dib = 0, png = 0;
    EnumResourceNames(h, (IntPtr)3, (m, t, n, l) => {
      IntPtr r = FindResource(m, n, t); IntPtr p = LockResource(LoadResource(m, r));
      if (SizeofResource(m, r) >= 4) {
        byte[] b = new byte[4]; Marshal.Copy(p, b, 0, 4);
        if (b[0] == 0x89 && b[1] == 0x50) png++; else if (BitConverter.ToInt32(b, 0) == 40) dib++;
      }
      return true; }, IntPtr.Zero);
    FreeLibrary(h);
    return dib + " " + png;
  }
  public static uint Count(string path, int index) {
    IntPtr[] l = new IntPtr[1], s = new IntPtr[1];
    uint n = ExtractIconEx(path, index, l, s, 1);
    if (l[0] != IntPtr.Zero) DestroyIcon(l[0]); if (s[0] != IntPtr.Zero) DestroyIcon(s[0]);
    return n;
  }
}
"@

function UninstKeyChecks($label, $inst, $version) {
    $k = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\rustshot' -ErrorAction SilentlyContinue
    if (-not $k) { Check "$label uninstall key: present" $false; return }
    $exe = "$inst\rustshot.exe"
    $want = [ordered]@{
        DisplayName = 'Rustshot'; Publisher = 'PowerNapps'; DisplayVersion = $version
        DisplayIcon = "$exe,0"; InstallLocation = $inst
        UninstallString = "`"$inst\uninstall.exe`""; QuietUninstallString = "`"$inst\uninstall.exe`" /S"
        URLInfoAbout = 'https://powernapps.net/r/rustshot'; HelpLink = 'https://github.com/nappsllc/rustshot/issues'
        URLUpdateInfo = 'https://github.com/nappsllc/rustshot/releases'; NoModify = 1; NoRepair = 1
    }
    $bad = @()
    foreach ($n in $want.Keys) { if ("$($k.$n)" -ne "$($want[$n])") { $bad += "$n=[$($k.$n)] want [$($want[$n])]" } }
    Check "$label uninstall key: name, publisher, version, icon, location, links, NoModify/NoRepair" ($bad.Count -eq 0) ($bad -join '; ')
    $kb = [int](((Get-ChildItem $inst -Recurse -File | Measure-Object Length -Sum).Sum) / 1024)
    $type = (Get-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\rustshot').GetValueKind('EstimatedSize')
    Check "$label uninstall key: EstimatedSize is a DWORD close to the install folder" ($type -eq 'DWord' -and $k.EstimatedSize -gt 0 -and [math]::Abs($k.EstimatedSize - $kb) -le [math]::Max(64, $kb / 10)) "EstimatedSize=$($k.EstimatedSize) KB ($type) folder=$kb KB"
    $path, $idx = $k.DisplayIcon -split ',', 2
    $n = [ICO]::Count($path, [int]$idx)
    $kinds = [ICO]::Kinds($path)
    $dib = [int]($kinds -split ' ')[0]
    Check "$label uninstall key: DisplayIcon resolves to an icon with DIB images" ($n -ge 1 -and $dib -ge 1) "icons=$n images dib/png=$kinds"
}
