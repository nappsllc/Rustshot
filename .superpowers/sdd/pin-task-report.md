# Pin to screen + Quick Access thumbnail — report

Branch `pin` (from `handroll` ee43e23, `handroll` 8c95474 / OCR merged in).

Commits:
- f6033b5 pin: Pin to screen and the Quick Access thumbnail
- f96c85e pin: tests (thumbnail layout, hold, editor flow, live windows) and README
- 8d9bb7f merge handroll (OCR) into pin: Copy text and Pin share the output group

No new dependencies (Win32 via the existing `windows` features, Xlib FFI, AppKit via existing `wind::Layer`).

## Pin
- `src/pin.rs` (shared): zoom 25–400 % (×1.1 per notch, stops at 100 % when crossing it), opacity 20–100 % (±10 per notch), zoom about the pointer, drag/double-click math, `Frame` (1 px accent border scaled with DPI, room for the Spec 4-layer shadow), menu model (Copy, Save, Save As…, Zoom 100 % (disabled at 100 %), Opacity ▸ 100/80/60/40/20 % (checked), Close, Close all pins (disabled with one pin)), the drawn menu for X11, area-average downscale / nearest upscale (`resample`), per-pixel pin render (shadow only computed in the margin, so big zooms stay cheap), `open/close_all/count/tick` dispatch; test builds record pins (`take_opened`).
- Overlay: `Action::Pin` default **Ctrl+P** (free: P is the pencil, nothing used Ctrl+P; checked keymap + global defaults), remappable, in Settings › Key bindings ("Pin to screen" row) and README. Toolbar `Act::Pin` (Lucide pin glyph from the History board, tooltip "Pin to screen") ends the output group after Upload; busy OK width grows to match. Implemented as `Task::Pin` (no-op in `run_export`), opened in `App::run_finish` at the selection's global top-left with the exported (annotated) image.
- Windows `src/pin_win.rs`: layered WS_EX_TOPMOST|TOOLWINDOW popup, `UpdateLayeredWindow` (new `Dib::put_alpha` in rec_ui/win.rs: opacity = constant alpha, no re-render), CS_DBLCLKS, SetCapture drag, WM_MOUSEWHEEL (Ctrl = opacity), Esc / Ctrl+C / Ctrl+S / Ctrl+Shift+S / Ctrl+0, native TrackPopupMenu with Opacity submenu. Takes focus when shown (so Esc works). Not excluded from capture (user content).
- X11 `src/pin_x11.rs`: own thread + Display; `Kind::Managed` window (Motif no decorations, `_NET_WM_STATE_ABOVE`, skip taskbar/pager), `WM_NORMAL_HINTS` USPosition, KeyPress via XSelectInput, manual XMoveWindow drag (XWayland: `_NET_WM_MOVERESIZE`), buttons 4/5 wheel, Ctrl via `state` (made pub(crate) in rec_ui/x11.rs), double-click by timing, drawn menu in an override-redirect window with a pointer grab. Opacity = premultiplied pixels × alpha (needs a compositor).
- macOS `src/pin_macos.rs` (compile-only): `wind::Layer` floating panel; drag and double-click via the daemon driver's ticks (`pin::tick` in `App::tick`). No wheel/menu/keys yet.
- Ownership: the daemon; tray `MenuItem::ClosePins` ("Close all pins", id 15) after Capture while any pin is up (`tray::menu_with`, `pins_changed` bumps the tray generation); `HotEvent::ClosePins`. A one-shot `rustshot gui` that pinned stays hidden until its last pin closes (`App::wait_pins`).

## Thumbnail
- `notice.rs`: `Note.thumb: Option<Thumb>` (Arc image + capture top-left). `thumbnail(done, uploading, img, at)` only for Copied/Saved/SavedDesktop with no failure and no upload (errors, uploads, recordings keep the toast). Card 200 px wide (6 pad, picture 188 × 56–140 contained, rounded 8, 28 px toast line with okc icon + toast text truncated with …), surface radius 12 + Spec shadow; 16 px from the work area's right, 24 above its bottom (`place_note`). Hover: Close × top-right, pill with Copy / Save / Pin (Edit behind `edit_enabled()` = false). TTL 6 s; `hold()` freezes the clock while hovered/pressed (backends keep `frozen`). Buttons via `run_note_action` → `Next {Keep, Show, Hide}`; Save = auto-save path, the card then says "Saved · name" and knows the file; failures show the error toast. Picture scaled once per image (thread-local cache).
- Windows drag-out: press on the card, past SM_CXDRAG → `SHDoDragDrop` with the shell's data object for the file (`SHCreateItemFromParsingName` + `BHID_DataObject`: CF_HDROP and shell formats), saved file or a temp PNG in `%TEMP%\rustshot-drag` (files older than 1 h removed); the card goes when dropped. X11/macOS: no drag. macOS draws the buttons all the time (no hover events).
- Setting: Settings › General "Show thumbnail after capture" (`show_thumbnail`, default true, written via an appended `push_str` line in `to_toml`).

