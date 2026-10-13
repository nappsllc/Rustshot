# Configuration reference

Most people never need this page: the **Settings** window (tray menu ›
*Settings…*, or `rustshot settings`) edits everything below. On macOS,
where the Settings window is not available yet, `rustshot settings` opens
`config.toml` in your editor.

## Where the file lives

| OS | Path |
|---|---|
| Windows | `%APPDATA%\rustshot\config.toml` |
| macOS / Linux | `$XDG_CONFIG_HOME/rustshot/config.toml` or `~/.config/rustshot/config.toml` |

`rustshot config` prints the path. `rustshot config --check` validates the
file and exits non-zero on errors (unknown shortcut actions and chords
bound twice are reported too).

Saving from the Settings window rewrites `config.toml`. If the file has
comments or keys Rustshot does not know, the first save asks before
removing them. A `config.toml` with an error is never overwritten
silently: the window shows the error and offers *Open config file* or
*Reset to defaults…*, which keeps the old file as `config.toml.bak` (then
`.bak.1`, `.bak.2`, ...).

## Keys

Every key is optional; these are the defaults.

```toml
# Saving
save_path = ""                    # default save folder ("" = Pictures/rustshot)
save_subfolder = true             # one subfolder per day inside save_path
subfolder_pattern = "%F"          # its name (same tokens; "/" nests, e.g. "%Y/%m")
filename_pattern = "%F_%H-%M"     # strftime-style, see the token table below
save_format = "png"               # "png", "jpg" or "bmp" (clipboard and upload stay PNG)
jpeg_quality = 90                 # 1-100
save_dialog = false               # true = Ctrl+S always asks where to save
copy_after_capture = false        # a save or upload also copies the image

# Look
theme = "auto"                    # "auto" (follow the OS), "dark" or "light"
ui_color = ""                     # accent override, e.g. "#8b93ff" ("" = theme accent)
ui_scale = "auto"                 # overlay UI size: "auto" (each display's scaling) or
                                  # 75, 90, 100, 125, 150, 200 (% on top of it)
contrast_opacity = 148            # dim strength outside the selection (0-255)
renderer = "gdi"                  # Windows overlay: "gdi" (low memory) or "software"; ignored elsewhere

# Drawing
draw_color = "#f04438"
draw_thickness = 3.0
draw_marker_size = 15.0
draw_pixelate_size = 12.0
draw_font_size = 16.0
undo_limit = 100
user_colors = ["#f04438", "#ff8a1f", "#ffc532", "#2dc06f", "#19b5d6",
               "#3b82f6", "#8b5cf6", "#ec4899", "#ffffff", "#111318"]
# The editor also remembers each tool's last size, color, fill and text
# style on its own (tool_<id>_size, tool_<id>_color, tool_<id>_fill,
# tool_text_font, tool_text_style, tool_text_align, tool_callout_font,
# tool_callout_text_style, tool_callout_style, recent_colors).
# Settings › Capture › "Reset tool sizes & colors" clears them.

# Global hotkeys (Meta = Win, Cmd or Super; "" = none)
capture_hotkey = "Shift+Meta+X"         # the overlay
capture_full_hotkey = "Shift+Meta+F"    # every monitor, saved at once
capture_window_hotkey = "Shift+Meta+E"  # the overlay picking a window
record_hotkey = "Shift+Meta+G"          # starts (video mode) and stops a recording
capture_text_hotkey = "Shift+Meta+Z"    # pick an area, copy its text (OCR)
history_hotkey = "Shift+Meta+H"         # the History window
repeat_hotkey = ""                      # the last region again
quit_hotkey = "Ctrl+Alt+Shift+Q"

# Capture behavior
capture_active_monitor = false    # true = only the monitor under the cursor
snap_windows = true               # outline the window under the pointer; a click selects it
snap_tab_hint = true              # "Tab: parts of the window" until Tab is first used
magnifier = true                  # zoomed pixels and a crosshair while picking a region
magnifier_square = false          # square instead of round
accept_on_select = "off"          # "copy" or "save": a drawn region is exported at once
overlay_hints = true              # the key card when the overlay opens
screenclip_hint = true            # Windows: remind how to pick Rustshot for Win+Shift+S
show_thumbnail = true             # the thumbnail card after a copied or saved capture
shutter_sound = false             # a short click when a capture is taken
show_tray = true                  # false = no tray icon (hotkeys still work)

# Capture launcher (remembered choices)
launcher_source = "region"        # "region", "screen" or "window"
launcher_delay = 0                # seconds (0-60)
launcher_after = "edit"           # "edit", "copy", "save" or "pin"

# History
history = true                    # keep a history of captures (History, Edit)
history_days = 30                 # entries older than this go
history_max_mb = 2048             # then the oldest until it fits
edit_save = ""                    # the Edit window's Save: "" asks, "overwrite", "new"

# Upload
upload_client_id = "313baf0c7b4d3ff"
copy_url_after_upload = true

# Updates
check_updates = true              # the daemon checks GitHub once a day
skip_version = ""                 # a version the daily check ignores ("Skip this version")

# Recording (video mode's defaults)
rec_format = "mp4"                # "mp4" or "gif"
rec_fps = 30                      # MP4: 30 or 60
rec_gif_fps = 15                  # GIF: 10, 15 or 24
rec_gif_max = 30                  # GIF: stops after 15, 30 or 60 s
rec_quality = "medium"            # MP4: "low", "medium" or "high"
rec_countdown = true              # 3-2-1 before the first frame
rec_system_audio = true
rec_mic = false
rec_mic_device = ""               # "" = the default microphone
rec_camera = false
rec_camera_device = ""            # "" = the first camera
rec_camera_shape = "circle"       # "circle", "rounded" or "square"
rec_camera_size = "m"             # "s", "m", "l" or "xl" (120, 160, 200, 240 px)
rec_camera_mirror = true
rec_camera_ring_color = ""        # "" = the accent, or "#rrggbb"
rec_camera_ring_width = 2         # 0 (no ring) to 16
rec_linux_encoder = "auto"        # Linux MP4: "auto", "ffmpeg" or "openh264"
```

