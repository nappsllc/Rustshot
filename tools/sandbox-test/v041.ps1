# 0.4.1 checks, dot-sourced by test.ps1 and run on the portable exe:
# the exe's 16 and 32 px icons as the shell extracts them (ExtractIconEx,
# what Task Manager and Explorer use; the small entries are DIBs) are not
# blank, and a PNG written by the in-repo encoder reads back in Windows' own
# decoders (GDI+ and WIC) at its size, with the same pixels in both and as
# a BMP of the same region.
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA,
# unusual hotkeys. Only the PIDs started here are stopped.

Add-Type @"
using System; using System.Runtime.InteropServices;
public static class V41 {
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)]
  public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint n);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr h);
}
"@

# Opaque pixels and distinct colours of an icon handle drawn into a bitmap.
function IconStats([IntPtr]$h) {
    $ico = [System.Drawing.Icon]::FromHandle($h)
    $bmp = $ico.ToBitmap()
    try {
        $opaque = 0; $cols = @{}
        for ($y = 0; $y -lt $bmp.Height; $y++) {
            for ($x = 0; $x -lt $bmp.Width; $x++) {
                $c = $bmp.GetPixel($x, $y)
                if ($c.A -gt 0) { $opaque++; $cols[$c.ToArgb()] = 1 }
            }
        }
        @{ w = $bmp.Width; h = $bmp.Height; opaque = $opaque; colors = $cols.Count; bmp = $bmp.Clone() }
    } finally { $bmp.Dispose(); $ico.Dispose() }
}

# Width x height and BGRA pixels of an image read by WIC.
function WicRead($path) {
    Add-Type -AssemblyName PresentationCore
    $fs = [System.IO.File]::OpenRead($path)
    try {
        $dec = [System.Windows.Media.Imaging.BitmapDecoder]::Create($fs,
            [System.Windows.Media.Imaging.BitmapCreateOptions]::PreservePixelFormat,
            [System.Windows.Media.Imaging.BitmapCacheOption]::OnLoad)
        $f = $dec.Frames[0]
        $cv = New-Object System.Windows.Media.Imaging.FormatConvertedBitmap($f, [System.Windows.Media.PixelFormats]::Bgra32, $null, 0)
        $stride = $cv.PixelWidth * 4
        $px = New-Object byte[] ($stride * $cv.PixelHeight)
        $cv.CopyPixels($px, $stride, 0)
        @{ w = $cv.PixelWidth; h = $cv.PixelHeight; px = $px; fmt = "$($f.Format)"; codec = $dec.CodecInfo.FriendlyName }
    } finally { $fs.Dispose() }
}

# Samples (every 3rd px each way) where GDI+'s pixel differs from WIC's.
function GdiVsWic($path, $wic) {
    $b = New-Object System.Drawing.Bitmap $path
    try {
        $n = 0; $bad = 0
        for ($y = 0; $y -lt $b.Height; $y += 3) {
            for ($x = 0; $x -lt $b.Width; $x += 3) {
                $c = $b.GetPixel($x, $y); $i = ($y * $wic.w + $x) * 4
                $n++
                if ($c.B -ne $wic.px[$i] -or $c.G -ne $wic.px[$i + 1] -or $c.R -ne $wic.px[$i + 2] -or $c.A -ne $wic.px[$i + 3]) { $bad++ }
            }
        }
        @{ n = $n; bad = $bad }
    } finally { $b.Dispose() }
}

