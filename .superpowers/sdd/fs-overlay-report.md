# fs-overlay report: magnifier, color ring, accept on select, first-open hint

Branch `fs-overlay` (from `handroll` 41aa726, handroll merged at d2705c3).

## Commits
- c38f4ce wind: right-button events (`Ev::Right`) on Windows, X11 and macOS
- f8847bd config + keymap: `magnifier`, `magnifier_square`, `accept_on_select`, `overlay_hints`; `copy_color` (H)
- 1f9db95 overlay: pixel magnifier + crosshair, right-click color ring, accept on select, first-open hint card
- 13f2495 cli: `--accept-on-select` on gui/full/screen
- 0917178 settings: Show magnifier, Square magnifier, Accept on select, Show hints; Copy color key row
- 9ce1ce3 magnifier: row-span drawing, crosshair bands and skip under the toolbar, GDI bench
- 8db72ec README
- d2705c3 merge handroll
- 4b6a546 assist tests: hover repaint bound without the hint fade (Linux fonts)

## What was built
New module `src/editor/assist.rs` (+ `assist_tests.rs`) holds all four features; the
shared files only got small additive edits.

1. **Magnifier** — 140 px logical loupe (17×17 px at 8×: cell = round(8·s),
   rim 2·s), pixel grid lines when a cell is ≥ 6 physical px, the centre pixel
   outlined dark/light, caption pill under it "x, y · #rrggbb" (image coords,
   as the size label counts) with a color dot. Round by default, square
   option. Crosshair: 1 px (ui line) through the pointer pixel across the whole
   overlay, accent @ 60 % (153), not drawn under the toolbar. Placement:
   below-right 24 px off the pointer, flips left/up to stay on the monitor
   under the pointer and off the toolbar; never over the pointer (tested over
   a grid of points at 1/1.5/2×). Shown when nothing is selected and no tool is
   chosen, and during NewSel / Resize drags; never while drawing, typing, in the
   color picker, video mode, the Edit window, or with the ring open.
   **H** (`copy_color`, remappable, in Settings Key bindings "Copy text · Copy
   color") copies `#rrggbb` under the pointer with toast "Copied #rrggbb";
   Ctrl+C stays Copy. Rendering: scene elements `El::Cross` (painted under the
   selection chrome/bar) and `El::Mag`; repainted only through dirty rects
   (crosshair as one band per axis between old and new line).
2. **Color ring** — right-press opens a disc (surface tokens, shadows) with the
   palette colors (up to 12; the default palette has 10) at radius 70, 26 px
   swatches, current color ringed in text color, hot one enlarged with an
   accent ring; centre = color wheel ("Custom…" label when hot) opening the
   existing picker. Press–drag–release picks by direction past the 21 px dead
   zone; a click (release without leaving the dead zone) keeps it open for
   click-click (hover only highlights a swatch under the pointer). Esc, a click
   elsewhere, or a release back in the centre cancels. Applies through
   `Act::Color` (selection's objects if any, else current tool, remembered).
   Not opened with the Select tool on an object (old behaviour = nothing), on
   colorless tools, while typing/picking, in video mode. Needs a new
   `Ev::Right` (Win32 WM_RBUTTON*, X11 button 3 — previously delivered as a
   left click —, macOS right down/up/drag).
3. **Accept on select** — Settings › Capture segmented Off/Copy/Save
   (`accept_on_select`, default "off"). Releasing a new region (drag or
   window-snap click) runs Copy or Save (Save asks if *Ask where to save*) and
   closes; Shift on release bypasses; not in record/video mode. CLI
   `--accept-on-select` (gui/full/screen): copy unless `-p/--clip/--upload`
   name tasks; with `--region` it is accepted at once. (There is no `--save`
   flag in the CLI, so "--path" is the save case.)
4. **Hint card** — replaces the old persistent empty-state hint: a faint (92 %)
   surface card centred on the active monitor: "Drag to select · Ctrl+C copy ·
   Ctrl+S save · Esc cancel · right-click colors", key caps from the keymap
   (unbound actions dropped; "Click a window or drag to select" while
   snapping). Fades out after 3 s, or on the first pointer move (≥ 3 px from
   where the pointer first was: the window's own first WM_MOUSEMOVE does not
   dismiss), key or press. Settings › General "Show hints in the overlay"
   (`overlay_hints`, default on). Chrome only: never exported (tested).

Settings: Capture › Overlay rows "Show magnifier", "Square magnifier" (enabled
with it), "Accept on select"; General › Behavior "Show hints in the overlay".
README: Usage example, Settings, Selecting (magnifier / ring / accept / hint),
shortcut table (H, right-click), config keys.

## Tests
`cargo test`: 1040 passed, 0 failed (Windows, after the handroll merge);
`cargo clippy --all-targets -D warnings` clean (1.96); `cargo check --target
aarch64-apple-darwin --all-targets` clean. Linux docker x11: build pass; unit
tests 1010/1 — the one failure was my over-tight repaint-share threshold in
`magnifier_repaints_cover_every_changed_pixel` (Linux font metrics + hint fade),
fixed in 4b6a546; assist_tests then re-run in the docker image: 18/18 pass. Live x11 steps (capture-windows, rec, sni, oh264, cam) pass.
New tests (18 + 1 bench in assist_tests, 1 CLI test, settings clicks): loupe
geometry/zoom/grid threshold, flip placement (never on the pointer, on monitor,
off toolbar), caption/hex, sampling incl. off-image black, shown/hidden states,
H copy + remap, dirty rects cover every changed pixel (hover, new sel, resize
with toolbar skip, square) + random-tile equality, ring hit testing (dead zone,
sectors, click radius, 2×), press-drag-release, click-click, Esc / centre /
outside cancel, custom → picker, applies to selected objects, shut cases,
accept on select copy/save/Shift bypass/window-click/CLI tasks win/record mode,
begin_capture with the CLI pending, hint text from default and remapped keys,
dismissal by move/key/time and off setting, helpers never in the export,
`--accept-on-select` parse/plan. Preview PNGs (`RUSTSHOT_PREVIEW_DIR`):
`assist_mag_{round,square}_{dark,light}.png`, `assist_ring_{dark,light}.png`,
`assist_hint_{dark,light}.png` (checked visually).

## Perf (release, 3840×2160 at scale 1.5, per pointer move)
GDI renderer (`perf_gdi_magnifier`: move handling + damage() + paint):
- hover, magnifier off: 0.01 ms; **on: 1.15–1.45 ms median** (p90 ≈ 1.6–2.1)
- new-selection drag, off: 4.2–4.6 ms; on: 6.8–7.5 ms median (rects 11 → 15)
Software path (`perf_magnifier_bench`): hover on 1.37 ms; drag 4.0 → 5.9 ms.
First version was 4.8 ms hover / whole-window repaints during drags; fixed by
row-span disc/cell drawing with a hole under the opaque pixels, one band per
crosshair axis, the crosshair skipping the toolbar (it re-composited the bar
3.5×/move), and `MAX_DAMAGE_RECTS` 16 → 24 (a drag with magnifier needs up to 16).
Hover meets < 4 ms. The drag is above 4 ms, but the pre-existing drag baseline
is already ~4.3 ms at 4K (mostly the toolbar re-composite, `bar` ≈ 3 ms/move);
the magnifier adds ≈ 1.5–2.5 ms.

## Concerns
- Hint wording: the brief says "Enter copy", but Enter (Accept) saves in this
  app; the card shows the Copy chord (Ctrl+C copy) so it is truthful and
  keymap-driven. Change `hint_parts` if "Enter" is really wanted.
- The ring shows the palette (10 colors by default), not a fixed 12.
- The centre "Custom…" only works once a selection exists (the picker hangs
  off the toolbar); before that it is drawn dimmed and does nothing.
- X11 button 3 used to be delivered as a left click (Down/Up) everywhere
  (Settings, History, Edit window); it is now `Ev::Right`, ignored by those.
- Drag cost at 4K with the magnifier ≈ 7 ms (baseline 4.3 ms) — see Perf.
- Docker: after the fix only assist_tests were re-run on Linux (pass); the
  full unit run there had only that one failure. Wayland mode not run.
  `history::tests::concurrent_updates_all_land` failed once on Windows
  (file-replace contention; unrelated, passed on rerun).
- No live overlay run on the desktop (not driven interactively); covered by
  the App-level event tests and the GDI paint tests.
- Shared-file touches: keymap ALL (+CopyColor after CopyText), Settings rows,
  config (4 keys, separate `to_toml` line), `Ev` enum, compose El list,
  `MAX_DAMAGE_RECTS`, `uifb::Fb::bounds` made pub, `chrome::rainbow` pub(super),
  `chrome::hint` signature (owned parts).