Global hotkey defaults apply to a new `config.toml`; a file that already
has a key keeps its value. A config saved by 0.1.x may still have
<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> (Record) and
<kbd>Win</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> (Copy text), which some
Windows builds reserve; rebind them in Settings › Key bindings if Rustshot
reports that it could not register one.

## File-name tokens

`filename_pattern` and `subfolder_pattern` take these tokens. By default a
capture goes to `Pictures/rustshot/2026-10-10/2026-10-10_14-30.png`; a name
that is taken gets `_1`, `_2`, ...

| Token | Meaning | Token | Meaning |
|---|---|---|---|
| `%C` | century (00-99) | `%M` | minute (00-59) |
| `%j` | day of year (001-366) | `%m` | month (01-12) |
| `%d` | day (01-31) | `%S` | second (00-59) |
| `%e` | day of month (1-31) | `%V` | ISO week (01-53) |
| `%F` | `%Y-%m-%d` | `%u` | weekday (1 = Monday ... 7) |
| `%H` | hour (00-23) | `%y` | year (00-99) |
| `%I` | hour (01-12) | `%Y` | year (2026) |
| `%T` / `%R` | `%H:%M:%S` / `%H:%M` | `%p`, `%s` | AM/PM, Unix time |

Characters Windows does not allow in names (`<>:"/\|?*`) become `-`; in the
subfolder pattern `/` nests folders (`%Y/%m`) and `..` is dropped.

## Remapping editor shortcuts

Every editor key except the arrows, <kbd>Tab</kbd>, the mouse wheel and
hold-while-drawing modifiers can be remapped in **Settings › Key
bindings**: click a row (or its key cap) and press the new keys;
<kbd>Esc</kbd> cancels, <kbd>Backspace</kbd> unbinds, **Reset to
defaults** puts every key back. Keys bound to two actions are shown in red
with a note; the first action in the list wins.

In `config.toml` the same lives in a `[shortcuts]` table at the end: one or
more chords separated by commas, `""` unbinds.

```toml
[shortcuts]
tool_pencil = "P"
save_as = "Ctrl+Shift+S"
redo = "Ctrl+Shift+Z, Ctrl+Y"
ui_bigger = "Ctrl+=, Ctrl+Shift+=, Ctrl+NumpadAdd"
```

The action ids:

| Group | Actions |
|---|---|
| Tools | `tool_select`, `tool_pencil`, `tool_line`, `tool_arrow`, `tool_rectangle`, `tool_circle`, `tool_marker`, `tool_text`, `tool_callout`, `tool_counter`, `tool_pixelate`, `tool_invert`, `tool_spotlight`, `tool_magnify`, `tool_blur`, `size_down`, `size_up` |
| Output | `copy`, `copy_text`, `copy_color`, `save`, `save_as`, `save_desktop`, `open_with`, `share`, `upload`, `pin` |
| Editing | `undo`, `redo`, `select_all`, `toggle_palette`, `accept`, `cancel`, `record` |
| UI size | `ui_bigger`, `ui_smaller`, `ui_reset` |
| Selected objects | `delete_object`, `duplicate`, `bring_front`, `send_back` |
| While typing | `text_bold`, `text_italic`, `text_underline` |
| Edit window | `zoom_in`, `zoom_out`, `zoom_fit`, `zoom_actual` |
| While recording (global) | `rec_pause`, `rec_mute`, `rec_hide_pill` |

On layouts where `=`, `-` or `0` need a modifier or sit elsewhere (AZERTY,
German), use the keypad chords (`Ctrl+NumpadAdd`, `Ctrl+NumpadSubtract`,
`Ctrl+Numpad0`) or bind your own. Ctrl and Alt chords work while typing a
text and with the color picker open; a remapped plain key (no Ctrl or Alt)
does not, because those modes take every plain key as input.

`record` (unbound by default) switches the overlay to video mode
(<kbd>2</kbd> does too, <kbd>1</kbd> goes back to Screenshot; both are
built in). There is no stop action in the overlay: it is hidden while a
recording runs, so the record hotkey toggles start and stop.
