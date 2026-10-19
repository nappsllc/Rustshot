# Rustshot translation glossary

The terms `lang/en.txt` uses again and again, what each one means in the
app, and how to treat it. One concept gets one word in your language,
everywhere: if "capture" is "Aufnahme" in the History window it is
"Aufnahme" in the tray menu, the notices and Settings too. When you choose
a word for a term below, write it down in your language file's header
comment (see [docs/translating.md](../docs/translating.md)) so a reviewer
can check it.

Languages and tags: [languages.md](languages.md).

## 1. Style

**Tone.** Short, plain, calm. State what happened or what to do; no
exclamation marks, no "please" in buttons, no "successfully", no jokes.
"Uploaded — link copied", not "Great! Your image was uploaded successfully!".

**Case.** Sentence case for everything: buttons, menu items, titles,
section names ("Copy link after upload", not "Copy Link After Upload").
Capitalize what your language capitalizes anyway (German nouns), proper
names (Rustshot, Imgur, GitHub) and the window name **History** where it
names the window (see §3). A few English strings use title case by
accident ("Save As…", "Day of Month"): translate them in sentence case.

**Address.** Talk to the user directly with imperatives ("Choose another",
"Drag to select"). Where your language must pick a formal or informal
"you", use the register Windows 11 and macOS use in your language, and
use it in the whole file. Prefer constructions that need neither.

**Length.** Aim for at most English length + 30 % in characters, and
shorter for the tight places: Settings section names (the left rail),
segmented choices ("Off / Copy / Save"), button labels, toolbar tooltips
(one line on a 1280 px screen), the tray menu, the hints after key caps
("copy", "save", "cancel"). Long help texts (`*.help`, dialogs, notices)
can run longer; they wrap. The fit tests are the judge, not a character
count: run them (docs/translating.md) and shorten what they flag. Cutting
with "…" is the app's last resort, not a plan. CJK, Korean and Thai are
usually shorter in characters; their glyphs are wider, so still check.

**Punctuation.**
- `…` (one character, U+2026) at the end of a button or menu item means
  "opens a dialog or asks for more" ("Set up…", "Clear history…"). Keep it
  wherever English has it, and do not add it elsewhere. In "Uploading…",
  "Looking…" it means "in progress": keep it too.
- `·` (U+00B7) separates parts of one line ("Saved · name.png",
  "Objects · 3"). Keep it, with a space on each side.
