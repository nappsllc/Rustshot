# Start menu / desktop shortcut names (dot-sourced by test.ps1).
# 0.1.0 made "Programs\rustshot\rustshot.lnk" (lowercase, empty icon); Windows
# paths ignore case, so a newer setup writing "Rustshot\Rustshot.lnk" over
# it used to keep the old names and Start showed "rustshot". The setup now
# removes the old shortcuts first and makes them fresh.

$StartDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"

function NewLnk($path, $target, $arguments = '', $icon = '') {
    $sh = New-Object -ComObject WScript.Shell
    $l = $sh.CreateShortcut($path)
    $l.TargetPath = $target
    $l.Arguments = $arguments
    if ($icon) { $l.IconLocation = $icon }
    $l.Save()
}

function ReadLnk($path) {
    (New-Object -ComObject WScript.Shell).CreateShortcut($path)
}

# Before the install: what 0.1.0 left (lowercase folder and names, no icon),
# and an old lowercase desktop shortcut.
function OldShortcuts($exe) {
    New-Item -ItemType Directory -Force "$StartDir\rustshot" | Out-Null
    NewLnk "$StartDir\rustshot\rustshot.lnk" $exe
    NewLnk "$StartDir\rustshot\Uninstall.lnk" ($exe -replace 'rustshot\.exe$', 'uninstall.exe')
    NewLnk "$env:USERPROFILE\Desktop\rustshot.lnk" $exe
}

# The exact-case name of the single entry of $dir matching $name (any case).
function ExactName($dir, $name) {
    $e = Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -ieq $name } | Select-Object -First 1
    if ($e) { $e.Name } else { '' }
}

# After the install over OldShortcuts: capitalized folder and names, icons
# pointing at the exe, the Settings shortcut, the desktop one kept.
function ShortcutChecks($exe) {
    $folder = ExactName $StartDir 'rustshot'
    Check 'shortcuts: Start menu folder is "Rustshot" (exact case)' ($folder -ceq 'Rustshot') "found '$folder'"
    $dir = "$StartDir\Rustshot"
    $main = ExactName $dir 'rustshot.lnk'
    Check 'shortcuts: "Rustshot.lnk" (exact case)' ($main -ceq 'Rustshot.lnk') "found '$main'"
    $l = ReadLnk "$dir\Rustshot.lnk"
    Check 'shortcuts: Rustshot.lnk targets the exe' ($l.TargetPath -ieq $exe) $l.TargetPath
    Check 'shortcuts: Rustshot.lnk icon is the exe' ($l.IconLocation -ieq "$exe,0") $l.IconLocation
    $s = ReadLnk "$dir\Rustshot Settings.lnk"
    Check 'shortcuts: "Rustshot Settings.lnk" runs rustshot.exe settings' ((Test-Path "$dir\Rustshot Settings.lnk") -and $s.TargetPath -ieq $exe -and $s.Arguments -eq 'settings') "$($s.TargetPath) $($s.Arguments)"
    Check 'shortcuts: Rustshot Settings.lnk icon is the exe' ($s.IconLocation -ieq "$exe,0") $s.IconLocation
    $u = ExactName $dir 'uninstall.lnk'
    Check 'shortcuts: Uninstall.lnk' ($u -ceq 'Uninstall.lnk') "found '$u'"
    $d = ExactName "$env:USERPROFILE\Desktop" 'rustshot.lnk'
    Check 'shortcuts: the old desktop shortcut is kept as "Rustshot.lnk"' ($d -ceq 'Rustshot.lnk') "found '$d'"
    $dl = ReadLnk "$env:USERPROFILE\Desktop\Rustshot.lnk"
    Check 'shortcuts: desktop icon is the exe' ($dl.IconLocation -ieq "$exe,0") $dl.IconLocation
}

# After the uninstall: no shortcut of any version is left.
function ShortcutsGone {
    Check 'shortcuts: uninstall removes the Start menu folder (any case)' (-not (ExactName $StartDir 'rustshot')) (ExactName $StartDir 'rustshot')
    Check 'shortcuts: uninstall removes the desktop shortcut' (-not (ExactName "$env:USERPROFILE\Desktop" 'rustshot.lnk'))
}
