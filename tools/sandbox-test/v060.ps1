# 0.6.0 checks, dot-sourced by test.ps1 and run on the portable exe:
# Settings › About (the rail's last section, reached with Tab and End):
# its powernapps.net link opens https://powernapps.net/r/rustshot (0.6.0's
# Website link went in 0.6.5, with the rest of the links row: v065.ps1)
# (Enter on the focused link; the browser this sandbox starts for it is
# found by its command line, the sandbox has no network) and the footer
# names Apache-2.0 (on the screenshot: the catalog in the exe is packed).
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA, no
# hotkeys. Only the PIDs started here are stopped. Settings gets posted
# keys. Uses v050.ps1's helpers (WinShot, TabTo, OpenSettings, CloseSettings).

$PowerNapps = 'https://powernapps.net/r/rustshot'

# Processes (other than rustshot) whose command line names `url`.
function UrlProcs($url) {
    @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'rustshot.exe' -and "$($_.CommandLine)" -like "*$url*" })
}

function Checks060($exe) {
    $root = "$env:TEMP\v060-test"
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    $appdata = "$root\appdata"; $local = "$root\localappdata"; $save = "$root\shots"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local, $save | Out-Null
    $focus = "$root\focus.txt"
    $vars = @{ APPDATA = $appdata; LOCALAPPDATA = $local; RUSTSHOT_INSTANCE = 'sbx-060'; RUSTSHOT_TEST_FOCUS_FILE = $focus }
    $base = "history = false`noverlay_hints = false`nmagnifier = false`nrecord_hotkey = `"`"`ncapture_full_hotkey = `"`"`ncapture_window_hotkey = `"`"`nrepeat_hotkey = `"`"`ncapture_text_hotkey = `"`"`n"
    V4Config $appdata $save $base

    # ------------------------------------------------ the exe: the short link
    $bytes = [System.IO.File]::ReadAllBytes($exe)
    $txt = [System.Text.Encoding]::UTF8.GetString($bytes)
    Check 'v060 about: the exe opens https://powernapps.net/r/rustshot (the short link)' ($txt.Contains($PowerNapps) ) ''
    # (The footer's words are in the DEFLATE-packed catalog: read them on out\v060-settings-about.png.)

    # ------------------------------------------------ Settings › About: the links
    $s = OpenSettings $exe $vars '060-about'
    $onRail = $s.h -ne [IntPtr]::Zero -and (TabTo $s.h $focus 'rail')
    Check 'v060 about: Tab reaches the section rail' $onRail "focus=$(FocusId $focus) want=$([V50]::Id('rail'))"
    if ($onRail) {
        [V3]::PostKey($s.h, 0x23); Start-Sleep 1   # End: the last section, About
        $null = WinShot $s.h 'v060-settings-about'
        $pn = TabTo $s.h $focus 'link_powernapps'
        Check 'v060 about: More from PowerNapps has its powernapps.net link (Tab reaches "link_powernapps")' $pn "focus=$(FocusId $focus)"
        if ($pn) {
            $null = WinShot $s.h 'v060-settings-about-powernapps'
            [V3]::PostKey($s.h, 0x0D)
            $opened = WaitFor { @(UrlProcs $PowerNapps).Count -gt 0 } 20
            Check 'v060 about: powernapps.net opens https://powernapps.net/r/rustshot' $opened "$((UrlProcs $PowerNapps | Select-Object -First 1).CommandLine)"
            Start-Sleep 2
            Get-Process msedge -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        }
    }
    CloseSettings $s
}
