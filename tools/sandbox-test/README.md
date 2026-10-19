# Clean-system test (Windows Sandbox)

Installs and drives the CI's Windows builds on a fresh Windows (Windows
Sandbox: no network, no vGPU, no clipboard sharing) and writes a
`PASS` / `FAIL` / `SKIP` line per check. It is run before each release.

Needs Windows 11 Pro/Enterprise with the *Windows Sandbox* feature, `gh`
(for `-RunId`) and, for the MSIX checks, `signtool.exe` from the Windows SDK.

```powershell
# The Windows artifacts of a CI run, plus the last release's installer for the upgrade checks:
gh release download v0.3.0 -R nappsllc/rustshot -p 'rustshot-0.3.0-setup.exe' -D $env:TEMP\rs-old
.\tools\sandbox-test\run.ps1 -RunId <ci run id> -Old $env:TEMP\rs-old\rustshot-0.3.0-setup.exe

# The QR picture for the 0.5.0 Read QR check, from the preview tests:
#   $env:RUSTSHOT_PREVIEW_DIR = "$env:TEMP\rs-previews"; cargo test qr_make::tests::previews
.\tools\sandbox-test\run.ps1 -RunId <ci run id> -Old <old setup> -QrPng $env:TEMP\rs-previews\qr_make_link_light_1x.png

# Plus the last release's .msix for the MSIX update check (signed with the same throwaway certificate):
gh release download v0.5.1 -R nappsllc/rustshot -p 'rustshot-0.5.1-x64.msix' -D $env:TEMP\rs-old
.\tools\sandbox-test\run.ps1 -RunId <ci run id> -Old <old setup> -OldMsix $env:TEMP\rs-old\rustshot-0.5.1-x64.msix

# Already downloaded artifacts; only one group of checks:
.\tools\sandbox-test\run.ps1 -Artifacts D:\ci -Old D:\rustshot-0.3.0-setup.exe -Only v040
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
| `test.ps1`    | NSIS silent install over 0.1.0's lowercase `rustshot\rustshot.lnk` + desktop shortcut (`shortcuts.ps1`: the folder and shortcuts come back as `Rustshot`, `Rustshot.lnk`, `Rustshot Settings.lnk` (`settings`), icons pointing at the exe, all gone after the uninstall), files, Start menu, Run value, version resource and icon; the first launch is a daemon (tray, no overlay) holding the default Win+Shift hotkeys, which open the overlay as real key presses; a second launch captures; Settings in the daemon; saving into `Pictures\rustshot\<date>` (CLI and Ctrl+S); uninstall (History kept); upgrade from `old\` through `setup /S /RELAUNCH` (relaunch, config and History kept); the sideload-signed MSIX (`msix.ps1`: with `-OldMsix`, installed over the older release's package as an in-place update; one manifest language per `lang\<tag>.txt` with `en-us` first; LICENSE, NOTICE and THIRD_PARTY_NOTICES.md in the install folder; Start entry, alias, startup task); the portable exe's own Settings window; then the groups below. |
| `probe.ps1`   | Which Win+Shift+<key> chords are free on a clean system. |
| `uninst.ps1`  | The Apps & features (Uninstall) entry: name, publisher PowerNapps, version, icon with DIB images, size, links. |
| `history.ps1` | History in an isolated instance: entries from direct and daemon captures, `rustshot history` and its hotkey, `history = false`, `history --list`. |
| `rec.ps1`     | 2 s MP4 and GIF recordings through the record hotkey, checked by box layout, Media Foundation and GIF structure. |
| `v030.ps1`    | 0.3.0: a delayed region capture from the CLI, `--accept-on-select` with a region, `rustshot launcher` (alone and in the daemon) and a Screen / 3 s / Save launch with its countdown pill, Open with (Ctrl+O) writing into a private `%TEMP%\rustshot-open`, the new config keys after a Settings save, and no global Ctrl+O. |
| `data.ps1`    | Data files (downloaded on demand, in no package): no `models\` or `lang\` beside the installed exe; `rustshot data` names the model's release URL and where it goes; offline `data --fetch` says it couldn't download (exit 1) and leaves no partial file; the repo's model put into the isolated cache by hand is "SHA-256 verified"; a damaged one with `download_data = false` is reported as damaged. |
| `v040.ps1`    | 0.4.0: the tray icon off (a 0.3.0 config with `show_tray = false` gets the one-time notice and `tray_off_mode_seen`; then `rustshot daemon` exits, a plain launch opens Settings, `daemon --force` runs), `rustshot screenclip "ms-screenclip:?source=HotKey"` with no daemon (an on-demand capture holding no global hotkeys, ended by an Esc posted to its overlay), `register-screenclip on` / `off` (inside the sandbox only: the ProgID, Capabilities and RegisteredApplications values, gone again), the export border, image edits (crop, flip, rotate, canvas) from an `annotations.json` fixture rendered by History's Open with, a freeform lasso snip saved with a transparent outside, Text mode (O, Ctrl+A, Ctrl+C) and Quick Redact (Shift+R, solid) on a window showing an e-mail address and an IP. |
| `v041.ps1`    | 0.4.1: the exe's 16 and 32 px icons as `ExtractIconEx` returns them (what Task Manager and Explorer show; the small entries are DIBs) are not blank (saved as `out\v041-icon-*.png`), and a `full --region` PNG from the in-repo encoder decodes in GDI+ and WIC at its size, with the same pixels in both and as a BMP of the same region (sizes against GDI+'s own PNG in the log). |
| `v050.ps1`    | 0.5.0: `rustshot data` lists every `lang-<tag>.txt` of this version's release; with no language set Settings opens in English; Settings › General's Language list is reached by Tab (`RUSTSHOT_TEST_FOCUS_FILE`) and opens with Enter; picking a language with no network says it couldn't download it (red, Retry) and stays in English, leaving no file; `language = "de"` offline starts in English and `data --fetch` says lang-de.txt couldn't download; the repo's `de.txt` put in the cache by hand is SHA-256 verified and opens Settings in German, `ar.txt` in Arabic (screenshot for the mirrored layout); the ruler (U and a drag) in the overlay; Read QR code (Q) in a one-shot capture of a window showing `-QrPng` prints its link and, with no code, "No QR code found"; Scrolling capture (Shift+S) on a long text box scrolls it and saves a stitched PNG taller than the selection. |
| `v060.ps1`    | 0.6.0: the exe holds the `https://powernapps.net/r/rustshot` short link and Settings › About (the footer's Apache-2.0 on its screenshot) is reached by Tab (the rail) and End; Enter on More from PowerNapps' powernapps.net link opens that link (the Website link went in 0.6.5) (the browser the sandbox starts for it, found by its command line; no network). The NSIS install and the MSIX (`msix.ps1`) carry LICENSE (Apache-2.0), NOTICE and THIRD_PARTY_NOTICES.md. |
| `v062.ps1`    | 0.6.2: the tray icon is the app icon drawn by Rustshot: a daemon of an isolated instance makes its tray window and its notification-area icon (`NIM_MODIFY` on its hWnd succeeds), its stderr has no "could not draw the tray icon" (the fallback to the exe's icon) or "could not add the notification-area icon", and the taskbar's button for it (UI Automation; SKIP when it is in the overflow) shows the blue tile and white glyph (`out\v062-tray-icon.png`). |
| `v063.ps1`    | 0.6.3: the exe's 16 px icon (`ExtractIconEx`'s small entry) is the pixel-aligned frame `packaging/icons/small/rustshot-16.png` and not 0.6.2's (read from git by `run.ps1`; SKIP without it), as is the Settings window's small title-bar icon (its class's `GCLP_HICONSM`) when it is 16 px (`out\v063-*.png`); `rustshot settings` and `rustshot history` open windows titled "Rustshot — Settings" and "Rustshot — History"; E on History's one entry opens the Edit window "v063-shot.png — Rustshot", and a stroke drawn in it makes it "*v063-shot.png — Rustshot"; on screen, the Settings window's caption is Rustshot's theme surface and its 1 px border (the outer pixels of `DWMWA_EXTENDED_FRAME_BOUNDS`) the caption colour (`out\v063-settings-frame.png`; the DWM colours can't be read back). |
| `v064.ps1`    | 0.6.4: a window of the test with `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` (topmost, magenta) is the "Hidden from screenshots" placeholder in a full-screen `rustshot full --path` (the dark theme's panel near its edges, the label's ink in its middle; `out\v064-hidden-on.png`), and with `mark_hidden_windows = false` the capture shows what is behind it (no panel, no magenta); SKIP when the affinity can't be set. The Settings window's client area is 680 x 552 logical px (`GetClientRect` on a per-monitor-aware thread over `GetDpiForWindow`). From the rail (posted Tab, End, Up), Tab reaches a row on each Shortcuts page: Edit window (`key_zoom_in`), Overlay actions (`key_accept`), Overlay editing (`key_undo`), Overlay tools (`key_tool_text`), Recording (`key_rec_mute`), Global (`global3`); and Capture's `mark_hidden_windows` switch (`out\v064-keys-*.png`, `out\v064-capture.png`). |
| `v065.ps1`    | 0.6.5: Settings > About has no links row: from the rail (posted Tab, End), a Tab round the page reaches `link_powernapps` and none of `link_0` .. `link_5` (Website, GitHub, Report an issue, What's new, Licenses; `out\v065-about.png`). Several hotkeys in use by another app: the test's PowerShell holds 4, then 8 of Ctrl+Alt+Shift+F1 .. F8 (`RegisterHotKey`), and a daemon whose slots want them opens Settings with the hotkey summary dialog; Tab reaches its OK and Change shortcuts, and its card (in `PrintWindow` pictures: the pixels that differ from the page after OK under the dark scrim, connected to the middle) lies inside the client area, centred, the 8-failure one ("and N more") taller (`out\v065-hotkeys-*.png`). The summary's lines in every language are left to the unit fit tests. |

The checks of the portable exe use their own `RUSTSHOT_INSTANCE`, `APPDATA`
and `LOCALAPPDATA` and unusual hotkeys. `-Only probe|rec|v030|v040|v041|v050|v060|v062|v063|v064|v065|data|msix`
runs one group.
