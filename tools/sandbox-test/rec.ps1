# Recording checks (video Task 9), dot-sourced by test.ps1.
# Records 2 s MP4 and 2 s GIF through the record hotkey in an isolated
# instance (own RUSTSHOT_INSTANCE, own APPDATA, unusual hotkeys) of the
# portable exe, then probes the files: MP4 box layout (ftyp, moov, mdat)
# plus Media Foundation's property handler (duration, frame size) through
# the Shell; GIF header, trailer and frame count.
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class R {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  // Ctrl+Alt+Shift+<key> as real key presses (RegisterHotKey sees them).
  public static void Hyper(byte key) {
    byte[] m = { 0x11, 0x12, 0x10 };
    foreach (var k in m) keybd_event(k, 0, 0, IntPtr.Zero);
    keybd_event(key, 0, 0, IntPtr.Zero); keybd_event(key, 0, 2, IntPtr.Zero);
    Array.Reverse(m);
    foreach (var k in m) keybd_event(k, 0, 2, IntPtr.Zero);
  }
  public static void Key(byte key) { keybd_event(key, 0, 0, IntPtr.Zero); keybd_event(key, 0, 2, IntPtr.Zero); }
}
"@

# The top-level MP4 boxes: (type, size) in file order.
function Mp4Boxes($path) {
    $b = [System.IO.File]::ReadAllBytes($path)
    $i = 0; $boxes = @()
    while ($i + 8 -le $b.Length) {
        $size = [uint64](([uint32]$b[$i] -shl 24) -bor ([uint32]$b[$i+1] -shl 16) -bor ([uint32]$b[$i+2] -shl 8) -bor [uint32]$b[$i+3])
        $type = [System.Text.Encoding]::ASCII.GetString($b, $i + 4, 4)
        if ($size -eq 1) {   # 64-bit size
            $size = [uint64]0; for ($k = 0; $k -lt 8; $k++) { $size = ($size -shl 8) -bor $b[$i + 8 + $k] }
        } elseif ($size -eq 0) { $size = [uint64]($b.Length - $i) }
        if ($size -lt 8) { break }
        $boxes += ,@($type, $size)
        $i += [int64]$size
    }
    @{ boxes = $boxes; end = $i; len = $b.Length }
}

# GIF: header, trailer, image descriptors (one per frame, after each
# block is skipped properly), the total of the frame delays (cs).
function GifInfo($path) {
    $b = [System.IO.File]::ReadAllBytes($path)
    $hdr = [System.Text.Encoding]::ASCII.GetString($b, 0, 6)
    $w = $b[6] + 256 * $b[7]; $h = $b[8] + 256 * $b[9]
    $i = 13
    if ($b[10] -band 0x80) { $i += 3 * (1 -shl (($b[10] -band 7) + 1)) }
    $frames = 0; $delay = 0; $ok = $false
    while ($i -lt $b.Length) {
        $t = $b[$i]
        if ($t -eq 0x3B) { $ok = $true; break }
        if ($t -eq 0x21) {
            if ($b[$i+1] -eq 0xF9) { $delay += $b[$i+4] + 256 * $b[$i+5] }
            $i += 2
            while ($b[$i] -ne 0) { $i += $b[$i] + 1 }
            $i++
        } elseif ($t -eq 0x2C) {
            $frames++
            $flags = $b[$i+9]; $i += 10
            if ($flags -band 0x80) { $i += 3 * (1 -shl (($flags -band 7) + 1)) }
            $i++   # LZW min code size
            while ($b[$i] -ne 0) { $i += $b[$i] + 1 }
            $i++
        } else { break }
    }
    @{ header = $hdr; w = $w; h = $h; frames = $frames; delay = $delay; trailer = $ok }
}

function ShellProp($path, $name) {
    $sh = New-Object -ComObject Shell.Application
    $item = $sh.Namespace((Split-Path $path)).ParseName((Split-Path $path -Leaf))
    $item.ExtendedProperty($name)
}

