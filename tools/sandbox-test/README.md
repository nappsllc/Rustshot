# Clean-system test (Windows Sandbox)

Installs and drives the CI's Windows builds on a fresh Windows (Windows
Sandbox: no network, no vGPU, no clipboard sharing) and writes a
`PASS` / `FAIL` / `SKIP` line per check. It is run before each release.

Needs Windows 11 Pro/Enterprise with the *Windows Sandbox* feature, `gh`
(for `-RunId`) and, for the MSIX checks, `signtool.exe` from the Windows SDK.

```powershell
# The Windows artifacts of a CI run, plus the last release's installer for the upgrade checks:
gh release download v0.2.0 -R nappsllc/rustshot -p 'rustshot-0.2.0-setup.exe' -D $env:TEMP\rs-old
.\tools\sandbox-test\run.ps1 -RunId <ci run id> -Old $env:TEMP\rs-old\rustshot-0.2.0-setup.exe

# Already downloaded artifacts; only one group of checks:
.\tools\sandbox-test\run.ps1 -Artifacts D:\ci -Old D:\rustshot-0.2.0-setup.exe -Only v030
```

`run.ps1` makes a work folder (default `%TEMP%\rustshot-sandbox`, emptied
first) with the scripts, `in\` (the artifacts), `old\` (the older setup) and
`rustshot-test.wsb`, signs a copy of the MSIX for sideloading
(`sign-sideload.ps1`: a throwaway self-signed certificate with a random
password, deleted again; only its public `sideload.cer` goes into the
sandbox) and opens the sandbox. At logon the sandbox runs `test.ps1`; the
results are in `<work>\out\results.txt` (the last line is `DONE`), with
screenshots and logs beside it. Close the sandbox window when done (it
discards everything inside it).

| script        | what it checks |
|---------------|----------------|
| `test.ps1`    | NSIS silent install over 0.1.0's lowercase `rustshot\rustshot.lnk` + desktop shortcut (`shortcuts.ps1`: the folder and shortcuts come back as `Rustshot`, `Rustshot.lnk`, `Rustshot Settings.lnk` (`settings`), icons pointing at the exe, all gone after the uninstall), files, Start menu, Run value, version resource and icon; the first launch is a daemon (tray, no overlay) holding the default Win+Shift hotkeys, which open the overlay as real key presses; a second launch captures; Settings in the daemon; saving into `Pictures\rustshot\<date>` (CLI and Ctrl+S); uninstall (History kept); upgrade from `old\` through `setup /S /RELAUNCH` (relaunch, config and History kept); the sideload-signed MSIX (Start entry, alias, startup task); the portable exe's own Settings window; then the groups below. |
| `probe.ps1`   | Which Win+Shift+<key> chords are free on a clean system. |
| `uninst.ps1`  | The Apps & features (Uninstall) entry: name, publisher PowerNapps, version, icon with DIB images, size, links. |
| `history.ps1` | History in an isolated instance: entries from direct and daemon captures, `rustshot history` and its hotkey, `history = false`, `history --list`. |
| `rec.ps1`     | 2 s MP4 and GIF recordings through the record hotkey, checked by box layout, Media Foundation and GIF structure. |
| `v030.ps1`    | 0.3.0: a delayed region capture from the CLI, `--accept-on-select` with a region, `rustshot launcher` (alone and in the daemon) and a Screen / 3 s / Save launch with its countdown pill, Open with (Ctrl+O) writing into a private `%TEMP%\rustshot-open`, the new config keys after a Settings save, and no global Ctrl+O. |

The checks of the portable exe use their own `RUSTSHOT_INSTANCE`, `APPDATA`
and `LOCALAPPDATA` and unusual hotkeys. `-Only probe|rec|v030` runs one
group.
