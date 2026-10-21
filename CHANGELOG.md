# Changelog

Release notes for each version are on the GitHub release page
(https://github.com/nappsllc/rustshot/releases). Lines here are for the
next release.

## 0.6.7

- Updates: users on 0.6.1 or earlier still need to download this
  version once from the Releases page (or reinstall it from the
  Microsoft Store or their package manager): those versions reject the
  repository's renamed release links. From 0.6.2 on, updates are
  offered in the app.
- Microsoft Store build: Win+Shift+S and Print Screen. The package
  declares the `ms-screenclip:` protocol (Windows' link for those keys;
  not a reserved scheme name), so Rustshot is listed under Windows
  Settings › Apps › Default apps › Rustshot › MS-SCREENCLIP. Nothing is
  written to the registry and Rustshot never changes the default: the
  user picks it there (or "Always" when Windows asks once after the
  install). Settings › Capture › "Use Win+Shift+S and Print Screen" is
  on while Windows opens Rustshot; turning it on shows the "One more
  step" guide, whose Open Default apps goes straight to Rustshot's page
  there, and the guide closes once Rustshot is chosen. Turning it off
  while Windows opens Rustshot opens that page too (only the user can
  pick another app). `rustshot register-screenclip` writes nothing in
  the Store build; it says where to choose and which app opens now.
- Microsoft Store build: the tray icon off means no background process,
  as in the installer build: Win+Shift+S and Print Screen start Rustshot
  on demand, launching Rustshot (or Rustshot Settings) opens Settings,
  and the startup task at sign-in starts Rustshot, which exits at once
  (it tells the startup task's activation from a launch from Start).
- Microsoft Store build: Launch at login is back in Settings › General
  and the tray menu (Start at login). It turns the package's "Rustshot
  hotkeys" startup task on or off (the StartupTask API). Turned off in
  Windows (Settings › Apps › Startup, Task Manager), only the user can
  turn it on there again: the switch stays off and a message says where.
  Set by the organization's policy, it shows read-only with "Set by your
  organization". The new messages are in all 35 languages.
- The installer, portable and Linux/macOS builds behave as in 0.6.6.

## 0.6.6

- Updates: users on 0.6.1 or earlier still need to download this
  version once from the Releases page (or reinstall it from the
  Microsoft Store or their package manager): those versions reject the
  repository's renamed release links. From 0.6.2 on, updates are
  offered in the app.
- Microsoft Store package fix: the package has no hidden "Rustshot
  hotkeys" app any more (Partner Center refuses an app with no Start
  entry, a "headless" app, without a waiver). Its startup task moved to
  the Rustshot entry, whose plain launch runs the daemon, so the
  hotkeys still start at sign-in. The task keeps its id and name
  ("Rustshot hotkeys" in Windows Settings › Apps › Startup), so an
  update keeps it on or off as the user left it. Start shows Rustshot
  and Rustshot Settings, as before.

## 0.6.5

- Updates: users on 0.6.1 or earlier still need to download this
  version once from the Releases page (or reinstall it from the
  Microsoft Store or their package manager): those versions reject the
  repository's renamed release links. From 0.6.2 on, updates are
  offered in the app.
- Hotkeys: when several shortcuts can't be registered (another app
  already uses them), the Settings dialog is one summary that always
  fits the window: a heading ("4 shortcuts are in use by another app"),
  one short line per shortcut (its keys and the fallback it got, or
  "not set"), the how-to once, and "and N more" past 6 lines. Before,
  each failure was a long sentence with its own how-to, and with four
  the last was cut off above the buttons. A single failure keeps its
  sentence. The tray balloon lists 2 lines and "and N more" (within
  the 255 characters Windows keeps); stderr still gets every failure
  in full. Translated into all 35 languages.
- Settings › About has no links row any more (Website, Source on
  GitHub, Report an issue, What's new and Licenses are gone, and their
  strings with them in every language); the powernapps.net link stays.
- Settings › About: the footer is two lines, "© 2026 PowerNapps · Made
  in Los Angeles by Denis Platonov" and the license ("Apache-2.0
  License") below it, translated in every language.
- Overlay: the magnifier's coordinates and color caption has the
  loupe's even shadow all round. The panel shadow it used sat low.

## 0.6.4

- Updates: users on 0.6.1 or earlier still need to download this
  version once from the Releases page (or reinstall it from the
  Microsoft Store or their package manager): those versions reject the
  repository's renamed release links. From 0.6.2 on, updates are
  offered in the app.
- Windows hidden from screenshots: windows that keep themselves out of
  captures (`SetWindowDisplayAffinity`, as password managers and some
  secure apps do) are marked with a "Hidden from screenshots"
  placeholder instead of coming out empty, so a window that snapping
  outlines no longer shows as a blank hole. Settings › Capture "Mark
  windows hidden from screenshots" (on by default) turns it off. Full
  support on Windows; on macOS (`sharingType` none) it is checked by
  builds and tests only, not yet on a Mac; Linux is unaffected. The
  Settings window is 552 px tall (was 520) for the new row.
- Shortcuts: one shortcut a row, each with its own label and key, in two
  columns where they fit, under six sections: Global, Recording, Overlay
  tools, Overlay editing (new), Overlay actions and Edit window (new).
  About 40 translations of action and section names were shortened to
  fit (bg, cs, da, de, el, es, fr, id, it, lt, nl, pl, pt-BR, pt-PT, ro,
  ru, sl, sr-Latn, tr, uk, vi); native speakers are welcome to review
  them.
- The Settings rail starts at the top, without the logo and name (the
  title bar names the app).
- Overlay: a wheel notch repaints only what changed (a loupe's ring
  band, the toolbar's size readout), so resizing the Magnify loupe with
  the wheel feels much quicker. Inside a Pixelate, Blur or Invert
  region, the arrow, box or text beneath it is picked up (the region
  keeps its border and its empty area). The capture selection's handles
  resize it with every tool, not only with none.
- The magnifier loupe's shadow is even all round. It used to sit below
  the disc only, which read as an off-centre gray disc on light
  screens.

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
