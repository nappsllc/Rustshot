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
- [QR codes](#qr-codes)
- [Recording](#recording)
- [Uploads](#uploads)
- [Global hotkeys](#global-hotkeys)
- [Win+Shift+S and Print Screen (Windows)](#winshifts-and-print-screen-windows)
- [The tray icon off (Windows)](#the-tray-icon-off-windows)
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
through the pointer across the whole overlay. The crosshair stands in for
the system pointer, which is hidden meanwhile (the two would otherwise drift
apart by a frame); its crossing is left open, so the exact pixel under the
pointer shows between the four arms. The pointer comes back at once over the
toolbar and the hint, over an object or a handle, over text, with the
magnifier off, when another window takes the focus and when the overlay
closes. <kbd>H</kbd> copies that color as `#rrggbb`. Settings › Overlay *Show magnifier* turns it off,
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

**Freeform** (<kbd>Shift</kbd>+<kbd>F</kbd>, or the lasso button right of
the Screenshot / Video switch) makes the next selection a lasso: drag
around what you want and let go, the path closes by itself. The lasso's
bounding box is the selection (the toolbar, tools, handles and arrow keys
work as usual; moving or resizing the box takes the lasso along), and what
lies outside the lasso is dimmed. The capture is that box: transparent
outside the lasso in a PNG (an anti-aliased edge), white in a JPG or BMP;
annotations are cut to the lasso too. The clipboard gets it with its
transparency (`CF_DIBV5` and PNG on Windows; PNG elsewhere). The mode is
remembered (`selection_mode = "freeform"`); press
<kbd>Shift</kbd>+<kbd>F</kbd> again for rectangles. History keeps the lasso
with the capture (`mask` in its `meta.json`), so the Edit window opens it
the same way; its `original.png` is cut by the lasso too, so nothing
outside it is kept (an older Rustshot shows it transparent as well). A
lasso that cannot be read is not shown at all.

**Crop** (<kbd>C</kbd>) shows a frame with handles inside the selection:
drag its handles or move it (a drag outside it starts a new frame), then
<kbd>Enter</kbd> makes the frame the selection; <kbd>Esc</kbd> cancels.
It is most useful after a window click or a full-screen capture. In the
Edit window it crops the picture (see below).

**Trim to content** (<kbd>Shift</kbd>+<kbd>A</kbd>, or click the `W × H`
readout above the selection and pick *Trim to content*) tightens the
selection to what is in it: each edge moves inward while its row or column
is the edge's color (within 6 levels per channel, for at least 98 % of its
pixels, so JPEG noise and a stray pixel still count as margin), and 2 px of
the margin are kept. A window selected by a click whose own edges are not
uniform (its title bar) trims its client area instead when that has
margins, and otherwise stays whole (Windows). With a drawing tool chosen, a
press on the readout draws as anywhere else; Trim is not offered in video
mode. Annotations are ignored; <kbd>Ctrl</kbd>+<kbd>Z</kbd> puts the
selection back. *Trim margins automatically* (Settings › Capture,
`trim_margins = true`) trims every new region or window as you select it
(not a click that selects the whole screen).

**Scrolling capture** (<kbd>Shift</kbd>+<kbd>S</kbd>, or *Scrolling
capture* in the `W × H` readout's menu) takes a page longer than the
screen: select the part of a window that scrolls, and Rustshot closes the
overlay, scrolls that window step by step (it waits for smooth scrolling to
settle after each), grabs the selection each time and stitches the frames
by finding how far the content moved. Rows that stay put at the top and
bottom (a sticky header, a footer, a toolbar) come once, and a small
animation in the page does not throw it off. A pill beside the selection
shows the height so far; **Stop** or <kbd>Esc</kbd> ends it, and it ends
by itself at the bottom of the page, at 20 000 px, or when the next part no
longer matches (said in a toast; what was stitched is kept). The picture
opens in the Edit window (with History off it is saved where a capture
would be). Only the window under the selection is scrolled: on Windows
with wheel messages to it (or UI Automation's scroll pattern when those
do nothing), on X11 with wheel presses while the pointer is over it (it is
moved there once; moving it away stops the run, and it goes back at the
end). Not on Wayland, where an app cannot scroll
another's window, nor on macOS yet.

## Annotating

The tools: select (<kbd>V</kbd>), pencil (<kbd>P</kbd>), line
(<kbd>L</kbd>), arrow (<kbd>A</kbd>), rectangle (<kbd>R</kbd>), ellipse
(<kbd>E</kbd>), marker (<kbd>M</kbd>), text (<kbd>T</kbd>), callout
(<kbd>K</kbd>), numbered tags (<kbd>N</kbd>), pixelate (<kbd>X</kbd>), blur
(<kbd>B</kbd>), invert (<kbd>I</kbd>), spotlight (<kbd>S</kbd>) and
magnify (<kbd>Z</kbd>). Hold <kbd>Shift</kbd> while drawing for squares,
circles and 45° lines.

Annotations stay editable until the capture is finished. With any tool,
point at an object: the pointer turns into the move cursor and a faint
outline shows what a press picks up. Drag it to move it, click it to
select it; hold <kbd>Ctrl</kbd> while you start a stroke to draw over an
object instead. Pixelate, blur, invert, spotlight, magnify and the ruler
act on what is under them, so they always draw. A dragged object snaps
to other objects' edges and the selection's center within 6 px (hold
<kbd>Alt</kbd> to drag without snapping). Resize a selected one by its square handles,
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

**The ruler** (<kbd>U</kbd>; a key only, like invert: the toolbar keeps
its width) measures what is on screen: drag a line and its label shows the
length in px (and in logical px when the display is scaled, e.g. `400 px
(200 logical)` at 200 %), Δx, Δy and the angle (counterclockwise from the
right). <kbd>Shift</kbd> snaps the angle to 15° steps; with <kbd>Ctrl</kbd>
it measures a rectangle instead: width × height and the area. The magnifier
shows while you measure, for pixel accuracy. When you let go the
measurement stays: <kbd>Enter</kbd> keeps it as an annotation (a dimension
line with arrowheads at both ends and its length beside it, or the
rectangle with its size, in the ruler's color and size), <kbd>Esc</kbd>
discards it; another drag measures again. A kept measurement is ordinary
arrows, a rectangle and a text, so older builds show it too.

**Mark windows hidden from screenshots** (Settings › Capture,
`mark_hidden_windows`, on by default; Windows and macOS). Some apps keep
their windows out of every capture: password managers do it for their
pickers, with Windows' `SetWindowDisplayAffinity` (`WDA_EXCLUDEFROMCAPTURE`
or `WDA_MONITOR`) or macOS' `NSWindow.sharingType = .none`. The system
then hands every screenshot tool what is behind such a window (or black),
so window snapping outlines the window but the capture looks empty there.
Rustshot paints a placeholder over the part of each such window that was
visible when the capture was taken: a plain panel in the theme's colors
with the window's rounded corners, a lock and *Hidden from screenshots*.
It is part of the captured picture: the overlay, the saved file, the
clipboard, History and the Edit window all show it. Nothing of the window
is shown: no title, app name or content. What covers part of it is worked
out from rectangles (the windows above it, taken as solid boxes), so the
rounded corners or shadow of a window above can leave a sliver unmarked.
Not in recordings or scrolling captures, and not on Linux (no such
window flag there). Turn it off to get what the system returns.

**Include mouse pointer** (Settings › Capture, `include_cursor`, off by
default): the pointer is captured with the screen and added as an object
(*Mouse pointer* in the Edit window's list) where it was. Move it or
delete it like any annotation; it is never resized. It is there from the
start, so undo does not take it away. On Windows it is read with
`GetCursorInfo` and drawn with `DrawIconEx`; on Linux it needs X11 and
libXfixes (Wayland: no pointer). Not on macOS yet. Captures without an
editing step (`rustshot full`) get it drawn in. Older builds skip the
object (it hides nothing). Annotations you add later draw over it, so a
blur or a box over the pointer covers it.

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

**Add a border** (Settings › Saving, `border_width` 1-8 logical px and
`border_color`): a frame of that color around every exported image (copy,
save, upload, pin, Open with, and the same from History and the Edit
window), outside the capture so nothing is covered; the overlay previews it
as a band around the selection. The History keeps the capture without it
(the direct captures, `rustshot full` and the others, too: before this
version they recorded the bordered picture).

**Watermark on export** (Settings › Saving, `export_watermark`, off by
default): every exported image (copy, save, upload, pin, from the
overlay, History and the Edit window) gets a watermark after its border:
the text in the row (`watermark_text`), or a PNG image when
`watermark_image` is set, at `watermark_anchor` (`top-left` …
`bottom-right`, `center`) `watermark_margin` px from the edges,
`watermark_size` percent of the width wide, at `watermark_opacity`
percent, or `watermark_tiled` over the whole picture. The Edit window's
**Effects…** dialog sets all of these with a preview (*For all exports*).
The History keeps the capture without it, and a pin (from the overlay, the
thumbnail card, History or the Edit window) never gets it: a pin is a view
of the screen, not a file. A picture with its own watermark (Edit ›
Effects) does not get the export one on top; *For all exports* turns the
picture's own off. The image is a regular local PNG file of at most 64 MB
and 16 megapixels (anything else is refused, and Settings says why); the
mark never grows past the picture.

**Save at** (Settings › Saving, `save_scale`): *Screen pixels*
(the default: the capture as the screen has it), *1×* (a capture from a
high-DPI display at its logical size: a 200 % screen's capture at half its
pixels, area-averaged so text stays smooth) or *Custom* (10-100 %,
`save_scale_percent`). It applies to what you copy, save and upload (Open
with and Share too), from the overlay, the direct captures, History and
the Edit window; the border keeps its logical width at the new size. A pin
always shows the capture at screen size, the History keeps it at full size, and a picture you resized in the Edit window keeps
the size you chose.

The folder, subfolder pattern, file-name pattern, format and JPEG quality
(1-100, default 90) are in Settings › Saving and Settings › File names; the
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
PNG in the private `rustshot-open` folder. One sheet at a time: while one
is open, Share says "A share sheet is already open". The overlay closes at
once; Rustshot keeps running (also for a one-off capture without the tray)
until the share is done: the sheet closed or the target took it, at most
10 minutes (60 s after a target was chosen, or when Windows cannot tell
when the share ends). Linux has no standard share sheet, so Share is not
offered there.

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
`history_max_mb`; Settings › History). Turning *Keep a history of
captures* off (`history = false`) keeps nothing new. Saved and uploaded
files are never touched (only **Clear history…** with *Also delete saved
files* removes them). Uninstalling keeps the history unless you tick *Also
remove capture history and downloaded files* in the uninstaller (the
same folder keeps the downloaded blur model and language files).

**History** (<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd>, or the tray's
**History…**) lists captures by day as cards or rows. Search matches the
window title, the app, the file name and the date; filter Screenshots,
Recordings or Uploaded. <kbd>Ctrl</kbd>/<kbd>Shift</kbd>+click or a drag
on empty space selects several (Copy, Save to folder…, Delete). Hovering a
card offers Edit, Copy, Save, Upload, Pin and more; a double-click (or
<kbd>Enter</kbd>, <kbd>E</kbd>) edits; <kbd>Del</kbd> removes from History
only. A card's ⋯ menu has Save As…, Open with…, Share… (Windows, macOS),
Show in folder and Copy file path, and for an uploaded capture **Copy
link** and **Delete upload…**.

Every upload's link, where it went (Imgur and its *deletehash*, the key
that deletes it; or the S3 bucket, the object's key and the access key id
used), the time and a small thumbnail are kept with the capture's History
entry, or in `uploads.json` beside the history folder when History is off
(the last 50). The tray's **Recent uploads** lists the last five: a click
copies the link, and **Delete upload …** asks, then deletes it (from
Imgur, or with a signed `DELETE` from the bucket).

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

**Crop** in the Edit window (the crop button after the tools, or
<kbd>C</kbd>) puts a frame over the whole picture; <kbd>Enter</kbd> crops
to it, <kbd>Esc</kbd> cancels, and undo takes a crop back like any edit.
The crop is not destructive: the objects outside it are kept (just
hidden), and opening the frame again shows the whole picture, so it can be
widened later. It is saved in `annotations.json` as `"crop":[x,y,w,h]`;
for older builds (0.2, 0.3), which know nothing of it, opaque fills over
everything outside the crop are written after the objects
(`"stands_for":"crop"`), so what a crop hides stays hidden there too.
Every export (Save, Copy, Upload, History's own Copy) is the cropped part. What a crop or a lasso leaves out is blanked before the objects are drawn, so a magnifier (or a blur, an invert) inside never shows it; for older builds a magnifier that reads from outside the crop gets its loupe covered too.

**Image edits** in the Edit window: click the picture's size at the left
of the status bar for the **Image** menu, or use the keys.

| Do | Keys |
|---|---|
| Rotate left / right (a quarter turn) | <kbd>Ctrl</kbd>+<kbd>L</kbd> / <kbd>Ctrl</kbd>+<kbd>R</kbd> |
| Flip horizontal / vertical | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> |
| Resize… | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> |
| Canvas… | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>K</kbd> |
| Effects… | the menu (no key; bind `image_effects`) |
| Reset image edits | the menu (no key; bind `image_reset`) |

- **Rotate** and **Flip** turn the picture and every object with it.
  Text stays upright: its box's center moves where the picture takes it
  and the letters are not turned (outlined callouts and numbers alike); a
  filled callout keeps covering exactly what it covered (a quarter turn
  swaps its sides and its text wraps again); strokes, shapes, redactions
  and magnifiers follow exactly.
- **Resize…** takes a width and height in px or a percent, *Keep aspect
  ratio*, and the resampling: *Lanczos-3* (sharp) or *Area average*
  (smooth). It shows the new size and about how large the file will be
  (in the Save format). A size larger than the picture is enlarged
  bilinearly, and the dialog says so. The Edit window keeps showing the
  picture at full size; the status bar shows what it exports as
  ("1180 × 760 → 590 × 380").
- **Canvas…** adds padding (all sides or each), on a solid color (white,
  gray, black, transparent or any `#rrggbb`) or *Presentation*: a soft
  diagonal gradient of neutral tones taken from the colors at the
  picture's edges (light grays for a mostly-white screenshot), with 12 px
  rounded corners and a drop shadow (24 px blur, 25 %). The corner radius
  and the shadow can be changed for either, and the dialog shows a
  preview.
- **Effects…** come after the canvas, each on its tab with an *On* switch
  and a live preview: a **watermark** (text, or a PNG file, at one of nine
  places with a margin, or tiled; its size as a percent of the width, its
  opacity and the text's color, with a soft halo so it reads on any
  picture), **torn edges** (any of the four sides, depth and tooth width;
  *New pattern* tears them differently, the same pattern every time; a
  look, not a redaction: older builds show the full edges), a
  **reflection** (height as a percent of the picture, the opacity it
  starts at, fading out, and a gap) and a **drop shadow** round it all
  (offset, blur, color and opacity; the presentation shadow's look by
  default, and it follows the picture's shape: torn edges, rounded
  corners). They apply in that order; each takes under 30 ms on a 4K
  picture. *For all exports* keeps the watermark for Settings › Saving
  *Watermark on export*.

Every one of them is an undo step, and none touches `original.png`: they
are saved in `annotations.json` as an ordered `"transforms"` list after the
crop (flip, rotate, resize, canvas) and applied on export: the cropped
picture is turned, the objects are drawn on it, then it is resized (the
objects scale with it), put on its canvas and given its effects. The
effects are a separate `"effects"` list (`watermark`, `torn`, `shadow`,
`reflection`), which every older build ignores (they add to the picture
and hide nothing); an effect this build does not know is skipped.
The objects stay in the original's pixels, so an older build (0.2, 0.3),
which ignores the list, shows the picture unturned with every redaction
where it belongs; this build refuses a list it does not know rather than
guess.

## Copy text (OCR)

**Copy text** (the toolbar's T-in-a-frame button, <kbd>O</kbd> or
<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> in the overlay and the Edit
window) recognizes the text in the selection and opens **Text mode**: the
words it found are tinted over the picture (the rest dimmed a little), the
pointer is an I-beam over them, and you select words as in a document:

| Do | How |
|---|---|
| Select a range (in reading order, across lines) | drag from one word to another |
| Extend the selection | <kbd>Shift</kbd>+click |
| A word / its line / everything | double-click / triple-click / <kbd>Ctrl</kbd>+<kbd>A</kbd> |
| Copy the words selected (line breaks kept) | <kbd>Ctrl</kbd>+<kbd>C</kbd>, <kbd>Enter</kbd> or the **Copy** pill by them |
| Copy everything | <kbd>Ctrl</kbd>+<kbd>C</kbd> with nothing selected (*Copied all text*), or **Copy all** in the bar |
| Leave Text mode | <kbd>Esc</kbd>, Copy text again, or a tool (then <kbd>Ctrl</kbd>+<kbd>C</kbd> copies the picture) |

The notice says *Copied text · 3 lines* (or *No text found*). Settings ›
Overlay › *Copy text* picks **Copy all at once** instead (the old
behavior: the overlay closes and the text is copied;
`copy_text_mode = "all"`). <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd>
always copies everything straight from an area pick. Text mode reads only
what the capture keeps (inside a lasso, inside the Edit window's crop);
text in columns is read column by column.

A **long press** on Copy text opens its menu: *Select text*, *Copy all
text*, *Read QR code*, *Redact sensitive text*, and the kinds Quick Redact
looks for.

**Quick Redact** (<kbd>Shift</kbd>+<kbd>R</kbd>, or the menu) recognizes
the selection and covers what looks sensitive: e-mail addresses (Unicode
ones too), phone numbers (`+` and 7 to 15 digits, or grouped national
numbers), card numbers (13 to 19 digits that pass the Luhn check),
IBAN-like account numbers (mod 97), IPv4 addresses, URLs whose query names
a token, key, secret, signature or session, webhook URLs (Slack, Discord,
Telegram bots) and token-like path segments, JSON Web Tokens, `Bearer`
tokens and `Authorization:` headers, `.env` lines whose key names a
SECRET, TOKEN, KEY, PASS or PWD, the value after `api key:` / `token:`,
and the whole rest of the line after `password:` or `passphrase:`.

Each find becomes an ordinary object over its words (padded by 3 px or
15 % of their height, whichever is more), all in one undo step, so you can
delete or adjust any of them; the notice says *Redacted 3 items* or
*Nothing sensitive found*. Settings › Overlay › *Redact style*:

- **Solid** (the default, `redact_style = "solid"`): an opaque fill in the
  color around the words (the median of the box's edge), so it blends in.
  Nothing can be read back from it, and older builds draw it too.
- **Blur** (`"blur"`): a mosaic whose cells are twice the text's height
  (each cell averages whole letters across the line), then a blur of one
  cell on top. It looks softer, but only Solid is a guaranteed redaction.

The menu's checkboxes (Emails, Phone numbers, Card numbers, IDs (IBAN),
URLs, tokens & passwords, IP addresses) are remembered
(`redact_kinds = "all"`, or e.g. `"emails,phones"`). In Text mode with
words selected, Redact (the bar's *Redact selected*) covers exactly those.
Copy, Save, Upload, Pin and Enter wait while Redact is still reading
(*Redacting…*), so a capture never goes out without its covers; moving
the selection meanwhile reads the new area again.

Lines come top to bottom; small areas are read at twice their size.
Nothing is bundled or downloaded, and the image never leaves your machine:

- **Windows** uses the built-in Windows OCR (`Windows.Media.Ocr`) in your
  profile's languages; add an OCR language under *Settings › Time &
  language › Language & region* if the notice says none is installed.
- **macOS** uses Vision's text recognition.
- **Linux** runs `tesseract` when it is on `PATH` (e.g. `apt install
  tesseract-ocr`); Text mode and Redact read its `tsv` output for the
  words' boxes.

## QR codes

**Read QR code** (<kbd>Q</kbd> in the overlay, the Copy text menu, the
same key in the Edit window, or the *Read QR code from area* hotkey: no
default, `read_qr_hotkey`) reads every QR code in the selection (all of the
capture when there is none) and closes the overlay; nothing is kept in
History. The decoder is Rustshot's own: it finds the three finder patterns
of each code (at any rotation, also skewed in perspective or noisy),
corrects errors with the code's Reed-Solomon bytes, and reads numeric,
alphanumeric and byte segments (UTF-8, or ISO-8859-1 by ECI or when the
bytes are not UTF-8); Kanji segments are skipped. Several codes are read
in reading order. The toast shows what they say with **Copy** (every
code's text, one per line) and, when one is an `http://` or `https://`
link, **Open link**: the toast shows that link first, and nothing opens
until you click it (any other scheme gets no button). The toast shows the
scheme, host and port in full (an international host as punycode, e.g.
`https://xn--pple-43d.com`, with the form as written beside it), then the
path cut short; a link with user info (`user@host`) gets no Open link. A
read takes at most about 300 ms whatever the picture: a picture over 2
megapixels is read at that size first, at full size (up to 40 MP) only
when that finds nothing. The Edit window shows *Reading QR code…* and
stays responsive meanwhile. A one-shot run prints each code on its own
line with control characters escaped.

**Make QR code…** (the tray menu, and *QR code…* in the launcher) opens a
small window: the text field starts with the clipboard's text, the code
updates as you type (byte mode, error correction level M, the smallest of
versions 1-40 that fits, up to 2331 bytes, and the mask with the lowest
penalty), and **Copy image**, **Save…** and **Pin** hand it out at 10 px a
module with a 4-module quiet zone (no border or watermark). Not on macOS
yet.

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
- The camera's **Background** (under the Camera row): Off, Blur light or
  Blur strong. The person is found in each picture on your computer (the
  MediaPipe Selfie Segmentation model, or Vision on macOS 12 and later)
  and what is behind them is blurred; no picture leaves the machine. The
  model (215 KB) is downloaded from this version's GitHub release the
  first time you choose a blur (the row shows the progress; *Couldn't
  download the model · Retry* if it fails). With *Download language and
  blur files* off (Settings › General), Settings › Recording says which
  file to download and where to save it.

The camera bubble is a real window on the screen, so it is in the
recording: drag it anywhere (it snaps into the nearest corner when let go
near one) and scroll over it for its size (120, 160, 200 or 240 px). After
the countdown a red border marks the area and a pill beside it shows the
time, **Pause**, **Stop**, the microphone, camera and system audio switches
and **Discard**; neither is in the video on Windows and macOS. Stop with the
pill, the record hotkey again, or the tray's **Stop recording**. The file
goes where a screenshot would, as `.mp4` or `.gif`; it is written as
`<name>.part` and renamed when complete, and a notice offers **Copy link**
(an MP4: uploads it to the configured destination, Imgur up to 200 MB
or an S3 bucket of any size, and copies the link; with *Upload recordings
automatically* the upload starts when the recording is saved, so the
link is ready when you click) or
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

### Clicks and keystrokes

Settings › Recording › **Show clicks & keys**: **Clicks** draws a ring
that pulses where you press a mouse button (yellow for the left, blue for
the right); **Keystrokes** shows the last shortcuts in a pill at the bottom
center ("Ctrl + Shift + S", repeats as "×2"), which fades 1.5 s after the
last one. Both are drawn into the video's frames only, never on the
screen. By default only chords with <kbd>Ctrl</kbd>, <kbd>Alt</kbd> or
<kbd>Win</kbd> show, so what you type does not; **All keys** shows every
key, except (Windows) while a password field has, or may have, the focus
(after each focus change keys are hidden until Windows says what has it).
All keys can still show passwords typed into terminals, remote desktops,
games and some apps, and Linux has no password detection: Settings and
the start of a recording say so. Nothing shows while the recording is
paused or the lock screen is up, and AltGr characters (@, €, ą) are never
shown as Ctrl + Alt chords. Rustshot
listens to the input only while the recording runs and keeps nothing (see
[PRIVACY.md](../PRIVACY.md)). Windows and X11; on Wayland no app sees
another's input, and macOS needs the Input Monitoring permission.

### Trim after recording

Settings › Recording › **Trim after recording: Ask** opens a window after
Stop: the preview, a timeline of thumbnails with the start and end handles
(drag them, or <kbd>I</kbd> / <kbd>O</kbd> at the playhead), **Play**
(<kbd>Space</kbd>; it plays the selection once), <kbd>←</kbd> / <kbd>→</kbd>
a frame (<kbd>Shift</kbd>: a second). **Save trimmed** (<kbd>Enter</kbd>)
replaces the recording with the cut, **Keep full length** (<kbd>Esc</kbd>,
or closing the window) keeps it as it is; the notice, the history and an
automatic upload come after. An MP4 is cut without re-encoding: the
samples from the key frame before the start are copied and the file's
edit list starts and ends it exactly. Windows' own player ignores edit
lists, so on Windows the frames from the start to the next key frame are
encoded again with the same encoder and the rest is copied (when that
encoder's settings cannot be matched, the edit-list cut is used, and that
player then starts at the key frame). A GIF is cut by frames. The window
needs a decoder for its pictures: Media Foundation on Windows, `ffmpeg` on
Linux (without it, it trims without pictures). Not on macOS yet.

Limitations: one recording at a time; MJPG-only cameras are not
supported; a selection spanning two screens on Wayland records the whole
shared screen.

## Uploads

<kbd>Ctrl</kbd>+<kbd>U</kbd> (or **Upload** in History, Edit and the
CLI's `--upload`) sends the capture to the destination chosen in
Settings › Upload; the link replaces the image on the clipboard (*Copy
link after upload*).

- **Imgur** (the default): anonymous, with Rustshot's client id (or your
  own).
- **S3-compatible**: any bucket that speaks the S3 API: Amazon S3,
  Cloudflare R2, MinIO, Backblaze B2, Wasabi and the like. Set, in
  Settings › S3 storage (Upload's **Set up…** opens it):
  - **Endpoint**: `https://<account>.r2.cloudflarestorage.com` (R2),
    `https://s3.<region>.amazonaws.com` (AWS), your server's address
    (`https://` only; a `:port` is fine).
  - **Bucket and region** (one row): the region is `auto` for R2 (the
    default), the bucket's region for AWS.
  - **Access key ID**.
  - **Secret access key**: typed into a masked field (trimmed) and kept in
    the OS credential store, never in `config.toml` or a log, under
    `rustshot:s3:<bucket>@<endpoint host>/<access key id>` (so the same
    bucket name at two providers, or a rotated key, keeps its own) —
    Windows Credential Manager (a generic credential), the Secret
    Service on Linux (GNOME Keyring, KWallet, KeePassXC; without one
    Rustshot refuses to save the secret and says so), the login Keychain
    on macOS.
  - **Public URL base**: what links start with — a CDN in front of the
    bucket, or its `https://pub-….r2.dev` address. Empty: the object's
    own URL on the endpoint, which opens for others only when the bucket
    is public (Settings warns).
  - **Key prefix**: where in the bucket, with the file-name tokens
    (`%Y/%m/%d/` by default). The object is named after the capture's
    file name plus 128 random bits from the OS generator
    (`2026/10/12/rustshot_2026-10-12_09-14-<26 characters>.png`), so
    nothing is overwritten and links cannot be guessed.
  - **Path-style URLs** (on by default; R2 and most self-hosted servers
    want it): `https://<endpoint>/<bucket>/<key>`; off, virtual-hosted
    `https://<bucket>.<endpoint>/<key>` (AWS; a bucket name with a dot
    always goes path-style, since it would not match the endpoint's TLS
    certificate).
  - **Test connection** uploads a tiny `rustshot-test-<random>.txt` and
    deletes it again, and says what the bucket answered.

  Requests are signed with AWS Signature Version 4 (written in Rustshot,
  no SDK); pictures are hashed and signed, recordings stream with
  `UNSIGNED-PAYLOAD` over HTTPS, and files over 64 MB go up as a multipart
  upload in 16 MB parts. Every request is tried up to four times
  (network errors, 5xx, `SlowDown`; waiting 0.5, 1, 2 s plus a random
  extra, signed afresh each time). A multipart upload that still fails
  is kept: the next **Copy link** on that recording lists the parts
  already in the bucket and sends only the rest (it starts over if the
  upload is gone); one the bucket refuses (`AccessDenied` and the like)
  is aborted. Parts of an upload never finished (the app quit, the
  recording deleted) stay billed until the bucket removes them — add a
  lifecycle rule *abort incomplete multipart uploads after 1 day* (R2:
  Settings › Object lifecycle rules; AWS: Management › Lifecycle). The
  objects need no special ACL: make the bucket (or the CDN in front of
  it) public for the links to open for others.

## Global hotkeys

| Action | Default | Config key |
|---|---|---|
| Capture region (the overlay) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>X</kbd> | `capture_hotkey` |
| Capture full screen (saved at once) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> | `capture_full_hotkey` |
| Capture window (a click selects it) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | `capture_window_hotkey` |
| Record video (again: stop) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd> | `record_hotkey` |
| Copy text from an area (OCR) | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | `capture_text_hotkey` |
| Read QR code from an area | none | `read_qr_hotkey` |
| Open History | <kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> | `history_hotkey` |
| Repeat last capture (same region, saved at once) | none | `repeat_hotkey` |
| Quit Rustshot | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Q</kbd> | `quit_hotkey` |

<kbd>Win</kbd> is <kbd>⌘</kbd> on macOS and <kbd>Super</kbd> on Linux
(`Meta` in `config.toml`). While the overlay is open, a global hotkey without
<kbd>Win</kbd> acts as the overlay's own key; only Quit always quits.
Repeat last capture remembers the region for as long as the daemon runs.

### When another app has the key

- **A default falls back.** A hotkey still on its default that another app
  holds moves to a spare key: Capture full screen Win+Shift+F, then
  Win+Shift+Y, then Win+Shift+J; the others from Q, J and Y, never a key
  another Rustshot hotkey uses. A tray notification (or, without the icon,
  a note near the pointer) says e.g. *Shift+Win+F is in use by another
  app — Capture full screen is on Shift+Win+Y instead.* `config.toml`
  keeps the default, so it comes back once the other app lets go.
- **A key that stays unregistered** (one you chose, or no spare free)
  opens Settings with a message that says what to do and names a key that
  registers right now: *Shift+Win+F is used by another app. Pick another
  key: click Change shortcuts…, click the Capture full screen key and
  press a new one (Shift+Win+Q is free).* **Change shortcuts…** opens Key
  bindings with that row already waiting for the new key; the row shows
  red keys and *In use by another app*. A *While recording* key that
  another app holds is only a note (nothing opens over the recording).
- **Old defaults move.** A `config.toml` written by an older version that
  still has a former default (Record on Win+Shift+R, Copy text on
  Win+Shift+T, Capture window on Win+Shift+W; compared as keys, so
  `Meta+Shift+R` counts) gets today's default once, and that line of the
  file is rewritten (logged on stderr). If another of your hotkeys already
  has today's default, a spare key no hotkey uses is taken instead. A key
  you chose yourself is never changed.

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

### In the Microsoft Store build

The Store (MSIX) package declares `ms-screenclip` in its manifest, so
installing it is all the "registering" there is: nothing is written to the
registry, and Rustshot never changes the default itself. Windows lists
Rustshot under *Settings › Apps › Default apps › Rustshot › MS-SCREENCLIP*,
and after the install it may ask which app to use at the next
<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> (pick Snipping Tool to keep it,
or Rustshot and **Always**).

- **Use Win+Shift+S and Print Screen** is on while Windows opens Rustshot.
  Turning it on shows the same **One more step** message, whose **Open
  Default apps** opens Rustshot's own page there (MS-SCREENCLIP, *Choose a
  default*, Rustshot, *Set default*); back in the window, the message
  closes and the toast shows once Rustshot is the chosen app. The daemon
  does not remind you at start (the package offers itself to everyone).
- Turning it off while Windows opens Rustshot opens that page too: only you
  can pick another app (the message says so).
- `rustshot register-screenclip on|off` writes nothing there; it prints
  where to choose.

## The tray icon off (Windows)

With *Show icon in tray* off, nothing of Rustshot runs in the background
(0 MB): Win+Shift+S and Print Screen start it for the capture when it is
set up above.

- Turning it off asks first (*Rustshot won't run in the background. …*;
  **Turn off** / **Cancel**, and **Set up…** while Rustshot is not yet the
  Win+Shift+S app: it turns the option on and opens the message above).
  The running daemon quits as soon as nothing it opened is left.
- `rustshot daemon` (Launch at login) then exits at once (a line on
  stderr); `rustshot daemon --force` runs it anyway, also over an
  on-demand capture, and stays forced across in-app updates and in the
  login item. The Store build's startup task ("Rustshot hotkeys") does the
  same at sign-in: it starts Rustshot, which exits at once.
- Launching Rustshot, or **Rustshot Settings** in the Start menu, opens
  Settings. **About › Check for updates** there installs the update by
  itself; a capture started by Win+Shift+S checks once a day too.
- A capture started by Win+Shift+S has no other global hotkeys (the
  *While recording* keys and the recording pill work). Its process stays
  while its pins, Edit, History or Settings windows, the thumbnail, a
  recording, an upload or a save are up (background work at most two
  minutes more), then exits.
- Turning the icon on again starts the daemon.
- A `config.toml` from before this (where the icon off meant a hidden
  daemon) gets one message at the next start: *Rustshot no longer runs in
  the background while the tray icon is off. …* with **Set up
  Win+Shift+S**, **Show tray icon again** and **OK**
  (`tray_off_mode_seen = true` afterwards).
- The Microsoft Store build works the same way since 0.6.7 (its package
  declares Win+Shift+S, above). On Linux and macOS the icon off keeps
  Rustshot running without it, its hotkeys working.

## Settings

Open **Settings…** from the tray menu, or run `rustshot settings`. A rail
on the left picks the section:

- **General:** theme (System, Dark, Light) and accent (Periwinkle, Teal,
  Amber or any `#rrggbb`), *Launch at login*, *Show icon in tray* and the
  daily update check. In the Microsoft Store build *Launch at login* (and
  the tray's *Start at login*) is the package's startup task, "Rustshot
  hotkeys": turned off in Windows (*Settings › Apps › Startup* or Task
  Manager) it can only be turned on there again, so the switch stays off
  and a message says where; set by your organization's policy it is shown
  read-only (*Set by your organization*; the tray item is greyed).
- **Capture:** *Use Win+Shift+S and Print Screen* (Windows), *Snap to
  windows*, *Accept on select*; after a capture: *Copy to clipboard after
  capture*, *Play shutter sound*, *Show thumbnail after capture* and *Trim
  margins automatically*.
- **Overlay:** the magnifier (and *Square magnifier*), *Show hints in the
  overlay*; the toolbar: *Copy text*, *Redact style*, the UI size (Auto
  follows each display's scaling; 75-200 % on top of it), *Reset tool
  sizes & colors* and the renderer (Windows).
- **Saving:** *Save to*, *Ask where to save*, the format and JPEG
  quality, *Save at* (screen pixels, 1× or a percent), *Add a border*
  (width and color) and *HDR screenshots*
  (Windows).
- **File names:** daily subfolders and the file-name pattern editor.
- **History:** whether History keeps captures, its limits and *Clear
  history…*.
- **Recording:** video mode's defaults (format, frame rate, quality, GIF
  length, countdown, sound and microphone); on Linux also the MP4
  encoder.
- **Camera:** the camera bubble and its background.
- **Upload:** the destination (*Imgur* or *S3-compatible*), the Imgur
  client id (or, for S3, a line with the bucket and **Set up…**), *Copy
  link after upload* and *Upload recordings automatically*.
- **S3 storage:** the bucket's settings with **Test connection** (see
  [Uploads](#uploads)).
- **Shortcuts › Global, Recording, Overlay tools, Overlay editing,
  Overlay actions, Edit window:** the global hotkeys, the record hotkey
  with the *While recording* chords, and every editor shortcut, one
  action a row (the tools by their toolbar names; two columns where the
  rows are slim).
- **About:** the version, *Check for updates* and the powernapps.net link.

Every section fits the window; nothing scrolls.

**OK** saves and closes, **Apply** saves and keeps the window open,
**Cancel** (or <kbd>Esc</kbd>) discards. The running daemon picks the new
settings up at once. On Linux, *Browse…* needs `zenity` or `kdialog`, and
pasting into the text fields needs `wl-paste` (Wayland), `xclip` or
`xsel`. On macOS the window is not available yet; `rustshot settings` opens
`config.toml` in your editor instead.

## Updates

The daemon checks GitHub for a new release once a day (turn it off in
Settings › General or with `check_updates = false`); `rustshot update` checks
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
- **HDR displays (Windows):** a monitor in HDR is read as FP16 (scRGB)
  through Desktop Duplication and tone-mapped to SDR: the SDR content
  brightness (Settings › Display) becomes white, so SDR content (white
  pages included) comes out exactly as on an SDR monitor, with no seam
  where a capture spans both kinds; highlights brighter than that go to
  white, keeping their hue where they can; then the sRGB curve. Very
  saturated wide-gamut colors are clipped to sRGB. Rustshot keeps a
  duplication of each HDR display open, so a capture of a still desktop
  takes at most about 60 ms more per HDR display. Recordings do the same
  on the CPU for what changed in each frame, on up to four threads (MP4s
  of an HDR display convert to NV12 on the CPU too): fine for windows and
  regions, while a whole 4K HDR screen that changes everywhere at once
  drops frames (about 40 ms a frame for the tone mapping alone). *HDR
  screenshots: Keep as captured* (`hdr_screenshots = "keep"`) turns it off
  for screenshots and recordings. SDR monitors are never affected; Linux
  and macOS hand over SDR pixels, so there is nothing to do there.
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
