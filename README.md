<p align="center">
  <img src="packaging/icons/rustshot-128.png" alt="Rustshot" width="128" height="128">
</p>

<h1 align="center">Rustshot</h1>

[![Rust](https://img.shields.io/badge/Rust-2024%20edition-000000?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![release](https://img.shields.io/github/v/release/nappsllc/rustshot?sort=semver&style=flat-square&color=007ec6&label=release)](https://github.com/nappsllc/rustshot/releases/latest)
[![downloads](https://img.shields.io/github/downloads/nappsllc/rustshot/total?style=flat-square&color=44cc11&logo=github&label=Downloads)](https://github.com/nappsllc/rustshot/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/nappsllc/rustshot/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/nappsllc/rustshot/actions/workflows/ci.yml)
[![tests](https://img.shields.io/badge/tests-1500%2B-brightgreen?style=flat-square)](https://github.com/nappsllc/rustshot/actions/workflows/ci.yml)
[![coverage](https://img.shields.io/codecov/c/github/nappsllc/rustshot?style=flat-square&logo=codecov&logoColor=white)](https://codecov.io/gh/nappsllc/rustshot)
[![languages](https://img.shields.io/badge/languages-35-blueviolet?style=flat-square)](i18n/languages.md)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square)](LICENSE)
[![Buy me a coffee](https://raw.githubusercontent.com/pachadotdev/buymeacoffee-badges/main/bmc-yellow.svg)](https://buymeacoffee.com/denis.platonov)

**Tiny yet powerful, beautiful, open-source and cross-platform tool for
screenshots, editing and video capture.** Windows, Linux and macOS.

- **Under 3 MB, one file.** A single native executable with no runtime,
  no GUI toolkit and no web view. Every pixel of the overlay is drawn by
  Rustshot's own anti-aliased renderer. Language files and the camera's
  blur model are downloaded only when you need them.
- **Zero memory when you're not capturing.** Turn the tray icon off and
  nothing runs in the background at all: **0 MB**. Rustshot replaces the
  Snipping Tool: Win+Shift+S or Print Screen starts Rustshot for the
  capture, and it quits when you're done.
  With the tray icon on, Rustshot uses about 1.9 MB of memory in the
  background, and about 4.1 MB with the capture overlay open on a
  5120 × 1440 desktop (as shown in Task Manager). Measured on Windows 11,
  release build.
- **Smooth.** On Windows the overlay repaints only the pixels that changed,
  through GDI dirty rectangles, and draws nothing while it sits idle.
  At 3840 × 2160, drawing a selection with the pixel magnifier takes about
  1.7 ms per mouse move (0.6 ms without it), and moving a rectangle about
  1 ms. A 60 fps frame is 16 ms.
- **Good-looking.** Dark and light themes follow your OS, and you can pick
  any accent color. The UI is built from one set of design tokens, with
  the Inter font, Lucide icons, soft shadows and short animations.
- **In your language.** 35 languages, with right-to-left layouts for
  Arabic and Hebrew.

![Rustshot's overlay in the dark theme: a selection with a rectangle and an arrow, the toolbar and the color palette](docs/screenshots/overlay-dark.png)

| Light theme with a text annotation | Video mode |
|---|---|
| ![Light theme](docs/screenshots/overlay-light.png) | ![Video mode panel](docs/screenshots/video-mode.png) |

<sub>Rendered by Rustshot's own preview tests over a sample window.</sub>

## Features

**Capture**
- Region, full screen, a single monitor or a window. Hover a window to
  outline it, then click to take exactly that window, menu or tooltip.
  <kbd>Tab</kbd> steps into its panels and fields (*Snap to windows*,
  Settings › Capture).
- Resize handles, arrow-key nudging and a live `W × H` readout.
- *Freeform*: draw a lasso instead of a rectangle
  (<kbd>Shift</kbd>+<kbd>F</kbd>, or the lasso by the mode switch).
  Outside it the PNG is transparent (white in a JPG or BMP).
- *Scrolling capture*: <kbd>Shift</kbd>+<kbd>S</kbd> (or the `W × H`
  readout's menu) scrolls the window under the selection and stitches it
  into one tall picture (up to 20 000 px; sticky headers and footers come
  once), which opens in the Edit window. A pill shows the height so far;
  Stop or <kbd>Esc</kbd> ends it early. Windows and X11 (not Wayland,
  where an app cannot scroll another's window, nor macOS yet).
- The launcher (the tray's **Capture…**): choose what to capture, a delay
  of 0 to 60 s, and what happens next: edit, copy, save or pin. A
  countdown pill shows the delay and is never in the capture.
- *Accept on select*: copy or save the moment you release the mouse.
- A pixel magnifier shows the position and color under the pointer.
  <kbd>H</kbd> copies the color as `#rrggbb`. Its crosshair replaces the
  system pointer while it shows (no lag between the two), with the exact
  pixel left open at its center.
- *Include mouse pointer* (Settings › Capture): the pointer is captured
  too, as an object you can move or delete (Windows and X11).
- *Trim to content*: <kbd>Shift</kbd>+<kbd>A</kbd> (or the `W × H`
  readout) cuts the uniform margins off the selection. Settings can trim
  every new region or window for you.
- Works across multiple monitors. The UI follows each display's scaling.
- On Windows, HDR displays are tone-mapped to SDR, so screenshots and
  recordings look as they do on screen instead of washed out.

**Annotate & edit**
- Pencil, line, arrow, rectangle, ellipse, marker, text, callout,
  numbered tags, pixelate, blur, invert, spotlight and magnifier.
- A screen ruler (<kbd>U</kbd>): drag to measure a length in px (and
  logical px on a scaled display) with Δx, Δy and the angle;
  <kbd>Shift</kbd> snaps to 15°, <kbd>Ctrl</kbd> measures a rectangle
  (W × H and area). <kbd>Enter</kbd> keeps it as a dimension line,
  <kbd>Esc</kbd> drops it.
- Text styles: sans, serif or mono font, bold, italic, underline,
  strikethrough and alignment.
- Quick Redact (<kbd>Shift</kbd>+<kbd>R</kbd>): e-mail addresses, phone
  and card numbers, IBANs, IP addresses, URLs with tokens, keys, passwords
  and JWTs are covered with an opaque fill in the surrounding color, all in
  one undo step (Settings › Overlay › Redact style: Solid, or Blur).
  Long-press Copy text to choose which kinds.
- Crop (<kbd>C</kbd>): a frame inside the selection, or inside the
  picture in the Edit window, where the crop can be undone or widened
  again later.
- Annotations stay editable objects. Select, move, resize, restyle,
  restack and duplicate them, with snapping and unlimited undo. With any
  drawing tool, point at an object and drag it to move it or click it to
  select it; hold <kbd>Ctrl</kbd> to draw over it instead.
- Right-click opens a color ring.
- The Edit window (from History or the thumbnail) zooms the picture and
  edits it, all undoable and non-destructive: rotate, flip, resize
  (Lanczos-3 or area average, with a file size estimate) and a canvas
  (padding, a solid color or a soft "presentation" gradient, rounded
  corners, a drop shadow). Text stays upright when the picture turns.
- Effects (Image › Effects…): a watermark (text or a PNG, at one of nine
  places or tiled, with its size, opacity and margin), a drop shadow, torn
  edges and a reflection.

**Copy text (OCR)**
- Copy the text from any area. Rustshot uses the OCR built into Windows
  and macOS, or `tesseract` on Linux. Nothing is uploaded.
- Text mode: Copy text (<kbd>O</kbd>) shows the words it found. Drag
  across them to select (double-click a word, triple-click a line,
  <kbd>Ctrl</kbd>+<kbd>A</kbd> all), then <kbd>Ctrl</kbd>+<kbd>C</kbd> or
  **Copy**; with nothing selected <kbd>Ctrl</kbd>+<kbd>C</kbd> copies all
  the text, and **Copy all** does too. <kbd>Esc</kbd> leaves Text mode.
  The Copy text hotkey (<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd>)
  copies all at once.

**QR codes**
- Read QR code (<kbd>Q</kbd>, or long-press Copy text): every QR code in
  the area is decoded on your machine; the toast shows the text with
  **Copy** and, for an http(s) link, **Open link**. Nothing opens until
  you click it.
- Make QR code… (the tray, the launcher): type or paste text and copy,
  save or pin the code.

**Record**
- Record a region, a window or a screen to MP4 (30 or 60 fps) or GIF.
- System sound and your microphone (MP4).
- A camera bubble you can drag and resize: a circle, rounded or square,
  with a ring. Its background can be blurred (light or strong); the person
  is found on your computer, and no picture leaves it.
- A 3-2-1 countdown, and pause, mute and discard while recording.
- *Show clicks & keys* (Settings › Recording, off by default): a ring
  pulses where you click and the last shortcuts show at the bottom
  ("Ctrl + Shift + S"), in the video only. Only chords with
  <kbd>Ctrl</kbd>, <kbd>Alt</kbd> or <kbd>Win</kbd> by default (typed text
  never shows); *All keys* shows every key, but on Windows never while a
  password field has (or may have) the focus. It can still show passwords
  typed into terminals, remote desktops, games and some apps, and on Linux
  none are detected. Windows and X11.
- *Trim after recording* (Settings › Recording › Ask): after Stop, cut the
  start and end on a timeline of thumbnails with a preview, then **Save
  trimmed**. MP4s are cut without re-encoding; GIFs by frames.

**Share**
- Copy to the clipboard, or save as PNG, JPEG or BMP into dated folders
  with your own file-name pattern.
- Save As, Save to Desktop, Pin, **Open with…** any app, **Share…**
  through the system's share sheet (Windows, macOS), or (on Windows) drag
  the thumbnail into another app.
- Pin a capture on top of every window. Zoom it, fade it and compare
  against it.
- Upload to Imgur, or to your own S3-compatible bucket (Amazon S3,
  Cloudflare R2, MinIO, …), with the link copied for you. Images and MP4
  recordings can be uploaded, and you can delete an upload later
  ([Uploads](docs/guide.md#uploads)).
- An optional border (any color, 1 to 8 px) around everything you copy,
  save, upload or pin, and an optional watermark (Settings › Saving).
- *Save at*: screen pixels, 1× (a high-DPI capture at its logical size) or
  a percent, for everything you copy, save or upload. Pins and History keep
  the full capture.

**History**
- Every capture is kept for 30 days (up to 2 GB) with its annotations
  still editable.
- Search and filter your captures, then reopen one in the Edit window.

**Languages**
- 35 languages. English is built in; the other 34 are machine
  translations ([i18n/languages.md](i18n/languages.md); corrections are
  welcome). *System default* follows your OS, or pick one in Settings ›
  General › Language.
- Only the language you use is downloaded (one file of 50 to 90 KB),
  when you pick it or on first run when your system language has a
  translation. Arabic and Hebrew get a right-to-left layout.

**Settings and the rest**
- A Settings window in 15 short sections that never scroll: General,
  Capture, Overlay, Saving, File names, History, Recording, Camera,
  Upload, S3 storage, four Shortcuts sections (Global, Recording, Overlay
  tools, Overlay actions) and About.
- Global hotkeys and every editor key can be remapped. On Windows,
  Rustshot can take over <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> and
  <kbd>Print Screen</kbd>.
- Tray icon off (Windows): nothing runs in the background; Win+Shift+S
  and Print Screen start Rustshot for a capture, and launching it opens
  Settings.
- In-app updates (direct downloads), checked against the release's
  `SHA256SUMS`.
- A scriptable [command line](#command-line).

## Install

Download from the **[latest release](https://github.com/nappsllc/Rustshot/releases/latest)**
(v0.5.1 sizes shown):

| Platform | File | Size | Notes |
|---|---|---|---|
| Windows 10/11 | `rustshot-<v>-setup.exe` | 1.3 MB | Per-user installer, no admin. Start menu *Rustshot* and *Rustshot Settings*; optional desktop shortcut and start with Windows. Installing over an older version replaces its shortcuts (0.1.0 named them in lowercase). |
| Windows (portable) | `rustshot-<v>-windows-x86_64.exe` | 2.8 MB | The whole app. Run it from anywhere. |
| Windows (Store package) | `rustshot-<v>-x64.msix` | 1.5 MB | The unsigned Microsoft Store upload package. You can't install it directly; use the installer or the portable exe. |
| Debian / Ubuntu | `rustshot-<v>-amd64.deb` | 1.6 MB | `sudo apt install ./rustshot-<v>-amd64.deb` |
| Any Linux | `rustshot-<v>-x86_64.AppImage` | 2.6 MB | `chmod +x`, then run it. |
| Any Linux | `rustshot-<v>-linux-x86_64.tar.gz` | 1.8 MB | The binary and a desktop file. See `INSTALL.txt` inside. |
| Any Linux | `rustshot.flatpak` | 1.5 MB | `flatpak install --user rustshot.flatpak` |
| Any Linux | `rustshot_<v>_amd64.snap` | 9.4 MB | `sudo snap install --dangerous rustshot_<v>_amd64.snap` |
| macOS 11+ | `rustshot-<v>-macos-universal.dmg` | 2.8 MB | Universal (Apple Silicon and Intel). Drag it to Applications. |
| macOS 11+ | `rustshot-<v>-macos-universal.zip` | 2.3 MB | The same `Rustshot.app`, zipped. |

The release also carries `SHA256SUMS`, the language files
(`lang-<tag>.txt`, 50 to 90 KB each) and the camera's background-blur
model (`selfie-<hash8>.bin`, 215 KB). You don't download those yourself:
Rustshot fetches one language file when it needs it, and the blur model
only when you turn a camera blur on (macOS 12+ blurs with Vision and
needs no model). Each is checked against the SHA-256 built into
Rustshot. Settings › General › *Download language and blur files*
(`download_data = false`) turns this off; Settings then says which file
to put where, and `rustshot data` lists them.

> **Unsigned builds.** Releases are not code-signed yet. On Windows,
> SmartScreen may warn on first run: choose *More info* › *Run anyway*. On
> macOS, Gatekeeper blocks the first launch ("Rustshot Not Opened"). Move
> `Rustshot.app` to Applications, try to open it once, then open **System
> Settings › Privacy & Security**, scroll down and click **Open Anyway**.
> If that button doesn't appear, run this once in Terminal and open the app
> again:
>
> ```bash
> xattr -dr com.apple.quarantine /Applications/Rustshot.app
> ```

Store and package-manager listings (Microsoft Store, winget, Mac App Store,
Homebrew, Flathub, Snap Store, AUR) are being set up. See
[docs/STORES.md](docs/STORES.md).

## Quick start

1. Run `rustshot` (or launch it from the Start menu or your app list). It
   starts in the background with a tray icon. The installers can start
   it at login.
2. Press <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>X</kbd>. Use <kbd>⌘</kbd>
   instead of <kbd>Win</kbd> on macOS and <kbd>Super</kbd> on Linux.
3. Drag a region, or click a window.
4. Annotate it, then press <kbd>Ctrl</kbd>+<kbd>C</kbd> to copy,
   <kbd>Enter</kbd> or <kbd>Ctrl</kbd>+<kbd>S</kbd> to save, or
   <kbd>Ctrl</kbd>+<kbd>U</kbd> to upload. <kbd>Esc</kbd> cancels.

If you launch Rustshot while it is already running, the running copy
starts a capture, or opens Settings when its tray icon is hidden (or a
recording runs). With the tray icon off on Windows nothing runs in the
background: launching Rustshot (or **Rustshot Settings** in the Start
menu) opens Settings. On Linux, macOS and in the Microsoft Store build
the icon off keeps Rustshot running without it. The tray menu has
**Capture…**, **Make QR code…**, **History…** and **Settings…**.

## Replace the Snipping Tool (Windows)

Windows sends <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> and
<kbd>Print Screen</kbd> to the Snipping Tool, and getting rid of that is
well hidden. Rustshot does it in three steps:

1. In Rustshot, open **Settings › Capture** and turn on **Use Win+Shift+S
   and Print Screen**.
2. Open Windows **Settings › Apps › Default apps**, scroll to the bottom
   and click **Choose defaults by link type**.
3. Find **MS-SCREENCLIP** and pick **Rustshot**.

Now both keys open Rustshot, even when it isn't running: it starts for
the capture and quits afterwards. For <kbd>Print Screen</kbd>, also keep
**Settings › Accessibility › Keyboard › Use the Print screen key to open
screen capture** on (it is on by default in Windows 11).

Rustshot walks you through it: turning the option on shows **One more step
to use Win+Shift+S** with these steps and an **Open Default apps** button,
and **Settings › Capture** says under the option which app Win+Shift+S
opens now.

Windows only lets you, not an app, make that choice, which is why step 2
can't be skipped. If Win+Shift+S shows "This file does not have an app
associated with it", the Snipping Tool was uninstalled and the old choice
points nowhere: the same three steps fix it. To go back, turn the option
off in Rustshot, and Windows uses the Snipping Tool again. More detail:
[docs/guide.md](docs/guide.md#winshifts-and-print-screen-windows).

## Shortcuts

| Global | Default |
|---|---|
| Capture a region | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>X</kbd> |
| Capture the full screen (saved at once) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> |
| Capture a window | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> |
| Start or stop recording | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd> |
| Copy text from an area | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> |
| Read QR code from an area / Repeat the last capture | (unbound; Settings › Shortcuts › Global) |
| Open History | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> |
| Quit Rustshot | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Q</kbd> |
| While recording: pause / mute the microphone / hide the pill | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> / <kbd>M</kbd> / <kbd>H</kbd> |

| In the overlay | Keys |
|---|---|
| Select / Pencil / Line / Arrow | <kbd>V</kbd> / <kbd>P</kbd> / <kbd>L</kbd> / <kbd>A</kbd> |
| Rectangle / Ellipse / Marker / Text / Callout | <kbd>R</kbd> / <kbd>E</kbd> / <kbd>M</kbd> / <kbd>T</kbd> / <kbd>K</kbd> |
| Number / Pixelate / Blur / Invert | <kbd>N</kbd> / <kbd>X</kbd> / <kbd>B</kbd> / <kbd>I</kbd> |
| Spotlight / Magnify / Ruler | <kbd>S</kbd> / <kbd>Z</kbd> / <kbd>U</kbd> |
| Stroke size − / + | <kbd>[</kbd> / <kbd>]</kbd> or the mouse wheel |
| Copy / Save / Save As / Upload | <kbd>Ctrl</kbd>+<kbd>C</kbd> / <kbd>Ctrl</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>U</kbd> |
| Save to Desktop / Open with… / Pin | <kbd>Ctrl</kbd>+<kbd>D</kbd> / <kbd>Ctrl</kbd>+<kbd>O</kbd> / <kbd>Ctrl</kbd>+<kbd>P</kbd> |
| Share… (Windows, macOS) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> |
| Copy text (OCR) / Copy color under pointer | <kbd>O</kbd> (or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd>) / <kbd>H</kbd> |
| Crop / Quick Redact / Freeform selection | <kbd>C</kbd> / <kbd>Shift</kbd>+<kbd>R</kbd> / <kbd>Shift</kbd>+<kbd>F</kbd> |
| Read QR code | <kbd>Q</kbd> |
| Text mode: copy the words / select all / leave | <kbd>Ctrl</kbd>+<kbd>C</kbd> (or <kbd>Enter</kbd>) / <kbd>Ctrl</kbd>+<kbd>A</kbd> / <kbd>Esc</kbd> |
| Text: bold / italic / underline (while typing) | <kbd>Ctrl</kbd>+<kbd>B</kbd> / <kbd>Ctrl</kbd>+<kbd>I</kbd> / <kbd>Ctrl</kbd>+<kbd>U</kbd> |
| Undo / Redo | <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> (or <kbd>Ctrl</kbd>+<kbd>Y</kbd>) |
| Move an object / draw over it (drawing tools) | drag it / <kbd>Ctrl</kbd>+drag |
| Delete / Duplicate selected objects | <kbd>Del</kbd> (or <kbd>Backspace</kbd>) / <kbd>Ctrl</kbd>+<kbd>D</kbd> |
| Bring to front / Send to back | <kbd>Ctrl</kbd>+<kbd>]</kbd> / <kbd>Ctrl</kbd>+<kbd>[</kbd> |
| Select the whole screen / Toggle the palette | <kbd>Ctrl</kbd>+<kbd>A</kbd> / <kbd>Space</kbd> |
| Trim the selection to its content | <kbd>Shift</kbd>+<kbd>A</kbd> |
| Scrolling capture of the selection | <kbd>Shift</kbd>+<kbd>S</kbd> (<kbd>Esc</kbd> stops it) |
| Screenshot / Video mode | <kbd>1</kbd> / <kbd>2</kbd> |
| Video mode: record the Screen / a Window / a Region | <kbd>F</kbd> / <kbd>W</kbd> / <kbd>R</kbd> |
| Bigger / Smaller UI / Auto | <kbd>Ctrl</kbd>+<kbd>=</kbd> / <kbd>Ctrl</kbd>+<kbd>-</kbd> / <kbd>Ctrl</kbd>+<kbd>0</kbd> |
| Accept / Cancel | <kbd>Enter</kbd> / <kbd>Esc</kbd> |
| Move / resize the selection | arrow keys / <kbd>Shift</kbd>+arrow keys |
| Square, circle or 45° line | hold <kbd>Shift</kbd> while drawing |

| In the Edit window | Keys |
|---|---|
| Zoom in / out / fit / 100 % | <kbd>Ctrl</kbd>+<kbd>=</kbd> / <kbd>Ctrl</kbd>+<kbd>-</kbd> / <kbd>Ctrl</kbd>+<kbd>0</kbd> / <kbd>Ctrl</kbd>+<kbd>1</kbd> |
| Rotate left / right | <kbd>Ctrl</kbd>+<kbd>L</kbd> / <kbd>Ctrl</kbd>+<kbd>R</kbd> |
| Flip horizontal / vertical | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> |
| Resize… / Canvas… | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>K</kbd> |
| Effects… / Reset image edits | (unbound; the Image menu) |

On macOS, <kbd>⌘</kbd> works wherever <kbd>Ctrl</kbd> is listed. Every key
can be remapped in Settings › Shortcuts. The
[user guide](docs/guide.md) covers the Edit window, recording and the
rest.

## Command line

```bash
rustshot                          # start the background daemon (tray icon; default)
rustshot daemon --force           # Windows, tray icon off: run in the background anyway
rustshot gui                      # interactive capture
rustshot gui --clip               # select, annotate, Enter copies to the clipboard
rustshot gui --accept-on-select   # drag a region: copied at once (hold Alt on release to adjust)
rustshot gui --region 800x600+100+100 --upload
rustshot full --path ~/Pictures   # whole desktop, no editor
rustshot screen -n 1 --edit       # second monitor (0-based), open the editor
rustshot full --raw > shot.png    # PNG bytes to stdout
rustshot full --delay 3000 --clip # wait 3 s, copy the whole desktop
rustshot launcher                 # the capture launcher: what, after a delay, then what
rustshot history                  # open the History window
rustshot history --list           # print captures: captured, kind, WxH, bytes, file (tab-separated)
rustshot settings                 # open the Settings window
rustshot update                   # check for a newer release
rustshot data                     # where the blur model and language files are, or where to get them
rustshot data --fetch             # download the missing ones now
rustshot config                   # show the config file
rustshot config --check           # validate config.toml
rustshot register-screenclip on   # Windows: offer Rustshot for Win+Shift+S ("off" undoes it)
rustshot --help
```

Capture options: `-p/--path`, `-c/--clip`, `--raw`, `--geometry`,
`--upload` (to the destination set in Settings › Upload),
`-d/--delay <ms>`, `-f/--filename <pattern>`,
`--region WxH+X+Y|all|screenN`, `--noedit`, `--edit`,
`--accept-on-select`, and `-n/--number` for `screen`. Windows itself
runs `rustshot screenclip <URI>` for Win+Shift+S.

## Configuration

Everything is in the **Settings** window, which is easier than editing
the file by hand. Settings are stored in `config.toml`: on Windows in
`%APPDATA%\rustshot\config.toml`, on Linux and macOS in
`~/.config/rustshot/config.toml` (or under `$XDG_CONFIG_HOME`).
`rustshot config` prints the path. A few common keys:

```toml
theme = "auto"                    # "auto", "dark" or "light"
save_path = ""                    # "" = Pictures/rustshot
filename_pattern = "%F_%H-%M"
capture_hotkey = "Shift+Meta+X"   # Meta = Win, Cmd or Super
history_days = 30
check_updates = true
download_data = true
```

Every key, the file-name tokens and the `[shortcuts]` table are listed in
[docs/configuration.md](docs/configuration.md).

## Privacy

No telemetry and no account. Captures leave your computer only when you
choose **Upload** (images) or **Copy link** (an MP4 recording), or turn
on *Upload recordings automatically*. They go to the destination you
set: Imgur, or your own S3-compatible bucket, whose secret access key
stays in the OS credential store. History, upload records and settings
stay on your computer. OCR, QR codes and the camera's background blur
run on your machine.

Rustshot makes only these requests by itself, all anonymous and to
GitHub: the daemon's update check (one call to the GitHub Releases API,
at most once a day, direct downloads only; off with
`check_updates = false`), and a language file or the blur model from the
release when one is first needed (off with `download_data = false`). See
[PRIVACY.md](PRIVACY.md).

## Building from source

You need a current stable Rust toolchain (edition 2024); on Windows the
MSVC toolchain. On Linux, also install the X11 headers: `libx11-dev
libxrandr-dev libxext-dev libxfixes-dev` on Debian or Ubuntu.

```bash
cargo build --release          # target/release/rustshot(.exe)
cargo test                     # unit tests
cargo test -- --ignored        # live display / clipboard / network tests
```

The language files (`lang/<tag>.txt`) and the camera's background-blur
model (MediaPipe's Selfie Segmentation, Apache-2.0, converted to
`models/selfie.bin`; source and hash in `models/selfie.bin.txt`) are not
in the binary. A build downloads them from the release of its version
(`src/fetch.rs` has the SHA-256 it expects), into
`%LOCALAPPDATA%\rustshot-data` (Linux `~/.local/share/rustshot`, macOS
`~/Library/Application Support/rustshot`). A build whose version has no
release yet takes a file from the newest release only when that lists
the very same SHA-256; for a changed model or a new language file, point
`RUSTSHOT_DATA_DIR` at the source tree to use its `models/` and `lang/`.
To convert the model again (then update `fetch::MODEL`: the test
`model_manifest_matches_the_file` shows the new size and hash):

```bash
cargo run --manifest-path tools/convert-selfie/Cargo.toml -- \
    selfie_segmentation_landscape.tflite models/selfie.bin
```

Packaging scripts for NSIS, MSIX, `.app`/dmg, deb, AppImage, Flatpak,
Snap, AUR and Homebrew live in [`packaging/`](packaging). CI builds all of
them on every push. Pushing a `vX.Y.Z` tag that matches `Cargo.toml`
publishes a GitHub Release
([`.github/workflows/release.yml`](.github/workflows/release.yml)). The
UI design boards are in
[docs/design/rustshot-ui](docs/design/rustshot-ui), and the specs and
plans are in [docs/superpowers](docs/superpowers).

Translations are plain text files in [`lang/`](lang); how to write, test
and send one: [docs/translating.md](docs/translating.md) (languages in
[i18n/languages.md](i18n/languages.md), terms in
[i18n/glossary.md](i18n/glossary.md)).

## Known limitations

From the v0.5.1 release notes:

- **Translations:** the 34 translations are machine-made, and none has
  been reviewed by a native speaker yet. Reviews and corrections are
  welcome.
- **Linux text:** Arabic, Hindi and Thai text is drawn without shaping, so
  letters don't join and marks may be misplaced.
- **Right-to-left:** the window title bars (drawn by the OS) are not
  mirrored.
- **macOS:** the History, Edit, launcher and Make QR code windows and
  scrolling capture are not available yet (and neither is Settings, which
  opens `config.toml` instead). The trim window, Share and the text styles
  compile on macOS but are untested on real hardware.
- **Untested:** HDR tone-mapping on a real HDR monitor, the quality of the
  camera background blur on real people, and S3 uploads against a real
  bucket (only against the SigV4 test vectors).
- **Linux S3 key:** the Secret Service session that stores the S3 key is
  not encrypted (see [PRIVACY.md](PRIVACY.md)).
- **Tray icon off** (no background process) is Windows only. It is not in
  the Microsoft Store build, where the icon off keeps Rustshot running
  without it.
- **X11:** a pin's opacity only takes effect when a compositor is running.
- **Linux OCR:** needs `tesseract` (for example `apt install
  tesseract-ocr`).
- **Unsigned builds:** see the note under [Install](#install).
- **Imgur:** Imgur uploads use a shared client ID by default; add your own
  in Settings › Upload if they're refused.

The [platform notes](docs/guide.md#platform-notes) cover Wayland and
mixed-DPI setups.

## License

Rustshot is licensed under the **Apache License 2.0** ([LICENSE](LICENSE),
[NOTICE](NOTICE)). It bundles the Inter font (SIL OFL 1.1) and
Lucide icons (ISC), and downloads Google's MediaPipe selfie segmentation
model (Apache-2.0) on demand; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
Releases up to and including 0.5.1 remain under GPL-3.0-only.
