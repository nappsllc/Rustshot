# 0.6.7 checks, dot-sourced by test.ps1 and called from msix.ps1 on the
# installed sideload-signed MSIX (Win+Shift+S / Print Screen in the Store
# build, the tray icon off with nothing in the background, Start at login
# through the startup task). Inside Windows Sandbox only: the defaults
# and the startup task state changed here exist in the VM alone.
# - The manifest: the Rustshot application declares ms-screenclip
#   (uap3:Protocol, Parameters `screenclip "%1"`). Windows lists Rustshot
#   as a handler: the package's AppX ProgID (its AppUserModelID
#   <PFN>!Rustshot, ContractId Windows.Protocol, the parameters), the
#   package's Capabilities\URLAssociations, SHAssocEnumHandlersForProtocol-
#   ByApplication; with no choice made Windows has none (Undecided).
# - The user picks Rustshot as the Store build's guide says: Default apps
#   at Rustshot's page (ms-settings:defaultapps?registeredAUMID=<PFN>!Rustshot)
#   lists MS-SCREENCLIP; its "Choose a default" (UI Automation) opens
#   Windows' "Select a default app" dialog: Rustshot, Set default (clicks
#   inside the VM; the "Select an app" question Windows shows for
#   ms-screenclip:, Rustshot and Always, when that did not take). Then
#   UserChoiceLatest names the package's ProgID and AssocQueryString's
#   app is Rustshot.
# - The tray icon off (config in the package's and the real AppData):
#   Start's Rustshot opens Settings, nothing else runs; ms-screenclip:
#   (legacy and current forms), Win+Shift+S and Print Screen (real key
#   presses) each start an on-demand capture of the package's exe
#   (`screenclip "<uri>"`) holding no global hotkeys, and Esc ends it;
#   the startup task at a sign-in (Explorer restarted with its session's
#   startup record cleared, as at sign-in) runs nothing (SKIP when Windows
#   didn't run the task). With the icon on, the same sign-in runs the
#   daemon (tray window), and Win+Shift+S captures in it.
# - Start at login in Rustshot's Settings (Tab, Space, OK, posted keys):
#   off, then on again; the StartupTask API in the package reads Disabled,
#   then Enabled. Turned off "by the user" (the startup state Windows
#   keeps, set in the VM's registry as Settings > Apps > Startup does),
#   the API reads DisabledByUser and Settings' switch stays off with the
#   steps shown (out\v067-startup-by-user.png).

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class V67 {
  [DllImport("shlwapi.dll", CharSet=CharSet.Unicode)]
  static extern int AssocQueryString(int flags, int str, string assoc, string extra, StringBuilder outs, ref uint len);
  // ASSOCF_IS_PROTOCOL | ASSOCF_INIT_IGNOREUNKNOWN; str: 4 FRIENDLYAPPNAME, 20 PROGID.
  public static string Query(string scheme, int str) {
    var sb = new StringBuilder(1024); uint n = 1024;
    int hr = AssocQueryString(0x1000 | 0x400, str, scheme, null, sb, ref n);
    return hr == 0 ? sb.ToString() : ("hr=0x" + hr.ToString("x8"));
  }
  [DllImport("shell32.dll", CharSet=CharSet.Unicode)]
  static extern int SHAssocEnumHandlersForProtocolByApplication(string protocol, ref Guid riid, [MarshalAs(UnmanagedType.IUnknown)] out object e);
  [ComImport, Guid("973810ae-9599-4b88-9e4d-6ee98c9552da"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  interface IEnumAssocHandlers { [PreserveSig] int Next(uint celt, [MarshalAs(UnmanagedType.LPArray, ArraySubType=UnmanagedType.IUnknown, SizeParamIndex=0)] object[] r, out uint got); }
  // How many handlers the shell lists for a protocol (-1: the call failed).
  public static int Handlers(string p) {
    var g = new Guid("973810ae-9599-4b88-9e4d-6ee98c9552da"); object o;
    if (SHAssocEnumHandlersForProtocolByApplication(p, ref g, out o) != 0) return -1;
    var en = (IEnumAssocHandlers)o; int n = 0;
    while (true) { var a = new object[1]; uint got; if (en.Next(1, a, out got) != 0 || got == 0) break; n++; }
    return n;
  }
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
  public static void Click(int x, int y) { SetCursorPos(x, y); System.Threading.Thread.Sleep(150); mouse_event(2, 0, 0, 0, IntPtr.Zero); System.Threading.Thread.Sleep(60); mouse_event(4, 0, 0, 0, IntPtr.Zero); }
  public static void Key(byte vk) { keybd_event(vk, 0, 0, IntPtr.Zero); keybd_event(vk, 0, 2, IntPtr.Zero); }
  public static void WinShiftS() {
    keybd_event(0x5B, 0, 0, IntPtr.Zero); keybd_event(0x10, 0, 0, IntPtr.Zero);
    keybd_event(0x53, 0, 0, IntPtr.Zero); keybd_event(0x53, 0, 2, IntPtr.Zero);
    keybd_event(0x10, 0, 2, IntPtr.Zero); keybd_event(0x5B, 0, 2, IntPtr.Zero);
  }
}
"@
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes

$v67Choice = 'HKCU:\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\ms-screenclip'

function V67Procs { @(Get-CimInstance Win32_Process -Filter "Name='rustshot.exe'") }
function V67Desktop { [void][W]::SetForegroundWindow([W]::FindWindow('Progman', $null)); Start-Sleep -Milliseconds 500 }
function V67Choice { (Get-ItemProperty "$v67Choice\UserChoiceLatest\ProgId" -ErrorAction SilentlyContinue).ProgId }
function V67Names($el) {
    @($el.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
        ForEach-Object { $_.Current.Name } | Where-Object { $_ } | Select-Object -Unique)
}
function V67SettingsWindow {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) |
        Where-Object { $_.Current.Name -eq 'Settings' -and (Get-Process -Id $_.Current.ProcessId -ErrorAction SilentlyContinue).Name -match 'ApplicationFrameHost|SystemSettings' } | Select-Object -First 1
}
function V67Find($root, [scriptblock]$match) {
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
        Where-Object $match | Select-Object -First 1
}
function V67Invoke($el) {
    $p = $null
    if ($el.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke(); return 'invoke' }
    if ($el.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$p)) { $p.Select(); return 'select' }
    $r = $el.Current.BoundingRectangle
    [V67]::Click([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2)); 'click'
}
function V67CloseSettingsApp {
    $s = V67SettingsWindow
    $p = $null
    if ($s -and $s.TryGetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern, [ref]$p)) { $p.Close() }
    Start-Sleep 2
}

