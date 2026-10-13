# Rustshot user guide

The [README](../README.md) has the overview and the main keys. This page
goes through each part of Rustshot in detail. Every key named here can be
remapped (see [configuration.md](configuration.md#remapping-editor-shortcuts)).

- [Selecting](#selecting)
- [Annotating](#annotating)
- [Saving](#saving)
- [The launcher and delayed captures](#the-launcher-and-delayed-captures)
- [Open with](#open-with)
- [Share](#share)
- [Pins and the thumbnail](#pins-and-the-thumbnail)
- [History and the Edit window](#history-and-the-edit-window)
- [Copy text (OCR)](#copy-text-ocr)
- [Recording](#recording)
- [Global hotkeys](#global-hotkeys)
- [Win+Shift+S and Print Screen (Windows)](#winshifts-and-print-screen-windows)
- [Settings](#settings)
- [Updates](#updates)
- [Platform notes](#platform-notes)

## Selecting

Drag to select a region. Before anything is selected, the window under the
pointer is outlined (the rest of the screen stays dimmed) with its size,
e.g. `1280 × 720`; a click (less than 4 px of movement) selects exactly
that window, then the handles and arrow keys adjust it as for a dragged
selection. Menus, popups and tooltips are windows of their own, so hovering
one outlines just it. Over the desktop the whole monitor is outlined.
<kbd>Tab</kbd> steps from the whole window into the part under the pointer
(a panel, a toolbar, a sidebar, an edit field), again for the part inside
that; <kbd>Shift</kbd>+<kbd>Tab</kbd> steps back out. <kbd>Esc</kbd>
cancels.

The windows are listed once, when the overlay opens. Parts are the child
windows an app has: on Windows most classic apps have them (many modern
ones draw everything in one window); on X11 few toolkits do; macOS lists
none. On Wayland there are no window positions to snap to, so only
dragging works. `snap_windows = false` (or Settings › Capture) turns it
off; a click then selects the whole screen. The *Capture window* hotkey
snaps even then.

While picking a region (before anything is selected, and while a selection
is drawn or resized) a **magnifier** follows the pointer: 17 × 17 pixels at
8× with a pixel grid, the pointer's pixel outlined, and under it the
position and color (`640, 360 · #1e2027`); a thin accent crosshair runs
through the pointer across the whole overlay. <kbd>H</kbd> copies that
color as `#rrggbb`. Settings › Capture *Show magnifier* turns it off,
*Square magnifier* makes it square.

**Right-click** anywhere (but on an object with the select tool) opens a
ring of the palette's colors around the pointer: move towards a color and
release to pick it, or click, then click a color. The center opens the
color picker. The color goes to the selected objects, else to the current
tool (and is remembered).

**Accept on select** (Settings › Capture, `accept_on_select = "copy"` or
`"save"`; `--accept-on-select` on the command line) finishes the capture
as soon as a region is drawn or a window clicked, without the toolbar; hold
<kbd>Alt</kbd> while releasing to adjust it first. A slip under 8 × 8 px
and a plain click that selects the whole screen are not accepted.

When the overlay opens, a card shows the main keys (as remapped) for three
seconds or until the pointer moves or a key is pressed (*Show hints in the
overlay*, `overlay_hints`).

## Annotating

The tools: select (<kbd>V</kbd>), pencil (<kbd>P</kbd>), line
(<kbd>L</kbd>), arrow (<kbd>A</kbd>), rectangle (<kbd>R</kbd>), ellipse
(<kbd>E</kbd>), marker (<kbd>M</kbd>), text (<kbd>T</kbd>), callout
(<kbd>K</kbd>), numbered tags (<kbd>N</kbd>), pixelate (<kbd>X</kbd>), blur
(<kbd>B</kbd>), invert (<kbd>I</kbd>), spotlight (<kbd>S</kbd>) and
magnify (<kbd>Z</kbd>). Hold <kbd>Shift</kbd> while drawing for squares,
circles and 45° lines.

Annotations stay editable until the capture is finished: pick one with the
select tool (or <kbd>Ctrl</kbd>+click with any tool) and drag it (it snaps
to other objects' edges and the selection's center within 6 px; hold
<kbd>Alt</kbd> to drag without snapping), resize it by its square handles,
re-aim a line's or arrow's ends, a callout's tail or the magnifier's source
and loupe by their round handles. <kbd>Shift</kbd>+click adds or removes
objects; a drag on empty space with the select tool draws a marquee. The
mini-toolbar above the selection sets its color, fill, size and (for a
callout) shape, restacks, duplicates and deletes; the toolbar's color and
size apply to the selection too. Every edit is one undo step.

- A **callout** starts typing at once: <kbd>Enter</kbd> for a new line, the
  bubble keeps its width and grows down, <kbd>Esc</kbd> or a click outside
  finishes; click a selected callout to type in it again.
- A **text** or callout has one style: its mini-toolbar has a font list
  (the OS UI font, a serif, a monospace), bold, italic, underline and
  strikethrough, and for a text left / center / right alignment. While
  typing, <kbd>Ctrl</kbd>+<kbd>B</kbd> / <kbd>I</kbd> / <kbd>U</kbd> toggle
  bold, italic and underline and <kbd>Shift</kbd>+<kbd>Enter</kbd> starts a
  new line. The tools remember their last style.
- **Spotlights** keep their rectangles bright and dim the rest of the
  selection to half; the **magnifier** shows the area under its source
  twice as large.
- The **blur** blurs its rectangle's own pixels; its size sets the
  strength. It shares a toolbar button with **invert**: a click picks blur,
  holding the button down opens a menu with both.

Save As, Save to Desktop, Open with… and Share… are in the menu of the
Save button's chevron. Object actions (<kbd>Del</kbd>, <kbd>Ctrl</kbd>+<kbd>D</kbd>,
…) win over the other keys while objects are selected.

## Saving

<kbd>Ctrl</kbd>+<kbd>S</kbd> (and <kbd>Enter</kbd> when no other task was
asked for) saves without a dialog to

    <folder>/<subfolder pattern>/<file name pattern>.<png|jpg|bmp>

By default that is `Pictures/rustshot/2026-10-10/2026-10-10_14-30.png`:
one folder per day (Pictures is the system's: on Windows the Pictures
folder also when OneDrive moved it, on Linux `XDG_PICTURES_DIR`). A name
that is taken gets `_1`, `_2`, ...; the notice shows the full path.
<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> always asks (Save As, seeded
with that folder and name; the format follows the extension you pick), and
*Ask where to save* makes <kbd>Ctrl</kbd>+<kbd>S</kbd> ask too. The
clipboard and uploads stay PNG. With *Copy to clipboard after capture* on,
a save (or an upload) also copies the image.

The folder, subfolder pattern, file-name pattern, format and JPEG quality
(1-100, default 90) are in Settings › General and Settings › Capture; the
pattern editor's token buttons insert at the caret, and the line above them
shows where the next capture would go. The tokens are listed in
[configuration.md](configuration.md#file-name-tokens).

## The launcher and delayed captures

The tray's **Capture…** (or `rustshot launcher`) opens a small window:
*Capture* Region / Screen / Window, *Delay* 0 / 3 / 5 / 10 s (− / + for
any 0-60 s) and *After capture* Edit / Copy / Save / Pin; **Capture** (or
<kbd>Enter</kbd>) closes it and starts. The choices are remembered. During
a delay a small pill bottom-right counts down ("Capturing in 3 s") with
**Cancel**; the tray's tooltip counts too and its menu offers **Cancel
capture**. The tray's **Capture in 3 s** and **Capture in 10 s** start a
delayed region capture straight away. The pill never shows in the capture.

## Open with

**Open with…** (<kbd>Ctrl</kbd>+<kbd>O</kbd>; in the overlay and the Edit
window it is in the Save button's chevron menu, in History in a card's ⋯
menu) hands the capture to an app you pick: the system's *Open with*
dialog on Windows, the desktop portal's app chooser on Linux (else
`xdg-open`, the default app), the default app on macOS. A capture that was
saved opens as that file; otherwise it is written to a private
`rustshot-open` folder first (`%TEMP%` on Windows, `$XDG_RUNTIME_DIR` on
Linux, 0700 with 0600 files), whose files are removed once they are a day
old, when the daemon starts.

## Share

**Share…** (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd>, also in the Edit
window; in the overlay it is in the Save button's chevron menu, in History
in a card's ⋯ menu) opens the system's share sheet with the capture: on Windows
the Share window (titled "Rustshot capture"; a picture goes as a file and
as an image, a recording as its MP4 or GIF file), on macOS the share menu
at the pointer. The file is the same as Open with's: the saved file, else a
PNG in the private `rustshot-open` folder. The overlay closes at once;
Rustshot keeps running (also for a one-off capture without the tray) until
the share is done, at most 10 minutes. Linux has no standard share sheet,
so Share is not offered there.

## Pins and the thumbnail

**Pin to screen** (<kbd>Ctrl</kbd>+<kbd>P</kbd>, or the pin button at the
end of the toolbar) turns the selection, with its annotations, into a
borderless always-on-top window exactly where it was captured. On a pin:

| Do | To |
|---|---|
| drag | move it |
| mouse wheel | zoom it about the pointer (25-400 %) |
| <kbd>Ctrl</kbd>+wheel | change its opacity (20-100 %) |
| double-click, <kbd>Esc</kbd> | close it |
| right-click | Copy, Save, Save As…, Zoom 100 %, Opacity, Close, Close all pins |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> / <kbd>Ctrl</kbd>+<kbd>S</kbd> | copy it / save it |

Pin as many as you like; they belong to the daemon, and the tray menu
offers **Close all pins** while any are up (a one-shot `rustshot gui` stays
until its last pin is closed). Pins are not hidden from screenshots. On
macOS a pin can be dragged and double-clicked closed for now (no zoom or
menu yet).

After a screenshot is copied or saved, a **thumbnail** card appears in the
bottom-right corner of that monitor for 6 seconds (it waits while the
pointer is on it). Point at it for **Copy**, **Save**, **Pin** and **×**;
on Windows drag it into another app or a folder to drop the PNG file. When
History keeps the capture, the card starts with an accent **Edit** that
opens it in the Edit window. Turn it off with *Show thumbnail after
capture* (`show_thumbnail = false`).

## History and the Edit window

Every screenshot that is copied, saved, uploaded or pinned is kept in
`%LOCALAPPDATA%\rustshot-data\history\<yyyy-mm-dd>\<hhmmss>\` (Linux:
`~/.local/share/rustshot/history`, macOS: `~/Library/Application
Support/rustshot/history`): the selection **without** its annotations
(`original.png`), the annotations (`annotations.json`), a thumbnail and
what is known about it (when, the size, the file it was saved to, the
monitor, the window and app it came from, the uploaded link). Recordings
keep their file's place and a poster. Entries older than 30 days go, then
the oldest until the history fits in 2 GB (`history_days`,
`history_max_mb`; Settings › Capture › History). Turning *Keep a history of
captures* off (`history = false`) keeps nothing new. Saved and uploaded
files are never touched (only **Clear history…** with *Also delete saved
files* removes them). Uninstalling keeps the history unless you tick *Also
remove capture history* in the uninstaller.

**History** (<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd>, or the tray's
**History…**) lists captures by day as cards or rows. Search matches the
window title, the app, the file name and the date; filter Screenshots,
Recordings or Uploaded. <kbd>Ctrl</kbd>/<kbd>Shift</kbd>+click or a drag
on empty space selects several (Copy, Save to folder…, Delete). Hovering a
card offers Edit, Copy, Save, Upload, Pin and more; a double-click (or
<kbd>Enter</kbd>, <kbd>E</kbd>) edits; <kbd>Del</kbd> removes from History
only. A card's ⋯ menu has Save As…, Open with…, Share… (Windows, macOS), Show in
folder and Copy file path, and for an uploaded capture **Copy link** and **Delete from
Imgur…**.

Every Imgur upload's link, its *deletehash* (the key that deletes it), the
time and a small thumbnail are kept with the capture's History entry, or in
`uploads.json` beside the history folder when History is off (the last 50).
The tray's **Recent uploads** lists the last five: a click copies the link,
and **Delete … from Imgur…** asks, then deletes it.

The **Edit window** reopens a capture with its annotations as objects: the
overlay's toolbar docked at the top, every tool and object edit at any
zoom, and the objects listed on the left (top is front-most; drag to
reorder, <kbd>Del</kbd> removes).

| Do | Keys |
|---|---|
| Zoom in / out | <kbd>Ctrl</kbd>+<kbd>=</kbd> / <kbd>Ctrl</kbd>+<kbd>-</kbd>, or <kbd>Ctrl</kbd>+wheel |
| Zoom to fit / 100 % | <kbd>Ctrl</kbd>+<kbd>0</kbd> / <kbd>Ctrl</kbd>+<kbd>1</kbd> |
| Pan | <kbd>Space</kbd>+drag, the middle button, the wheel (<kbd>Shift</kbd>: sideways) |
| Tool size | <kbd>[</kbd> / <kbd>]</kbd>, <kbd>Alt</kbd>+wheel |
| Close | <kbd>Ctrl</kbd>+<kbd>W</kbd> |

**Save** (<kbd>Ctrl</kbd>+<kbd>S</kbd>) asks whether to **overwrite the
original** or **save as a new file** (`<name>-edited.png`), with *Don't ask
again* (`edit_save = "overwrite"` or `"new"`); closing with unsaved changes
asks first. `original.png` is never changed, so a capture can always be
edited again. On macOS the History and Edit windows are not available yet
(the history folder opens instead).

## Copy text (OCR)

**Copy text** (the toolbar's T-in-a-frame button, <kbd>O</kbd> or
<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> in the overlay) recognizes
the text in the selection and copies it as plain text; the notice says
*Copied text · 3 lines* (or *No text found*).
<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> does the same straight from an
area pick. Lines come top to bottom; small areas are read at twice their
size. Nothing is bundled or downloaded, and the image never leaves your
machine:

- **Windows** uses the built-in Windows OCR (`Windows.Media.Ocr`) in your
  profile's languages; add an OCR language under *Settings › Time &
  language › Language & region* if the notice says none is installed.
- **macOS** uses Vision's text recognition.
- **Linux** runs `tesseract` when it is on `PATH` (e.g. `apt install
  tesseract-ocr`).

## Recording

Press the record hotkey (<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd>), or
switch the overlay to **Video** (the mode switch above the selection, or
<kbd>2</kbd>; <kbd>1</kbd> goes back to Screenshot). Its panel, centered
under the selection, offers:

- What to record: **Screen** (<kbd>F</kbd>), **Window** (<kbd>W</kbd>,
  click the outlined window) or **Region** (<kbd>R</kbd>, drag one).
- **Camera**, **Microphone** (with its device and a level meter; MP4 only)
  and **System audio** (MP4 only).
- **MP4** (H.264 + AAC, or Opus with OpenH264 on Linux; 30 or 60 fps;
  Low / Medium / High quality; areas larger than 4096 × 2304 are scaled to
  fit) or **GIF** (10, 15 or 24 fps, no sound; it stops by itself at its
  max length, 15, 30 or 60 s).
- The countdown (0 or 3 s) and the camera's shape and ring.

The camera bubble is a real window on the screen, so it is in the
recording: drag it anywhere (it snaps into the nearest corner when let go
near one) and scroll over it for its size (120, 160, 200 or 240 px). After
the countdown a red border marks the area and a pill beside it shows the
time, **Pause**, **Stop**, the microphone, camera and system audio switches
and **Discard**; neither is in the video on Windows and macOS. Stop with the
pill, the record hotkey again, or the tray's **Stop recording**. The file
goes where a screenshot would, as `.mp4` or `.gif`; it is written as
`<name>.part` and renamed when complete, and a notice offers **Copy link**
(an MP4: uploads it to Imgur, up to 200 MB, and copies the link) or
**Copy** (a GIF), **Open** and **Show in folder**.

The pill never takes the keyboard (it would steal it from the app being
recorded), so its buttons are also in the tray menu and on global chords
registered only while a recording runs:

| While recording | Default |
|---|---|
| Stop | the record hotkey (<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd>) |
| Pause / resume | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> |
| Mute the microphone | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> |
| Hide / show the pill | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> |

(On macOS these chords are not registered yet; use the menu.) A
`rustshot gui` run with no tasks can record too and exits when the
recording ends (`saved: <path>`, exit 0; 1 on failure, 2 when it is
cancelled during the countdown).

Per platform:

- **Windows 10/11:** desktop duplication (GDI when it is not available),
  Media Foundation's H.264 encoder (the graphics card's when there is one)
  and WASAPI sound. Windows N editions need the Media Feature Pack for MP4.
- **Linux, X11:** the screen through XShm. MP4 uses `ffmpeg` when it is
  installed. Without ffmpeg (and in the Flatpak and Snap) the first MP4
  recording asks: **Download OpenH264** (Cisco's library, about 1 MB,
  checked against a SHA-256 built into Rustshot), **Install ffmpeg**,
  **Record GIF instead** or **Cancel**; the choice is kept
  (`rec_linux_encoder`). Sound is read with `libpulse-simple` (or `parec`),
  which works on PipeWire too; the camera with V4L2. X11 cannot keep a
  window out of a recording, so the bar sits outside the area (or shrinks
  to a small pill in the corner when you record your only screen).
- **Linux, Wayland (and Flatpak on any session):** the screen comes through
  the xdg-desktop-portal ScreenCast dialog and PipeWire; the camera through
  the camera portal. There is no red border (the compositor shows its own
  indicator).
- **macOS 12.3+:** ScreenCaptureKit (screen and system sound),
  AVAssetWriter and AVCaptureSession. This code has not been run on a Mac
  yet.

Limitations: one recording at a time; no editing or trimming; MJPG-only
cameras are not supported; a selection spanning two
screens on Wayland records the whole shared screen.

## Global hotkeys

| Action | Default | Config key |
|---|---|---|
| Capture region (the overlay) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>X</kbd> | `capture_hotkey` |
| Capture full screen (saved at once) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> | `capture_full_hotkey` |
| Capture window (a click selects it) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | `capture_window_hotkey` |
| Record video (again: stop) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd> | `record_hotkey` |
| Copy text from an area (OCR) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | `capture_text_hotkey` |
| Open History | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> | `history_hotkey` |
| Repeat last capture (same region, saved at once) | none | `repeat_hotkey` |
| Quit Rustshot | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Q</kbd> | `quit_hotkey` |

<kbd>Win</kbd> is <kbd>⌘</kbd> on macOS and <kbd>Super</kbd> on Linux
(`Meta` in `config.toml`). A hotkey that cannot be registered (another app
uses it) is reported once and the previous one is kept; Settings says so
under its row. While the overlay is open, a global hotkey without
<kbd>Win</kbd> acts as the overlay's own key; only Quit always quits.
Repeat last capture remembers the region for as long as the daemon runs.

## Win+Shift+S and Print Screen (Windows)

Windows opens the Snipping Tool for <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd>
(and for <kbd>Print Screen</kbd> when *Use the Print screen key to open
screen capture* is on) by launching the `ms-screenclip:` link. **Settings ›
Capture › Use Win+Shift+S and Print Screen** (or `rustshot
register-screenclip on`) offers Rustshot as that link's handler, so these
keys open Rustshot's capture even when Rustshot is not running. The link's
mode is followed: window mode picks a window, full screen saves the whole
screen, a recording starts video mode, and a delay waits that long.

- No administrator rights: everything is written for your user only
  (`HKCU\Software\Classes\Rustshot.ScreenClip`,
  `HKCU\Software\Rustshot\Capabilities` and a `Rustshot` value under
  `HKCU\Software\RegisteredApplications`).
- Windows lets no app make itself the default; you pick it under
  *Settings › Apps › Default apps › Choose defaults by link type ›
  MS-SCREENCLIP*. (If Windows instead asks which app to use at the next
  <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd>, choose **Rustshot** and
  **Always**.)
- When Windows does not open Rustshot yet, turning the option on shows
  **One more step to use Win+Shift+S** with the steps and an **Open
  Default apps** button (*Settings › Apps › Default apps*, where *Choose
  defaults by link type* is; if that page cannot be opened, the message
  says so). Back in the window, once Rustshot is the chosen app, a toast
  says *Win+Shift+S now opens Rustshot*. While the option is on but another
  app is chosen, the daemon shows the same message when it starts. Tick
  **Don't show again** and the message, however it closes (Remind me later,
  Esc, the close button, or Rustshot chosen), turns that off
  (`screenclip_hint = false`).
- Under the option, Settings shows which app Win+Shift+S opens: *Opens
  Rustshot*, *Opens Snipping Tool* (or another app), *Not set*, or *Set to
  a missing app — click to fix*. The last one is the uninstalled Snipping
  Tool: Windows still remembers the removed app, so it shows "This file does
  not have an app associated with it" instead of asking. With the tray icon
  off and Rustshot not the chosen app, the line reads *Set up Win+Shift+S —
  otherwise Rustshot can't be opened by a shortcut*, even while the option
  is off; clicking it turns the option on and opens the message.
- To undo it, turn the option off (or run `rustshot register-screenclip
  off`); uninstalling does the same.
- Not offered in the Microsoft Store (MSIX) build.

## Settings

Open **Settings…** from the tray menu, or run `rustshot settings`. A rail
on the left picks the section:

- **General:** theme (System, Dark, Light) and accent (Periwinkle, Teal,
  Amber or any `#rrggbb`), *Launch at login*, *Show icon in tray*, *Copy to
  clipboard after capture*, *Play shutter sound*, *Show thumbnail after
  capture*, *Show hints in the overlay*, *Save to* and the file name.
- **Capture:** daily subfolders, *Ask where to save*, the format and JPEG
  quality, the file-name pattern editor, the overlay's UI size (Auto
  follows each display's scaling; 75-200 % on top of it), *Snap to
  windows*, the magnifier, *Accept on select*, *Use Win+Shift+S and Print
  Screen* (Windows), the renderer (Windows), History limits and *Reset tool
  sizes & colors*.
- **Recording:** video mode's defaults (format, frame rate, quality, GIF
  length, countdown, sound, microphone, camera bubble); on Linux also the
  MP4 encoder.
- **Key bindings:** the global hotkeys, every editor shortcut and the
  *While recording* chords.
- **Upload:** the Imgur client id and *Copy link after upload*.
- **About:** the version, *Check for updates*, the daily update check and
  links.

**OK** saves and closes, **Apply** saves and keeps the window open,
**Cancel** (or <kbd>Esc</kbd>) discards. The running daemon picks the new
settings up at once. On Linux, *Browse…* needs `zenity` or `kdialog`, and
pasting into the text fields needs `wl-paste` (Wayland), `xclip` or
`xsel`. On macOS the window is not available yet; `rustshot settings` opens
`config.toml` in your editor instead.

## Updates

The daemon checks GitHub for a new release once a day (turn it off in
Settings › About or with `check_updates = false`); `rustshot update` checks
now. A new version opens a dialog with the release notes: **Update**
downloads it, checks its SHA-256 against the release's `SHA256SUMS` (it
refuses to install on a mismatch or a missing entry), waits for an open
capture to close and restarts into the new version; **Skip this version**
silences the daily check for that version.

| Install | Update |
|---|---|
| Windows installer (also winget) | runs the new `setup.exe` silently and starts Rustshot again |
| Windows portable `.exe` (in a writable folder) | swaps the new `.exe` in and restarts |
| AppImage | replaces the `.AppImage` file and restarts |
| Linux tarball (outside `/usr`, in a writable folder) | replaces the binary and restarts |
| Microsoft Store, Scoop, Flathub, Snap, Mac App Store | the store or package manager handles updates |
| `.deb`, read-only installs | opens the release page |
| macOS | the daily check announces the release on stderr and in the next capture; `rustshot update` opens the release page |

Downloads come only from the project's GitHub releases, into a private
per-user folder that is cleaned up afterwards. `SHA256SUMS` protects
against corrupted or substituted downloads, not against a compromised
GitHub account or release; builds are not code-signed yet.

## Platform notes

- **Windows 10/11:** everything in this guide. The overlay paints with GDI
  by default (`renderer = "gdi"`), which repaints only the pixels that
  changed and keeps memory low; `renderer = "software"` is the fallback.
- **Linux:** X11 sessions get every feature. In a Wayland session the
  overlay runs through XWayland and the screen comes from the
  xdg-desktop-portal (Screenshot and ScreenCast); window snapping is off
  there. The global hotkeys are X11 key grabs, which a Wayland compositor
  may not pass on while a native Wayland app has the focus; a desktop
  shortcut that runs `rustshot` always works (launching it again while the
  daemon runs starts a capture). The notice after a capture is a small window on X11;
  on Wayland it is printed on stdout. The tray icon is a
  StatusNotifierItem (KDE Plasma, Xfce, LXQt, Cinnamon, Budgie, waybar, and
  GNOME with the AppIndicator extension). Runtime libraries: `libx11`,
  `libxrandr`, `libxext`, `libxfixes`; optional: `ffmpeg`, `libpulse` (or
  `pulseaudio-utils`), `libpipewire-0.3`, `libopus`, `bzip2` and `curl`
  (OpenH264), `tesseract` (OCR). On X11, a pin's opacity needs a running
  compositor.
- **Mixed-DPI setups** (for example a 150 % laptop with a 100 % monitor):
  the overlay captures the monitor under the cursor instead of spanning all
  of them.
- **macOS 11+:** asks for *Screen Recording* permission on first capture
  (System Settings › Privacy & Security), and for the microphone and camera
  the first time a recording uses them. The launcher, Settings, History and
  Edit windows are not available yet, and the build has not been tested on
  real hardware.
