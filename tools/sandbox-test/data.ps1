# Data files (0.5.0), dot-sourced by test.ps1 and run on the portable exe:
# the blur model is not in any package; `rustshot data` names the file to
# download, from where, and where to save it; with no network (the
# sandbox) `data --fetch` says it couldn't download and leaves no partial
# file; a model put in the cache by hand is verified ("SHA-256 verified"),
# and a damaged one is reported as damaged with downloads off.
# Isolated instance: own RUSTSHOT_INSTANCE, APPDATA and LOCALAPPDATA. The
# good model comes from <T>\data\selfie.bin (run.ps1 copies the repo's).

function ChecksData($exe) {
    $root = "$env:TEMP\rs-data"
    if (Test-Path $root) { Remove-Item -Recurse -Force $root }
    $appdata = "$root\Roaming"; $local = "$root\Local"
    New-Item -ItemType Directory -Force "$appdata\rustshot", $local | Out-Null
    $saved = @{ I = $env:RUSTSHOT_INSTANCE; A = $env:APPDATA; L = $env:LOCALAPPDATA }
    $env:RUSTSHOT_INSTANCE = 'sandbox-data'; $env:APPDATA = $appdata; $env:LOCALAPPDATA = $local
    try {
        Set-Content "$appdata\rustshot\config.toml" "capture_hotkey = `"Ctrl+Alt+Shift+F11`"`nquit_hotkey = `"Ctrl+Alt+Shift+F10`"`ncheck_updates = false"
        $model = "$local\rustshot-data\models\selfie.bin"
        $o = & $exe data 2>&1 | Out-String
        Add-Content $log "      rustshot data: $($o.Trim())"
        Check 'data: the model is not here at first, and the line says where to get it and where it goes' ($o -match 'selfie-[0-9a-f]{8}\.bin: not here; download https://github\.com/nappsllc/rustshot/releases/download/v[\d.]+/selfie-[0-9a-f]{8}\.bin and save it as ' -and $o.Contains($model)) ''
        $o = & $exe data --fetch 2>&1 | Out-String
        $rc = $LASTEXITCODE
        Add-Content $log "      rustshot data --fetch: $($o.Trim())"
        Check "data: offline, --fetch says it couldn't download (exit 1)" ($rc -eq 1 -and $o -match "couldn't download") "exit=$rc"
        $parts = @(Get-ChildItem "$local\rustshot-data" -Recurse -File -ErrorAction SilentlyContinue | Where-Object Name -like '*.part')
        Check 'data: no model and no partial file after the failed download' (-not (Test-Path $model) -and $parts.Count -eq 0) "parts=$($parts.Count)"
        # The good model put there by hand.
        if (Test-Path "$T\data\selfie.bin") {
            New-Item -ItemType Directory -Force (Split-Path $model) | Out-Null
            Copy-Item "$T\data\selfie.bin" $model
            $o = & $exe data 2>&1 | Out-String
            Check 'data: a model put in the cache by hand is verified' ($o -match 'SHA-256 verified') $o.Trim()
            # Damaged, downloads off: said so.
            Add-Content "$appdata\rustshot\config.toml" 'download_data = false'
            $b = [IO.File]::ReadAllBytes($model); $b[1000] = $b[1000] -bxor 1; [IO.File]::WriteAllBytes($model, $b)
            $o = & $exe data --fetch 2>&1 | Out-String
            Check 'data: a damaged model with downloads off is reported as damaged' ($o -match 'is damaged' -and $o -match 'downloads are off') $o.Trim()
        } else {
            Add-Content $log 'SKIP  data: verified / damaged model (no <T>\data\selfie.bin)'
        }
    } finally {
        $env:RUSTSHOT_INSTANCE = $saved.I; $env:APPDATA = $saved.A; $env:LOCALAPPDATA = $saved.L
    }
}

# No package carries data folders (they are downloaded on demand).
function NoDataFolders($dir, $what) {
    Check "${what}: no models\ or lang\ folder installed" (-not (Test-Path "$dir\models") -and -not (Test-Path "$dir\lang")) $dir
}