## Tests
- Windows: `cargo test` 921 passed, 0 failed (58 ignored); `cargo clippy --all-targets -D warnings` clean for x86_64-pc-windows-msvc, x86_64-unknown-linux-gnu and aarch64-apple-darwin (check/clippy only for the latter two).
- New unit tests: zoom/opacity clamps and steps, zoom-about-point, drag/double-click, frame, menu model + ids + drawn menu hits, resample, pin pixels; thumbnail composition/thumb_after, layout at 1/1.5/2×, bottom-right placement per work area (incl. negative origins), hold/pause logic, rendering/fade/hover, actions + drag file; tray menu with pins; config round-trip; settings click; keymap; toolbar Pin position; editor flow (`editor/pin_tests.rs`: Ctrl+P, toolbar button, one-shot wait, tray ClosePins, saved capture → card / setting off → toast).
- Live Windows (real windows): `pin_win` test (styles, rect, wheel zoom, opacity, close, close all), `notice_win::the_thumbnail_window` (bottom-right of rcWork, clickable, hover holds, Pin pins + closes card), `the_drag_carries_the_file` (data object has CF_HDROP); ignored `live_pin_on_screen` captures the screen: pin shows the image pixel-for-pixel where it was, accent border, 200 % zoom lands at twice the distance, 40 % is see-through, closed is gone — passed.
- Preview PNGs (`RUSTSHOT_PREVIEW_DIR`): `render_pin_previews` ({dark,light}-pin-{100,50}@{1,2}x, pin-menu), `render_thumbnail_previews` ({dark,light}-thumb-{copied,saved-hover,tall-hover}@{1,2}x); looked at, match the boards' toast style.
- Docker linux-test x11 (`-Ref worktree`: `git archive` cannot follow a Windows worktree's `.git` file): Linux unit tests 895 passed, including `pin::x11::tests::pin_windows_come_zoom_and_go`. The full run failed only on the harness's toast checks (it expected bottom-centre and gone within 5 s). The notice is now the card (bottom-right, 6 s), so I updated run.sh; the rerun (`RS_ONLY=daemon`) passed everything, and shot 04c-notice.png shows the card bottom-right. Wayland mode was not run.

## Concerns
- Interactive live test through the daemon (computer-use) was not possible: an elevated Task Manager was in front (UIPI) and the grant points at the installed rustshot.exe; covered instead by the live window tests above. The isolated daemon I started (PID 74668, RUSTSHOT_INSTANCE=pin-live-7731) was stopped by PID. Native TrackPopupMenu and an actual drop onto another app were not exercised end to end.
- Settings › General now needs a little scrolling (one more row; `sections_fit_or_scroll` no longer requires General to fit).
- Toolbar output group is now 7 buttons (+ Record): single-row bar 937 px; selections narrower than ~890 px wrap to two rows. Several toolbar test fixtures were widened.
- `editor::ocr_tests::two_copy_texts_in_flight_both_deliver` (from the OCR branch) failed once under full-suite load and passes alone: timing-dependent.
- Hover now pauses the recording toast too (before: restart of 6 s).
- X11 pin placement relies on the WM honouring USPosition / client moves; drag uses XMoveWindow (not `_NET_WM_MOVERESIZE`) except under XWayland. Opacity on X11 needs a compositor. macOS pins: drag + double-click only, compile-checked only.
- Merge-conflict hot spots for the annotation-rework agent: `editor/mod.rs` (`run_finish`, `apply_act`, `act_of`, App fields), `toolbar.rs` output group/test widths, `keymap.rs` ALL count, `export::Task`.