- `›` (U+203A) builds a path through menus or settings ("Settings ›
  General", "System Settings › Privacy & Security › Camera"). Keep it.
  The names on both sides must be the real labels (§4).
- `—` (em dash) joins two parts of a message. Use your language's dash
  convention (– with spaces in German, French, Russian…) or a colon.
- `×` in "1×", "{w} × {h}", "×{c}" is the multiplication sign. Keep it.
- `→` (U+2192) means "became" ("{sw} × {sh} → {w} × {h}", the hotkey
  summary's "{name}: {chord} → now {alt}"). Right-to-left languages write
  `←` (U+2190), so it points the way the line reads.
- Quotation marks: your language's own („…“, «…», «…», 「…」, "…").
- French: a (narrow) no-break space before `: ; ? !` and inside « ».
  Write it as the character itself (U+202F or U+00A0), not `\s`.
- Numbers with units: "15 s", "30 fps", "1 MB", "24 px", "100 %": keep a
  space between number and unit where your language does (it does in most
  of Europe); Japanese and Chinese write "100%". The numbers themselves
  are formatted by the app (decimal point, for now).

**Escapes.** `\n` is a new line (only in `screenclip.guide.steps`: keep
the same number of lines and the numbered steps), `\s` a space at the
start or end of a value, `\t` a tab, `\\` a backslash. A value is one line
in the file; never break it with a real new line.

## 2. Placeholders, fragments, plurals

**Placeholders** `{name}` are filled in by the app. Keep every one exactly
as written (same name, braces, ASCII), move it wherever your grammar
needs it, never translate the name inside the braces. A plain string
that drops a placeholder, or one that adds a name English does not have,
is refused for that key (English shows instead) and listed as a problem
when the file loads. Plural forms may leave `{n}` out (Arabic `[one]`,
`[two]`, Hebrew `[two]` often do).

What they hold:

| Placeholder | Holds |
|---|---|
| `{n}`, `{c}`, `{total}` | a whole number (plural keys pick their form by `{n}`) |
| `{e}`, `{error}`, `{why}` | an error detail: the OS's or a server's text (English, or the OS's language) or one of the lower-case fragments below |
| `{name}` | a file name, a window's or app's name, a language's own name, a camera, an action — see the key's English text |
| `{path}`, `{dir}`, `{file}`, `{rel}`, `{drive}` | a file system path, folder, file name, relative path, drive letter (`D:`) |
| `{chord}`, `{keys}`, `{alt}`, `{kept}`, `{free}` | a key combination as the app shows it: `Ctrl+Shift+S`, `Shift+Win+X` |
| `{label}`, `{action}` | the name of a shortcut action (from the `action.*` keys) |
| `{in_use}`, `{note}`, `{pick}` | a whole translated sentence from another key (`settings.hotkey.in_use`, `hotkey.fail.pick`) |
| `{version}`, `{current}` | a version number, `0.4.1` |
| `{store}` | a store's name: Microsoft Store, Snap Store, Flathub… |
| `{size}`, `{used}`, `{max}`, `{got}`, `{mb}`, `{mp}`, `{px}`, `{pct}` | a size with its unit ("1.2 MB"), or a bare number of MB, megapixels, pixels, percent |
| `{time}`, `{date}`, `{day}`, `{weekday}`, `{month}`, `{year}`, `{length}`, `{left}` | clock time, a formatted date, a day number or a whole day text, an abbreviated weekday/month (from `date.*`), a year, a recording's length, seconds left |
| `{w}`, `{h}`, `{x}`, `{y}`, `{sw}`, `{sh}` | pixel sizes and positions |
| `{link}`, `{url}`, `{host}` | a web address or host name |
| `{bucket}`, `{key_id}` | an S3 bucket name, an access key ID |
| `{place}` | where a secret is kept: one of the `keystore.*` names |
| `{what}` | a lower-case noun: `rec.what.system_sound`, `rec.what.microphone`, or an action ("Upload") |
| `{text}`, `{query}`, `{head}` | user text (a text object, a search), a notice's first line |
| `{format}`, `{fps}`, `{height}`, `{color}`, `{o}`, `{mode}`, `{dll}`, `{asset}`, `{save}`, `{by_hand}` | a file format (PNG), frames per second, video height (1080), a color (`#ff8800`), an opacity, a Unix file mode, a DLL name, a download's file name, a `cam.save_*` fragment, `settings.capture.trim.by_menu` |

**Fragments.** Keys whose English starts in lower case are pieces that
the app puts into another message (`{e}`, `{why}`, `{what}`, `{place}`,
`{save}`, `{by_hand}`), or the words after key caps in the overlay hints
(`editor.hint.copy` = "copy" after `Enter`). Keep them lower case where
your language allows, no full stop, and make them read well in **every**
message that takes them: "Upload failed: {e}", "Could not save {path}:
{e}". For languages with grammatical case, choose frames that keep the
inserted noun in the nominative ("Gespeichert in: {place}" rather than
a construction needing the dative), since `{place}` is the same text in
"Saved to {place}", "Stored in {place}" and "Not stored; goes to {place}".
The `keystore.*` names include the article in English ("the macOS
Keychain"); drop or keep articles to suit those frames.

**Plurals.** Keys with `[one]` / `[other]` in English are counts. Write
one line per form of your language's `@plural` rule (the table in
languages.md), always including `[other]`:

```
history.count[one] = {n} Aufnahme
history.count[other] = {n} Aufnahmen
```

```
history.count[one] = {n} снимок
history.count[few] = {n} снимка
history.count[many] = {n} снимков
history.count[other] = {n} снимка
```

A missing form falls back to your `[other]`, then to English. Never write
a plural with "(s)" or "item(s)". Do not make a plain key plural or a
plural key plain: the loader refuses it.

**Dates.** `date.mon`… `date.dec` are abbreviations shown inside
`date.short` ("{weekday} {day} {month}") and `date.short_year`; reorder
the pattern freely ("{weekday}, {day}. {month}", "{month}{day}日（{weekday}）").
Use forms that read correctly in that pattern; there is no genitive month
form yet (e.g. Russian: write the abbreviation that works after a day
number, "12 окт."). `date.today_at`, `date.day_at`, `date.day_time` place
a clock time (`{time}`, formatted by the app) after a day.

## 3. Terms

| English | Meaning in Rustshot | Rule |
|---|---|---|
| **Rustshot** | The app's name. | Never translate or transliterate; Latin letters in every script. Not inflected where your language allows (use "Rustshot app" constructions instead of case endings if needed). |
| **PowerNapps** | The developer's name. | Never translate. |
| **capture** (noun) | One screenshot or recording taken with Rustshot, as kept in History. The general word for both. | Translate. One word for the item, used in History, tray, notices. Must cover both a picture and a video. |
| **capture** (verb) | To take a screenshot: "Capture region", "Capture window", "Capture full screen". | Translate with the usual verb for taking a screenshot. |
| **screenshot** | A still capture (as opposed to a recording). | Translate (the common word in your OS, e.g. de "Screenshot", fr "capture d'écran"). Keep apart from "capture" if your language has two words; never use "photo". |
| **recording** / **record** | A video capture (MP4 or GIF); recording = the act or the result. "Record video", "Stop recording". | Translate. Use the screen-recording term your OS uses. |
| **region** | A rectangle of the screen the user drags. "Capture region". | Translate ("area" sense). |
| **selection** / **select** | (1) The region being drawn in the overlay ("Drag to select", "While selecting"). (2) Selecting annotation objects with the Select tool. (3) Text selection in Text mode. | Translate with the ordinary "selection" word; context makes the sense clear. |
| **freeform selection** / **lasso** | A selection drawn by hand instead of a rectangle. | Translate both; "lasso" may stay as a loan word if usual in your language's graphics apps. |
| **snap to windows** | While selecting, the window under the pointer is outlined; a click takes it. | Translate. |
| **overlay** | The full-screen layer that opens on capture: the frozen screen, the selection, the toolbar, annotation tools. Also a Settings section name. | Translate as a screen layer/editor word your users would understand ("Overlay" is fine in German, Dutch, Scandinavian…). Never "superposition". Same word for the section and in messages. |
| **toolbar** | The bar of tools in the overlay and the Edit window. | Translate. |
| **tool** | A drawing/editing tool on the toolbar (Pencil, Arrow…). | Translate. |
| **annotation** | Anything drawn on a capture: shapes, text, numbers, blur… "Unsaved annotations will be lost". | Translate (de "Anmerkung"/"Markierung", fr "annotation"). |
| **object** | One annotation as an item that can be selected, moved, deleted: "Selected objects", "Objects · 3", "Bring to front". | Translate as the drawing-app word for object/element. Same word in the Edit window's object list and in Shortcuts. |
| **Pencil, Line, Arrow, Rectangle, Ellipse, Marker, Text, Number, Pixelate, Invert, Callout, Spotlight, Magnify, Blur, Select** | Tool names (tooltips `tip.*`, Shortcuts `action.tool_*`, History object kinds `history.kind.*`). Marker = highlighter; Number = numbered step badge (1, 2, 3); Callout = speech bubble with text; Spotlight = darkens everything except a box; Magnify = enlarged inset of a spot; Blur = blurred box; Pixelate = mosaic; Invert = inverted colors. | Translate each consistently across the three key groups (`tip.*`, `action.tool_*`, `history.kind.*` say the same thing). Use the names image editors use in your language. |
| **magnifier** | The loupe that follows the pointer while selecting (zoomed pixels + crosshair). | Translate; a **different** word from the Magnify tool if your language allows (magnifier = loupe; Magnify = annotation). |
| **Cloud**, **Rounded**, **Fill** | Rectangle options: cloud-shaped outline, rounded corners, filled shape. | Translate. |
| **crop** | Cut the capture to a frame the user drags (Crop tool; Enter crops). | Translate with the standard image-editing word. |
| **trim** / **trim to content** / **trim margins** | Cut off uniform borders (a solid-colored margin) automatically. Not the same as crop. | Translate with a word distinct from crop ("trim/shave edges"). |
| **Quick Redact** | Finds sensitive text (emails, card numbers, tokens…) by text recognition and covers it in one step. A feature name. | Translate descriptively and keep it capitalized as a name in languages that do so ("Schnell schwärzen"), or keep "Quick Redact" if a translation would be long. Same choice everywhere (tooltip, Shortcuts, Settings help). |
| **redact** / **redacted** | Cover text so it can't be read: solid box or blur. "Redact selected", "Redacted 3 items". | Translate with the "black out / censor" word (de "schwärzen", fr "masquer/caviarder"). |
| **sensitive text** | Personal data Quick Redact looks for. | Translate. |
| **Text mode** / **Select text** / **Copy text** | The overlay's text recognition mode: lines of recognized text can be selected and copied ("Select text to copy · 3 lines", "Copy all text"). "Copy text" is also a global shortcut and a toolbar button. | Translate. "Copy text" must be the same words in Settings › Overlay, the tooltip and Shortcuts, because the help texts refer to it. |
| **text recognition** | OCR. | Translate ("Texterkennung"). Do not write "OCR" unless that is the common word in your language. |
| **pin** / **pin to screen** / **pinned screenshot** | A capture shown in a small always-on-top window that stays on screen; "Close all pins". | Translate as a verb "pin (to the screen)" and a noun for the floating window. Not a pushpin/PIN code. |
| **History** | (1) The **History window** (the app's list of past captures; window title "Rustshot — History"; Settings section "History"). (2) the history kept ("Keep a history of captures", "Clear history"). | Translate. As the window's name it is a proper name: capitalize it where your language capitalizes names, and use the same word in "Open History", "History settings", "removes it from History". |
| **History entry** | One capture's record in History. | Translate. |
| **Edit window** | The separate window that reopens a capture from History with all its objects; its window title is the file name first, "name.png — Rustshot", so the unsaved-changes marker attaches to the file. "Edit" is also the History button. | Translate. Window titles are "Rustshot — <window>" (Settings, History) and "{name} — Rustshot" (Edit), with an em dash in every language. |
| **canvas** / **Canvas…** | The Edit window's dialog that adds space around the picture: padding, a background (solid color or presentation), corners, shadow. "No canvas". | Translate as image editors do ("Arbeitsfläche", "zone de travail"). |
| **presentation (background)** | A canvas background style: the screenshot set on a decorative backdrop for slides and posts. | Translate the adjective/noun meaning "for presenting", not "slideshow". |
| **padding** | The space the canvas adds on each side. | Translate. |
| **resize** / **scale** / **resampling** | Image size dialog: new width × height, percent scale, the resampling filter (Lanczos-3, area average). | Translate; keep "Lanczos-3" as is. |
| **Save at** / **Screen pixels** / **1×** | The resolution a capture is saved at on HiDPI screens: every physical pixel, or one pixel per logical point (1×), or a custom scale. | Translate "Save at" as a short label ("Speichern mit", "Enregistrer en"). Keep "1×". |
| **accept** / **Accept on select** | The default action (copy or save, as set) done at once; Enter accepts. | Translate ("übernehmen", "valider"). |
| **save** / **Save as…** / **Save to Desktop** / **Save to** | Save to the Save to folder; Save as asks where; Desktop = the OS's Desktop folder. | Translate with the OS's words. "Desktop" is the folder name your OS shows. |
| **copy** / **clipboard** / **Copy link** | Copy the image (or text, or the upload's link) to the clipboard. | Translate with the OS's words. |
| **upload** / **uploaded** / **Delete upload** | Send a capture to Imgur or S3 and get a link; Delete upload removes it from the server (the link stops working). | Translate ("hochladen"). Keep "Delete upload" distinct from deleting the capture. |
| **link** | The web address of an upload. | Translate (the common word: Link, lien, enlace). |
| **share** / **share sheet** | The OS's share dialog (Windows, macOS). | Translate with the OS's own word for Share. |
| **Imgur** | Image hosting service. | Never translate. "Imgur client ID": translate "client ID" only. |
| **S3** / **S3-compatible** / **S3 storage** | Amazon S3 or any compatible object store (Cloudflare R2, MinIO…). | Keep "S3". Translate "storage", "compatible". |
| **bucket**, **endpoint**, **region**, **access key ID**, **secret access key**, **key prefix**, **public URL base**, **path-style URLs** | S3 settings, in the provider's terms. | Use the words AWS's own localized console/documentation uses in your language; if it keeps the English word ("Bucket"), keep it. "Cloudflare R2", "r2.dev", "auto" stay as they are. |
| **tray** / **tray icon** / **menu bar** | The icon in the Windows notification area / Linux tray, the macOS menu bar, and its menu. | Use the term your OS uses in its own UI (Windows: notification area / system tray word of your language). |
| **shortcut** / **hotkey** / **key binding** | A key combination. English uses "shortcut" (Settings › Shortcuts, "Change shortcuts…") and "hotkey" (messages) for the same thing. | One word for both in your language, the one Windows uses for keyboard shortcuts ("Tastenkombination", "raccourci clavier"). |
| **global (shortcut)** | Works in every app, Rustshot in the background. | Translate. |
| **Win+Shift+S** | The Windows screenshot shortcut Rustshot can take over (through the `ms-screenclip:` link type). | Never translate; it is a key combination. Write exactly `Win+Shift+S`. |
| **Snipping Tool** | Windows' own screenshot app. | Use the exact name Microsoft gives the app in your Windows language (Start menu, Microsoft Store listing); many languages keep "Snipping Tool". |
| **Print Screen** | The key. | Use the key's name as your localized Windows Settings write it (Accessibility › Keyboard); if unsure keep "Print Screen". |
| **MS-SCREENCLIP**, **Default apps**, **Choose defaults by link type** | Windows Settings labels in the Win+Shift+S guide. | `MS-SCREENCLIP` never translated. The other two: Microsoft's exact translation in Windows 11 Settings for your language, so the user can find them. |
| **System Settings › Privacy & Security › Screen Recording / Microphone / Camera** (macOS), **Windows Settings › Privacy › Camera**, **Settings > Apps > Default apps** | OS settings paths. | Apple's / Microsoft's exact localized labels. |
| **camera bubble** | The webcam picture shown in a round (or rounded, square) frame over a recording. | Translate as "a small floating camera frame" reads in your language; a literal "bubble" only if it sounds natural. Same word in Settings and messages. |
| **ring** | The colored outline of the camera bubble. | Translate ("Rand", "contour"). |
| **(background) blur** (camera) / **blur model** | Blurring what is behind the person on camera, using a downloaded model file. | Translate "blur" with the same word as the Blur tool. "model" = the machine-learning model file. |
| **mirror** (camera) | Flip the camera picture like a mirror. | Translate. |
| **recording controls** / **recording bar** | The small floating bar while recording (time, pause, stop). | Translate; same word in tray and Shortcuts ("Hide / show recording controls"). |
| **countdown** / **delay** | 3-2-1 before a recording starts / seconds before a delayed capture. | Translate. |
| **system sound** / **system audio** / **microphone** / **mic** | What a recording takes sound from. | Translate; "mic" = microphone (use the full word if no short form exists). |
| **encoder** / **ffmpeg** / **OpenH264** / **Cisco** / **Opus** / **libopus** / **bzip2** / **Media Foundation** / **Media Feature Pack** / **Windows N** | Video encoding pieces and the Windows component. | Translate "encoder". Product, library and package names never translated. "Media Feature Pack": Microsoft's name in your language if it has one. |
| **frame rate** / **fps** | Frames per second. | Translate "frame rate"; keep "fps" (or the usual abbreviation, e.g. fr "i/s", if your OS uses it). |
| **thumbnail** | The small preview after a capture. | Translate. |
| **notice** | A small Rustshot message window (bottom corner) with action buttons. | Not in the UI as a word; translate messages naturally. |
| **toast** | A short message in the overlay ("Copied", "UI size 125 %"). | Not shown as a word. |
| **UI size** / **Auto** | Scale of the overlay toolbar and labels; Auto follows each monitor. | Translate "UI size" ("UI" may stay if usual); "Auto" short. |
| **theme** / **System** / **Dark** / **Light** / **accent** | Appearance settings; accent = highlight color. | Translate. |
| **renderer** / **GDI** / **Software** | How the overlay draws. | Translate "renderer", "Software" only if your language translates it; keep "GDI". |
| **HDR** / **SDR** / **tone-map** | High dynamic range captures and converting them. | Keep HDR/SDR; translate "tone-map" with the imaging term ("Tonemapping" is fine). |
| **pattern** / **token** (file names) | The file name pattern and its date/time pieces ("Day of Month", "Full Date (%Y-%m-%d)"). | Translate the names; keep every `%Y`, `%m`, `%d` code and the ranges "(01-31)" exactly. |
| **config.toml**, `rustshot settings`, `rustshot update` | The config file and commands. | Never translate; keep backticks. |
| **GitHub**, **github.com**, **Wayland**, **X display**, **portal**, **tesseract**, **MJPG**, **IBAN**, **URL**, **IP**, **JPEG**, **PNG**, **MP4**, **GIF**, **RUSTSHOT_DATA_DIR** | Names, formats, technical terms. | Never translate. "portal" (the Linux screen-sharing dialog) may be translated as a common noun. |
| **Launch at login** (Settings) / **Start at login** (tray) | The same option, two labels. | Translate each; the error messages quote them ("Could not change Launch at login"), so keep each one identical to its label. |
| **Download language and blur files** | Settings › General: fetch these files from the release when needed. | Translate. |
| **System default** (language) | Follow the OS language. | Translate. The language names in the menu are each language's `@name`, never translated. |

## 4. Labels quoted inside other strings

Some messages name a button, setting or section. The quoted words must be
your translation of that label, letter for letter:

| String | Quotes |
|---|---|
| `hotkey.fail.pick`, `hotkey.fail.pick_free`, `hotkey.fail.pick_any` | "Change shortcuts…" = `settings.q.change_shortcuts` |
| `settings.tray.hide_win` | "Settings > Capture" = `settings.section.capture` |
| `settings.tray.hide`, `_keep`, `_win` | "Settings" (the window), "Start menu" (Windows' name) |
| `settings.overlay.copy_text.help` | "Copy text" = `tip.copy_text` / `hotkey.copy_text` |
| `settings.overlay.redact_style.help` | "Quick Redact", "Solid" = `settings.redact.solid` |
| `settings.saving.save_at.help` | "History" |
| `settings.file_names.daily_subfolders.help` | "Save to" = `settings.saving.save_to` |
| `settings.upload.upload_recordings_automatically.help` | "Copy link" = `history.menu.copy_link` |
| `settings.s3.used_when` | "Upload › Upload to" = `settings.section.upload` › `settings.upload.upload_to`; "S3-compatible" = `settings.upload.s3` |
| `settings.s3.public_warning` | "Public URL" = `settings.s3.public_url_base` |
| `settings.error.autostart` | "Launch at login" = `settings.general.launch_at_login` |
| `tray.autostart_failed` | "Start at login" = `tray.start_at_login` |
| `s3.no_secret_for` | "Settings › Upload" |
| `i18n.downloads_off` | "Settings › General" |
| `screenclip.guide.steps` | "Open Default apps" = `screenclip.guide.open`; "Choose defaults by link type" (Windows' label) |
| `settings.keys.group.*` | the action words as in `action.*` / `tip.*` (Undo, Redo, Copy, Save, Share, Pin, Zoom, Rotate, Flip); "Selected objects" as in the Edit window; "Capture" and "UI size" headings are `settings.section.capture` / `settings.overlay.ui_size` |
| `action.*` | the Shortcuts rows' labels, two columns of them: keep them about as short as English (the fit tests flag a label that wraps) |

## 5. Keys and key caps

Key names are config syntax (`capture_region = "Win+Shift+S"` in
config.toml) and the app draws them itself in key caps and in `{chord}`:
`Ctrl`, `Alt`, `Shift`, `Win` (Windows) / `Cmd` (macOS) / `Super`
(Linux), `Enter`, `Esc`, `Del`, `Tab`, `Space`, `F1`…, letters in capitals,
`+` between them. They stay in English for now: write `Ctrl+U`,
`Ctrl+Shift+S`, `Alt`, `Esc`, `Enter`, `Del`, `H` exactly as English does
in prose, so the text matches the key caps next to it (do not write
`Strg`, `Maj`, `Entrée`). A later version may show localized cap names;
the translation will not need to change for it.

What you do translate: mouse words ("click", "right-click", "drag",
"scroll", "Ctrl+drag" → translate "drag", keep "Ctrl+"), the wheel cap
`tip.key.wheel`, and the words around keys ("Esc to cancel", "Enter
crops"). In the overlay hints (`editor.hint.*`) the word follows a key
cap: keep it to one short word.