function Checks041($exe) {
    # ------------------------------------------------ icon: ExtractIconEx's 32 px (large) and 16 px (small) entries
    $large = New-Object IntPtr[] 1; $small = New-Object IntPtr[] 1
    $n = [V41]::ExtractIconEx($exe, 0, $large, $small, 1)
    foreach ($k in @(@{ name = 'small'; h = $small[0]; px = 16 }, @{ name = 'large'; h = $large[0]; px = 32 })) {
        if ($k.h -eq [IntPtr]::Zero) {
            Check "v041 icon: ExtractIconEx returns a $($k.name) ($($k.px) px) icon" $false "n=$n"
            continue
        }
        $s = IconStats $k.h
        $s.bmp.Save("$out\v041-icon-$($k.px).png"); $s.bmp.Dispose()
        [void][V41]::DestroyIcon($k.h)
        $area = $s.w * $s.h
        Check "v041 icon: the $($k.px) px icon from ExtractIconEx is not blank (out\v041-icon-$($k.px).png)" ($s.w -eq $k.px -and $s.opaque -ge $area / 4 -and $s.colors -ge 8) "size=$($s.w)x$($s.h) opaque=$($s.opaque)/$area colours=$($s.colors)"
    }

    # ------------------------------------------------ PNG round trip through Windows' decoders
    $root = "$env:TEMP\v041-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $esc = $save.Replace('\', '\\')
    Set-Content "$appdata\rustshot\config.toml" @"
save_path = "$esc"
capture_hotkey = "Ctrl+Alt+Shift+F11"
quit_hotkey = "Ctrl+Alt+Shift+F10"
check_updates = false
show_thumbnail = false
screenclip_hint = false
history = false
"@
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-041' }
    $png = "$save\roundtrip.png"; $bmpPath = "$save\roundtrip.bmp"
    # A region of the (still) desktop, then the same as BMP.
    $p = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '640x480+40+40', '--path', $png -PassThru -Wait }
    $q = WithEnv $vars { Start-Process $exe -ArgumentList 'full', '--region', '640x480+40+40', '--path', $bmpPath -PassThru -Wait }
    Copy-Item $png "$out\v041-roundtrip.png" -ErrorAction SilentlyContinue
    $ok = $p.ExitCode -eq 0 -and (Test-Path $png)
    Check 'v041 png: `full --region 640x480 --path x.png` writes a PNG' ($ok -and (PngSize $png) -eq '640x480') "exit=$($p.ExitCode) size=$(PngSize $png) bytes=$(if (Test-Path $png) { (Get-Item $png).Length })"
    if ($ok) {
        $g = $null; $gerr = ''
        try { $g = New-Object System.Drawing.Bitmap $png } catch { $gerr = $_.Exception.Message }
        Check 'v041 png: GDI+ (System.Drawing) decodes it at 640x480' ($g -and $g.Width -eq 640 -and $g.Height -eq 480) "$(if ($g) { "$($g.Width)x$($g.Height) $($g.PixelFormat)" }) $gerr"
        if ($g) { $g.Dispose() }
        $w = $null; $werr = ''
        try { $w = WicRead $png } catch { $werr = $_.Exception.Message }
        Check 'v041 png: WIC decodes it at 640x480' ($w -and $w.w -eq 640 -and $w.h -eq 480) "$(if ($w) { "$($w.w)x$($w.h) $($w.fmt) ($($w.codec))" }) $werr"
        if ($w) {
            $d = GdiVsWic $png $w
            Check 'v041 png: GDI+ and WIC read the same pixels' ($d.n -gt 0 -and $d.bad -eq 0) "differing samples=$($d.bad)/$($d.n)"
            $colors = @{}; for ($i = 0; $i -lt $w.px.Length; $i += 4 * 97) { $colors[[BitConverter]::ToInt32($w.px, $i)] = 1 }
            Add-Content $log "      png: $((Get-Item $png).Length) B, about $($colors.Count) distinct colours in 1/97 of the pixels"
        }
        if ($q.ExitCode -eq 0 -and (Test-Path $bmpPath)) {
            $st = DiffStats $bmpPath $png
            $total = [Math]::Ceiling(640 / 2) * [Math]::Ceiling(480 / 2)
            Check 'v041 png: the PNG has the pixels of a BMP of the same region (decoded by GDI+)' ($st.n -ge 0 -and $st.n -le $total / 100) "differing samples=$($st.n)/$total bmp=$((Get-Item $bmpPath).Length) B"
            # How the in-repo encoder compares with GDI+'s own PNG encoder on the same pixels.
            $b = New-Object System.Drawing.Bitmap $bmpPath
            try { $b.Save("$root\gdi.png", [System.Drawing.Imaging.ImageFormat]::Png) } finally { $b.Dispose() }
            Add-Content $log "      png: rustshot $((Get-Item $png).Length) B, GDI+ re-encode $((Get-Item "$root\gdi.png").Length) B, BMP $((Get-Item $bmpPath).Length) B"
        } else {
            Check 'v041 png: a BMP of the same region' $false "exit=$($q.ExitCode)"
        }
    }
}