# Rustshot's config (the icon on or off), in the real AppData and the
# package's redirected copy (a packaged process reads that one when it
# made its own).
function V67Config($pfn, $showTray) {
    $text = "show_tray = $(if ($showTray) { 'true' } else { 'false' })`ntray_off_mode_seen = true`nscreenclip_hint = false`ncheck_updates = false`nshow_thumbnail = false`n"
    foreach ($dir in "$env:APPDATA\rustshot", "$env:LOCALAPPDATA\Packages\$pfn\LocalCache\Roaming\rustshot") {
        New-Item -ItemType Directory -Force $dir | Out-Null
        Set-Content "$dir\config.toml" $text -NoNewline
    }
}

# An on-demand capture started by `how` (no daemon before): the overlay's
# process and command line, no global hotkeys held (Win+Shift+X, the
# default capture key, is free: not a daemon); Esc ends it. Returns
# @{ ok; detail }.
function V67OnDemand([scriptblock]$how, $tag, $pkg) {
    KillAll
    & $how
    $ov = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    $owner = if ($ov) { [W]::Owner([W]::Overlay()) } else { 0 }
    $p = V67Procs | Where-Object ProcessId -eq $owner | Select-Object -First 1
    $free = [HK]::Probe([uint32][char]'X')
    if ($ov) { Shot "v067-$tag" }
    $ended = $false
    if ($ov) {
        [void][W]::SetForegroundWindow([W]::Overlay()); Start-Sleep -Milliseconds 300
        [V67]::Key(0x1B)
        $ended = WaitFor { -not (V67Procs | Where-Object ProcessId -eq $owner) } 8
    }
    $ok = $ov -and $p -and "$($p.ExecutablePath)" -like "$($pkg.InstallLocation)*" -and "$($p.CommandLine)" -match ' screenclip "ms-screenclip:' -and $free -eq 0 -and $ended
    KillAll
    @{ ok = $ok; detail = "overlay=$ov win+shift+x-err=$free esc-ends=$ended cmd=$($p.CommandLine)" }
}

