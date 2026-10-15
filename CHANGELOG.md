# Changelog

Release notes for each version are on the GitHub release page
(https://github.com/nappsllc/rustshot/releases). Lines here are for the
next release.

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