function RecordOnce($exe, $fmt, $root) {
    $inst = "sbx-rec-$fmt"
    $appdata = "$root\appdata-$fmt"
    $save = "$root\shots-$fmt"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $save | Out-Null
    $esc = $save.Replace('\', '\\')
    Set-Content "$appdata\rustshot\config.toml" @"
save_path = "$esc"
capture_hotkey = "Ctrl+Alt+Shift+F11"
quit_hotkey = "Ctrl+Alt+Shift+F10"
record_hotkey = "Ctrl+Alt+Shift+F7"
check_updates = false
rec_format = "$fmt"
rec_countdown = false
"@
    $oldApp = $env:APPDATA; $oldInst = $env:RUSTSHOT_INSTANCE
    $env:APPDATA = $appdata; $env:RUSTSHOT_INSTANCE = $inst
    $d = Start-Process $exe -ArgumentList 'daemon' -PassThru -RedirectStandardError "$out\rec-$fmt-stderr.txt"
    $env:APPDATA = $oldApp; $env:RUSTSHOT_INSTANCE = $oldInst
    $up = WaitFor { [W]::FindWindow("rustshot_tray_$inst", [NullString]::Value) -ne [IntPtr]::Zero } 10
    Check "rec ${fmt}: isolated daemon up" ($up -and -not $d.HasExited) "instance=$inst pid=$($d.Id)"
    Start-Sleep 1
    [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point 400, 300
    [R]::Hyper(0x76)   # F7: record hotkey -> area selection in record mode
    $ov = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 8
    Check "rec ${fmt}: record hotkey opens the overlay in record mode" $ov
    Start-Sleep -Milliseconds 700
    if ($ov) { [void][R]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300 }
    [R]::Key(0x0D)     # Enter: record the monitor under the pointer
    $hidden = WaitFor { [W]::Overlay() -eq [IntPtr]::Zero } 5
    Check "rec ${fmt}: Enter starts the recording (overlay hidden)" $hidden
    $t0 = Get-Date
    Start-Sleep -Milliseconds 2500   # the encoder opens first: about 2 s recorded
    [R]::Hyper(0x76)   # record hotkey again: stop
    $saved = WaitFor { [bool](Get-ChildItem $save -Recurse -Filter "*.$fmt" -ErrorAction SilentlyContinue) } 20
    $file = Get-ChildItem $save -Recurse -Filter "*.$fmt" -ErrorAction SilentlyContinue | Select-Object -First 1
    $parts = @(Get-ChildItem $save -Recurse -Filter '*.part' -ErrorAction SilentlyContinue)
    Check "rec ${fmt}: file saved under the daily subfolder, no .part left" ($saved -and $parts.Count -eq 0) "$(if ($file) { $file.FullName.Substring($save.Length) + ' ' + $file.Length + ' B' }) parts=$($parts.Count) after=$([int]((Get-Date) - $t0).TotalMilliseconds) ms"
    if ($file) { Copy-Item $file.FullName "$out\rec-sample.$fmt" }
    Start-Sleep 1
    Check "rec ${fmt}: daemon still running after the recording" (-not $d.HasExited)
    Stop-Process -Id $d.Id -Force -ErrorAction SilentlyContinue   # the PID started above
    $null = WaitFor { $d.HasExited } 5
    $err = Get-Content "$out\rec-$fmt-stderr.txt" -ErrorAction SilentlyContinue
    if ($err) { Add-Content $log ("      stderr: " + (($err | Select-Object -First 6) -join ' | ')) }
    return $file
}

function RecordingChecks($exe) {
    $root = "$env:TEMP\rec-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $root | Out-Null
    # Something that changes on screen: a top-most window that changes colour every 100 ms.
    $flick = Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile', '-Command', @'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$f = New-Object System.Windows.Forms.Form
$f.FormBorderStyle = 'None'; $f.TopMost = $true; $f.StartPosition = 'Manual'
$f.Location = New-Object System.Drawing.Point 250, 200; $f.Size = New-Object System.Drawing.Size 320, 240
$cols = @('Red', 'Lime', 'Blue', 'White', 'Black', 'Yellow'); $i = 0
$t = New-Object System.Windows.Forms.Timer; $t.Interval = 100
$t.Add_Tick({ $script:i++; $f.BackColor = [System.Drawing.Color]::FromName($cols[$script:i % 6]) })
$t.Start(); $f.ShowInTaskbar = $false
[System.Windows.Forms.Application]::Run($f)
'@
    Start-Sleep 2

    $mp4 = RecordOnce $exe 'mp4' $root | Select-Object -Last 1
    if ($mp4) {
        $m = Mp4Boxes $mp4.FullName
        $types = ($m.boxes | ForEach-Object { $_[0] }) -join ','
        Check 'rec mp4: box layout ftyp first, moov and mdat present, boxes cover the file' (
            $m.boxes.Count -ge 3 -and $m.boxes[0][0] -eq 'ftyp' -and $types -match 'moov' -and $types -match 'mdat' -and $m.end -eq $m.len) "boxes=$types end=$($m.end) len=$($m.len)"
        $dur = ShellProp $mp4.FullName 'System.Media.Duration'
        $w = ShellProp $mp4.FullName 'System.Video.FrameWidth'
        $h = ShellProp $mp4.FullName 'System.Video.FrameHeight'
        $fr = ShellProp $mp4.FullName 'System.Video.FrameRate'
        $sec = if ($dur) { [double]$dur / 1e7 } else { 0 }
        Check 'rec mp4: Media Foundation reads it (duration 1.5-3.5 s, frame size)' ($sec -ge 1.5 -and $sec -le 3.5 -and $w -gt 0 -and $h -gt 0) ("duration={0:N2} s size={1}x{2} fps={3}" -f $sec, $w, $h, $(if ($fr) { $fr / 1000 }))
    }
    $gif = RecordOnce $exe 'gif' $root | Select-Object -Last 1
    if ($gif) {
        $g = GifInfo $gif.FullName
        Check 'rec gif: GIF89a with a trailer, frames >= 10, delays 1.5-3.5 s' (
            $g.header -eq 'GIF89a' -and $g.trailer -and $g.frames -ge 10 -and $g.delay -ge 150 -and $g.delay -le 350) "size=$($g.w)x$($g.h) frames=$($g.frames) delay=$($g.delay) cs"
        $dim = ShellProp $gif.FullName 'System.Image.Dimensions'
        Check 'rec gif: Windows reads its dimensions' ([bool]$dim) "$dim"
    }
    Stop-Process -Id $flick.Id -Force -ErrorAction SilentlyContinue   # the PID started above
}