# A sign-in as Explorer sees it: its record of this session's startup run
# cleared, Explorer started again (it then runs the startup apps and tasks).
function V67SignIn {
    $si = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo'
    Get-ChildItem $si -ErrorAction SilentlyContinue | ForEach-Object { Remove-Item $_.PSPath -Recurse -Force -ErrorAction SilentlyContinue }
    Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.Id -Force }
    Start-Sleep 2
    if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
    $null = WaitFor { [W]::FindWindow('Shell_TrayWnd', $null) -ne [IntPtr]::Zero } 20
}

# Rustshot's Settings (`settings` through the alias) with the focus
# reported to $focus; its window handle (or zero).
function V67OpenSettings($alias, $focus) {
    Remove-Item $focus -ErrorAction SilentlyContinue
    $null = WithEnv @{ RUSTSHOT_TEST_FOCUS_FILE = $focus } { Start-Process $alias -ArgumentList 'settings' }
    $null = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 10
    Start-Sleep 2
    [W]::Settings()
}
# Space on "Launch at login" (Tab to it), a picture, then OK.
function V67ToggleLaunch($h, $focus, $tag) {
    $reached = TabTo $h $focus 'autostart' 60
    if ($reached) { [V3]::PostKey($h, 0x20); Start-Sleep -Milliseconds 800 }
    Shot "v067-startup-$tag"
    $reached
}
function V67Ok($h) {
    $c = New-Object V3+RECT
    [void][V3]::GetClientRect($h, [ref]$c)
    $sc = $c.R / 680.0
    [V3]::PostClientClick($h, [int](461 * $sc), [int]($c.B - 23 * $sc))   # OK
    if (-not (WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 8)) { [V3]::PostKey($h, 0x0D); $null = WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 5 }
}

