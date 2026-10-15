# Changelog

Release notes for each version are on the GitHub release page
(https://github.com/nappsllc/rustshot/releases). Lines here are for the
next release.

## 0.6.3

- Updates: users on 0.6.1 or earlier still need to download this
  version once from the Releases page (or reinstall it from the
  Microsoft Store or their package manager): those versions reject the
  repository's renamed release links. From 0.6.2 on, updates are
  offered in the app again.
- Sharper small icons: the 16, 20 and 24 px frames of the app icon (the
  title bar, the tray and the icon file's small entries, also in the
  MSIX) are drawn on the pixel grid instead of scaled down from the
  large icon with blurry thin strokes.
- The mouse wheel changes the size of the current tool and of the
  selected objects by about 12 % a notch (as do the - / + buttons and
  keys), and every notch counts. Before, a notch moved the size by 1 px
  and notches within 160 ms of each other were dropped, so resizing the
  Magnify loupe with the wheel was very slow. The partial steps of
  high-resolution wheels and touchpads add up to notches; on macOS the
  trackpad's scroll deltas are converted to notches correctly.
- Window titles: "Rustshot — Settings" and "Rustshot — History"; the
  Edit window is titled "`<file name>` — Rustshot", with a `*` before it while
  there are unsaved changes (on macOS the close button's dot instead).
- Windows: the title bar's text and the window's 1 px border take the
  colors of Rustshot's own title bar (its theme), so the frame no longer
  shows a contrasting line.
- macOS groundwork: Settings, History and the Edit window can run their
  content under a transparent title bar, their controls clear of the
  traffic lights (a 72 px zone), with the close button's dot for unsaved
  changes. Those windows are not available on macOS yet, and this layout
  is checked by builds and tests only, not yet on a Mac.

## 0.6.2

- Updates: users on 0.6.1 or earlier won't be offered this update in the
  app. Since the repository's rename GitHub spells its release links
  `nappsllc/Rustshot`, which those versions reject, so they keep saying
  they are up to date. Download 0.6.2 once from the Releases page (or
  reinstall it from the Microsoft Store or your package manager); from
  0.6.2 on, the app accepts the links in any case and updates
  automatically again.
- The tray / menu-bar icon is now the app icon (the blue tile with the
  white glyph, a red dot while recording) on every platform, instead of
  a gray or white glyph that followed the theme. The Settings and
  update-dialog icon is the blue app icon too.
- Pixelate, Blur, Invert, Spotlight, Magnify and the Ruler pick up the
  arrow, box or text under the pointer like the other drawing tools
  (drag to move it, click to select it); hold Ctrl to draw over it.

## 0.6.1

- Fast pencil, marker, lasso and ruler strokes keep every pointer
  position again (0.6.0's once-per-refresh drawing had thinned them); on
  Windows the moves the system coalesced away are recovered from the
  mouse's history, so high-rate mice draw smooth lines.
- The size label and the freeform toggle keep off the toolbar when there
  is no Record button.
- The camera bubble's grip hint widens for languages with a longer hint.
- History: the details panel's buttons are spaced (Open file no longer
  touches Delete).
- Right-to-left languages: the object list's text reads in the right
  order.
- The Edit window's Resize image and Canvas dialog titles are translated
  in 14 more languages.
- Tools: Microsoft Store screenshots in all 35 interface languages
  (tools/store-screenshots.py).

## 0.6.0

- Rustshot is now licensed under Apache-2.0 (previously GPL-3.0).
  Releases up to and including 0.5.1 remain under GPL-3.0-only. Every
  package ships LICENSE and NOTICE.
- The magnifier keeps up with fast mice: on Windows the overlay draws the
  newest pointer position once per display refresh, just before the
  vblank, instead of painting every move (1-8 kHz mice no longer make the
  loupe and crosshair trail the pointer).
- Hovering an object with any drawing tool shows the move cursor and a
  faint outline: a drag moves it, a click selects it, Ctrl held draws
  over it. Pixelate, Blur, Invert, Spotlight, Ruler and Magnify always
  draw.
- The arrow cursor over the toolbar, its menus and the other controls.
- Under the magnifier's crosshair the system pointer is hidden (it ran a
  refresh ahead of the crosshair); the exact pixel stays open at the
  crosshair's center.
- Settings › About: Website and More from PowerNapps open
  powernapps.net/r/rustshot.
- When Imgur refuses the shared client ID, the notice says so, with Open
  Upload settings; Settings › Upload says the built-in key is shared, with
  "Get a key".
- README refreshed: badges, install table, measured memory (Task Manager:
  1.9 MB idle in the tray, 4.1 MB with the overlay open).
- Docs use US spelling.