function Checks067($pkg) {
    $man = Get-AppxPackageManifest $pkg
    $pfn = $pkg.PackageFamilyName
    $alias = "$env:LOCALAPPDATA\Microsoft\WindowsApps\rustshot.exe"
    KillAll

    # ------------------------------------------------ declared, and listed as a handler
    $main = $man.Package.Applications.Application | Where-Object Id -eq 'Rustshot'
    $proto = @($main.Extensions.Extension | Where-Object Category -eq 'windows.protocol')
    $pr = if ($proto.Count -eq 1) { $proto[0].Protocol } else { $null }
    Check 'v067 msix: the Rustshot application declares ms-screenclip (screenclip "%1")' ($pr -and $pr.Name -eq 'ms-screenclip' -and $pr.Parameters -eq 'screenclip "%1"' -and $proto[0].Executable -eq 'rustshot.exe') "count=$($proto.Count) name=$($pr.Name) params=$($pr.Parameters)"
    $ours = Get-ChildItem 'HKCU:\Software\Classes' -ErrorAction SilentlyContinue | Where-Object PSChildName -like 'AppX*' | Where-Object {
        (Get-ItemProperty "$($_.PSPath)\Application" -ErrorAction SilentlyContinue).AppUserModelID -eq "$pfn!Rustshot" -and
        (Get-ItemProperty "$($_.PSPath)\Shell\open" -ErrorAction SilentlyContinue).ContractId -eq 'Windows.Protocol' } | Select-Object -First 1
    $progid = $ours.PSChildName
    $open = if ($ours) { Get-ItemProperty "$($ours.PSPath)\Shell\open" } else { $null }
    $cap = (Get-ItemProperty "HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages\$($pkg.PackageFullName)\Rustshot\Capabilities\URLAssociations" -ErrorAction SilentlyContinue).'ms-screenclip'
    $n = [V67]::Handlers('ms-screenclip')
    Check 'v067 msix: Windows lists Rustshot as an ms-screenclip handler (its AppX ProgID, URLAssociations, the shell''s handler list)' ($progid -and $cap -eq $progid -and $open.Parameters -eq 'screenclip "%1"' -and $n -ge 2) "progid=$progid urlassoc=$cap params=$($open.Parameters) handlers=$n"
    $before = V67Choice
    Add-Content $log "      v067 before a choice: UserChoiceLatest=$before progid=$([V67]::Query('ms-screenclip', 20)) app=$([V67]::Query('ms-screenclip', 4))"

    # ------------------------------------------------ the user picks Rustshot (Default apps, as the guide says)
    # Rustshot's Settings › Capture (the rail, Down) offers "Use Win+Shift+S
    # and Print Screen"; turned on and applied, the guide shows, and its
    # "Open Default apps" opens Rustshot's page there.
    $how = ''
    $focus = "$out\v067-focus.txt"
    $h = V67OpenSettings $alias $focus
    $row = $false; $guided = $false
    if ($h -ne [IntPtr]::Zero -and (TabTo $h $focus 'rail' 80)) {
        [V3]::PostKey($h, 0x28); Start-Sleep -Milliseconds 600   # Down: Capture
        $row = TabTo $h $focus 'screenclip' 60
    }
    Check 'v067 msix: Settings › Capture offers "Use Win+Shift+S and Print Screen" (reached by Tab)' $row
    if ($row) {
        [V3]::PostKey($h, 0x20); Start-Sleep -Milliseconds 500   # Space: on
        $c = New-Object V3+RECT
        [void][V3]::GetClientRect($h, [ref]$c)
        $sc = $c.R / 680.0
        [V3]::PostClientClick($h, [int](622 * $sc), [int]($c.B - 23 * $sc))   # Apply
        Start-Sleep 2
        Shot 'v067-guide'
        if (TabTo $h $focus 'q_open_default_apps' 20) { [V3]::PostKey($h, 0x20); $guided = $true }
    }
    if (-not $guided) { Start-Process "ms-settings:defaultapps?registeredAUMID=$pfn!Rustshot" }
    $null = WaitFor { [bool](V67SettingsWindow) } 15
    Start-Sleep 4
    $set = V67SettingsWindow
    $names = if ($set) { V67Names $set } else { @() }
    Shot 'v067-default-apps'
    Check 'v067 msix: the guide''s Open Default apps opens Rustshot''s page there, listing MS-SCREENCLIP' ($guided -and $names -contains 'Rustshot' -and ($names | Where-Object { $_ -like 'MS-SCREENCLIP*' })) "guided=$guided $(($names | Select-Object -First 40) -join ' | ')"
    $btn = if ($set) { V67Find $set { $_.Current.Name -like 'MS-SCREENCLIP*' -and $_.Current.ControlType.ProgrammaticName -match 'Button|ListItem|Group' } } else { $null }
    # Windows' "Select a default app for 'MS-SCREENCLIP' links" (and the
    # "Select an app" question below) is centred on the primary screen and
    # not in the UI Automation tree: Rustshot is its first suggestion, the
    # buttons are at its bottom (clicks inside the VM).
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $cx = $b.X + $b.Width / 2; $cy = $b.Y + $b.Height / 2
    if ($btn) {
        $how = "Default apps: $(V67Invoke $btn) '$($btn.Current.Name)'"
        Start-Sleep 3
        Shot 'v067-default-apps-dialog'
        [V67]::Click([int]($cx - 129), [int]($cy - 8)); Start-Sleep 1
        Shot 'v067-default-apps-rustshot'
        [V67]::Click([int]($cx + 102), [int]($cy + 132))   # Set default
        $how += '; Rustshot, Set default'
        Start-Sleep 3
        Shot 'v067-default-apps-set'
    }
    V67CloseSettingsApp
    $viaDefaultApps = (V67Choice) -eq $progid
    # Back in Rustshot's Settings: it reads the choice (the guide closes, a
    # toast says Win+Shift+S now opens Rustshot).
    $h = [W]::Settings()
    if ($h -ne [IntPtr]::Zero -and $guided) {
        # Back to it as a user would: a click elsewhere (the desktop, so
        # it is surely not the active window), then on its title bar.
        $wr = New-Object V3+RECT
        [void][V3]::GetWindowRect($h, [ref]$wr)
        [V67]::Click([int]($b.X + $b.Width - 300), [int]($b.Y + $b.Height / 2)); Start-Sleep 1
        [V67]::Click($wr.L + 200, $wr.T + 14)
        $closed = WaitFor { (FocusId $focus) -ne "$([V50]::Id('q_open_default_apps'))" } 6
        Start-Sleep 1
        Shot 'v067-guide-done'
        Check 'v067 msix: back in Rustshot''s Settings the guide closes by itself (Windows opens Rustshot now; out\v067-guide-done.png)' ($viaDefaultApps -and $closed) "chosen=$viaDefaultApps focus=$(FocusId $focus)"
    }
    if ($h -ne [IntPtr]::Zero) {
        [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        $null = WaitFor { [W]::Settings() -eq [IntPtr]::Zero } 5
    }
    KillAll
    if (-not $viaDefaultApps) {
        # Windows' own question for a link with no default: Rustshot, Always.
        $how += '; the "Select an app" question'
        Start-Process 'ms-screenclip:?source=HotKey'
        Start-Sleep 6
        Shot 'v067-picker'
        # Rustshot, then Always (the left button).
        [V67]::Click([int]($cx - 129), [int]($cy - 8)); Start-Sleep 1
        [V67]::Click([int]($cx - 102), [int]($cy + 132))
        $null = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 10
        Start-Sleep 1
        KillAll
        Get-Process OpenWith -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.Id -Force }
    }
    $after = V67Choice
    $cli = (& $alias register-screenclip on 2>&1) -join ' '
    Check 'v067 msix: `rustshot register-screenclip on` writes nothing and says Win+Shift+S opens Rustshot' ($cli -match 'Store package already offers' -and $cli -match 'Win\+Shift\+S now: Opens Rustshot') $cli
    $app = [V67]::Query('ms-screenclip', 4)
    Check 'v067 msix: Rustshot chosen for MS-SCREENCLIP (UserChoiceLatest = the package''s ProgID, AssocQueryString = Rustshot)' ($after -eq $progid -and $app -eq 'Rustshot') "how=$how via-default-apps=$viaDefaultApps choice=$after app=$app"

    # ------------------------------------------------ the tray icon off: nothing runs, Win+Shift+S starts Rustshot
    KillAll
    V67Config $pfn $false
    Start-Process explorer.exe "shell:AppsFolder\$pfn!Rustshot"
    $sw = WaitFor { [W]::Settings() -ne [IntPtr]::Zero } 10
    Start-Sleep 1
    $tray = [W]::TrayWindow()
    if ($sw) { Shot 'v067-trayoff-start' }
    Check 'v067 msix tray off: Start''s Rustshot opens Settings, no tray window (the way back in)' ($sw -and -not $tray) "settings=$sw tray=$tray"
    if ($sw) { [void][W]::PostMessage([W]::Settings(), 0x0010, [IntPtr]::Zero, [IntPtr]::Zero); $null = WaitFor { (V67Procs).Count -eq 0 } 8 }
    Check 'v067 msix tray off: closing it leaves no Rustshot process' ((V67Procs).Count -eq 0) "$((V67Procs | ForEach-Object { "$($_.ProcessId) $($_.CommandLine)" }) -join ' ; ')"
    $r = V67OnDemand { Start-Process 'ms-screenclip:?source=HotKey' } 'trayoff-uri' $pkg
    Check 'v067 msix tray off: ms-screenclip:?source=HotKey starts an on-demand capture of the package (no global hotkeys), Esc ends it' $r.ok $r.detail
    $r = V67OnDemand { Start-Process 'ms-screenclip://capture/image?window' } 'trayoff-uri-window' $pkg
    Check 'v067 msix tray off: ms-screenclip://capture/image?window does too' $r.ok $r.detail
    $r = V67OnDemand { V67Desktop; [V67]::WinShiftS() } 'trayoff-winshifts' $pkg
    Check 'v067 msix tray off: Win+Shift+S (real key presses) starts an on-demand capture' $r.ok $r.detail
    $r = V67OnDemand { V67Desktop; [V67]::Key(0x2C) } 'trayoff-prtscn' $pkg
    Check 'v067 msix tray off: Print Screen starts an on-demand capture' $r.ok $r.detail

    # ------------------------------------------------ the startup task at a sign-in
    $task = Task066 $pfn 'Rustshot' 'enable' 'v067-enable'
    Add-Content $log "      v067 startup task before the sign-ins: $task"
    KillAll
    $seen = New-Object System.Collections.ArrayList
    V67SignIn
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 15) {
        foreach ($p in V67Procs) { if (-not ($seen | Where-Object ProcessId -eq $p.ProcessId)) { [void]$seen.Add($p) } }
        Start-Sleep -Milliseconds 100
    }
    $left = V67Procs
    $sw = [W]::Settings() -ne [IntPtr]::Zero
    Add-Content $log "      v067 sign-in, tray off: started=$(($seen | ForEach-Object { "$($_.ProcessId) $($_.CommandLine)" }) -join ' ; ') left=$($left.Count) settings=$sw tray=$([W]::TrayWindow())"
    if ($seen.Count -eq 0) {
        Add-Content $log 'SKIP  v067 msix tray off: the startup task at a sign-in runs nothing (Windows did not run the task after the Explorer restart)'
    } else {
        Check 'v067 msix tray off: the startup task at a sign-in starts Rustshot, which exits at once (no tray, no Settings, nothing left)' ($left.Count -eq 0 -and -not $sw -and -not [W]::TrayWindow()) "started=$($seen.Count) left=$($left.Count) settings=$sw"
    }
    if ($sw) { [void][W]::PostMessage([W]::Settings(), 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    KillAll
    V67Config $pfn $true
    V67SignIn
    $up = WaitFor { [W]::TrayWindow() } 20
    $d = V67Procs | Select-Object -First 1
    if ($seen.Count -eq 0 -and -not $up) {
        Add-Content $log 'SKIP  v067 msix tray on: the startup task at a sign-in runs the daemon (Windows did not run the task)'
    } else {
        Check 'v067 msix tray on: the startup task at a sign-in runs the daemon (tray window)' ($up -and -not ([W]::Settings() -ne [IntPtr]::Zero)) "tray=$up cmd=$($d.CommandLine)"
    }
    if (-not $up) {
        Start-Process explorer.exe "shell:AppsFolder\$pfn!Rustshot"
        $up = WaitFor { [W]::TrayWindow() } 10
        $d = V67Procs | Select-Object -First 1
    }
    V67Desktop; [V67]::WinShiftS()
    $ov = WaitFor { [W]::Overlay() -ne [IntPtr]::Zero } 8
    $owner = if ($ov) { [W]::Owner([W]::Overlay()) } else { 0 }
    if ($ov) { Shot 'v067-trayon-winshifts' }
    Check 'v067 msix tray on: Win+Shift+S captures in the running daemon' ($ov -and $d -and $owner -eq $d.ProcessId) "overlay=$ov owner=$owner daemon=$($d.ProcessId)"
    KillAll

    # ------------------------------------------------ Start at login in Rustshot's Settings (the startup task)
    $h = V67OpenSettings $alias $focus
    $reached = $false
    if ($h -ne [IntPtr]::Zero) { $reached = V67ToggleLaunch $h $focus 'off'; V67Ok $h }
    $s1 = Task066 $pfn 'Rustshot' 'list' 'v067-off'
    Check 'v067 msix: Launch at login off in Settings turns the startup task off (the API reads Disabled)' ($reached -and $s1 -match 'task RustshotDaemon Disabled' -and $s1 -notmatch 'DisabledBy') "reached=$reached $s1"
    KillAll
    $h = V67OpenSettings $alias $focus
    $reached = $false
    if ($h -ne [IntPtr]::Zero) { $reached = V67ToggleLaunch $h $focus 'on'; V67Ok $h }
    $s2 = Task066 $pfn 'Rustshot' 'list' 'v067-on'
    Check 'v067 msix: and on again (the API reads Enabled)' ($reached -and $s2 -match 'task RustshotDaemon Enabled') "reached=$reached $s2"
    KillAll
    # Off by the user's hand in Windows: the state Settings > Apps > Startup writes.
    $k = "HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData\$pfn\RustshotDaemon"
    $old = (Get-ItemProperty $k -ErrorAction SilentlyContinue).State
    if ($null -eq $old) {
        Add-Content $log "SKIP  v067 msix: Launch at login turned off in Windows (no startup state at ${k}: $(StartupReg066 $pfn))"
    } else {
        Set-ItemProperty $k -Name State -Value 1 -Type DWord
        $s3 = Task066 $pfn 'Rustshot' 'list' 'v067-by-user'
        $h = V67OpenSettings $alias $focus
        $reached = $false
        if ($h -ne [IntPtr]::Zero) { $reached = V67ToggleLaunch $h $focus 'by-user' }
        $s4 = Task066 $pfn 'Rustshot' 'list' 'v067-by-user-after'
        Check 'v067 msix: turned off in Windows, the API reads DisabledByUser and turning it on in Settings leaves it so (the steps show: out\v067-startup-by-user.png)' ($s3 -match 'task RustshotDaemon DisabledByUser' -and $reached -and $s4 -match 'task RustshotDaemon DisabledByUser') "was=$old before=$s3 reached=$reached after=$s4"
        if ($h -ne [IntPtr]::Zero) { [V3]::PostKey($h, 0x0D); Start-Sleep 1; [void][W]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
        Set-ItemProperty $k -Name State -Value $old -Type DWord
        KillAll
    }
    Remove-Item "$env:APPDATA\rustshot\config.toml", "$env:LOCALAPPDATA\Packages\$pfn\LocalCache\Roaming\rustshot\config.toml" -ErrorAction SilentlyContinue
}
