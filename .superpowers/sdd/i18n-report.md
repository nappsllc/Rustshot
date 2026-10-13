# i18n phase 1: report

Branch `i18n` (from `handroll` 43df3dc). Status: **done** for phase 1, with the
mid-task design changes applied (English only embedded; other languages are
release assets downloaded on demand). No real translations included.

## What was built

### 1. String extraction (`tr!` / `trn!` / `key!`)
- `lang/en.txt` is the source catalog and the template: `key = value`,
  `{name}` placeholders, CLDR plural forms `key[one]` / `key[other]` (and
  zero/two/few/many), escapes `\n \t \s \\`, metadata `@name`, `@dir`,
  `@plural`. **956 keys** (13 plural), grouped by area with comments.
- `src/i18n/mod.rs`: `tr!("key")` → `&'static str`; `tr!("key", name, x = expr)`
  → `String`; `trn!("key", n, ...)` plural; `key!("key")` → `Key` for
  const tables. Keys **and named arguments are checked at compile time**
  (const fn binary search over the build-generated key list; an unknown key or
  an argument the English string lacks is a build error). `Key` derefs to
  `&str`, implements `Display`/`PartialEq<&str>`/`Label`, so tables of keys
  drop into existing `&str` APIs.
- Everything user-visible goes through it: Settings (all sections, dialogs,
  About), overlay toolbar tooltips/menus/hints/toasts, recording panel/bar,
  camera, pins, OCR/Quick Redact, History (incl. dates, sizes, plurals),
  Edit window + image dialogs, launcher, update dialog, Linux encoder dialog,
  tray menus/tips, notices, action names (Settings › Shortcuts), hotkey
  failure messages (rewritten as whole sentences, no concatenation), uploads,
  share, S3, keystore names, window titles. CLI help stays English. Inner
  technical errors (OS/HTTP/codec details) stay English inside translated
  wrappers ("Upload failed: {e}"). Key caps (Ctrl, Shift, Enter…) stay
  English (they are also config syntax) — phase 2 may revisit (de: Strg).
- String-logic that depended on English text was fixed (e.g. Settings'
  `starts_with("Already")` checks → typed checks; History object kinds are
  stored in English in history.json and translated at display time).
- **Test `i18n::tests::no_untranslated_ui_literals`**: scans all non-test
  source for literals passed straight into UI sinks (`text(ui, size, "…")`,
  `.text_at`, `.cbutton("id", "…")`, `srow(ui, tok, "…", Some("…"))`,
  `Toast::new("…")`, `Note::plain`, `notify`, `label:`/`tip:`/`title:`
  fields, `=> "…"` arms in `label/tip/title` fns …) with an allowlist of
  same-everywhere words (Rustshot, PNG, MP4, ffmpeg, key caps …). It fails on
  any new untranslated UI literal. (Pragmatic: literals routed through a
  variable are not seen.)

### Catalog packing / loading (after the controller's design changes)
- build.rs embeds **only English** (`lang/en.txt` + `en.<part>.txt`):
  values + key names, DEFLATE-compressed with `src/deflate.rs`
  (`$OUT_DIR/i18n.bin`, 7.9 KB → ~11 KB with key names), inflated on first
  use; plus `AVAILABLE`: (tag, own name, rtl) for every other
  `lang/<tag>.txt`, read leniently (a translation file never breaks the build).
- Other languages are **runtime files**: `src/i18n/files.rs` parses
  defensively (bad lines skipped and logged; a value whose placeholders differ
  from English — an unknown one, or a plain string leaving one out — falls
  back to English for that key; never panics). Several files merge (later wins):
  the downloaded copy, then each data folder's `lang/<tag>.txt` on top (package
  or user fixes).
- `src/i18n/fetch.rs`: `trait Fetch { ensure(asset, rel, done); cached(rel) }`,
  `set_fetcher(Arc<dyn Fetch>)`, per-language `Status`
  (Here/Missing/Downloading/Failed), `request(tag)`. Default fetcher
  `NoDownloads` (reads `datadir::cached`, downloads nothing: says so) until the
  datafiles branch plugs in its `fetch::ensure`. Tests use a fake.
- `src/datadir.rs` (minimal copy of the shared API — keep the datafiles one at
  merge): `data_dirs()`, `find(rel)`, `cache_dir()`, `cached(rel)`.

**Names to coordinate with `datafiles`:** release asset `lang-<tag>.txt`,
cached at `<cache_dir>/lang/<tag>.txt` (`fetch::asset(tag)`, `fetch::rel(tag)`);
local overrides `<data dir>/lang/<tag>.txt` (`i18n::data_dirs()`); my
`datadir::cache_dir()` guesses `%LOCALAPPDATA%\rustshot\data`,
`~/Library/Application Support/rustshot/data`, `$XDG_DATA_HOME/rustshot/data`
— replace with datafiles' `cached(rel)`. Wire-up at startup:
`i18n::fetch::set_fetcher(Arc::new(<adapter over fetch::ensure / datadir::cached>))`.

### 2. Settings › General › Language
- Dropdown: "System default" + English + every release language in its own
  name (`language` in config.toml: "" = system's; docs/configuration.md).
- Picking a language not downloaded yet requests it; the row shows
  "Downloading…" or "Could not download it: …" (red) with **Retry**
  (preview `settings-language-failed.png`; test `language_row_shows_the_download`).
- Saving calls `i18n::set_language`; the daemon does it on its config reload,
  History on focus. A switch bumps `i18n::generation()`; every decorated
  window (Windows/Linux/macOS backends) notices it on its timer tick
  (`wind::relocalize`): it re-lays itself out and its title is re-set
  (`Driver::title()`). No restart.
- System language: Windows `GetUserPreferredUILanguages`, macOS
  `CFLocaleCopyPreferredLanguages`, Linux `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/
  `LANG`; matched by tag, script/region rules (zh-Hant/Hans, pt-BR/PT, nb,
  sr-Latn), then primary subtag.
- **First run**: with `language = ""` and the system's language not cached,
  the first i18n use requests it; English shows meanwhile; when it lands the
  windows switch and the daemon shows the notice "Rustshot is now in {name}"
  with **Use English** (writes `language = "en"`, switches at once) and Close.

### 3. Script support
- `src/uitext.rs`: any UI string with a character the baked Inter subset
  lacks is drawn as a whole by the OS text stack (`text::UiFace`): Windows
  GDI `DrawTextW` (Uniscribe shaping + font linking; RTL paragraphs via
  `DT_RTLREADING` + RLE/PDF), macOS CoreText, Linux ab_glyph with a
  fontconfig-chosen font per character (`fc-match sans-serif:charset=…`,
  cached per 256-codepoint block) and a simplified bidi
  (`text::visual_order`). Sized to the baked font's em, on its baseline;
  widths and rasters cached per (string, size, direction) (one list, 1024
  entries / 8 MB cap). Preview `uitext_scripts.png`: Latin-1, Greek, Cyrillic,
  CJK, Arabic, Hebrew, Thai, Devanagari all shaped and aligned (Windows).
- **Linux limitation (documented in uitext.rs):** no shaper: Arabic shows
  isolated letter forms, Devanagari/Thai marks are not reordered; Hebrew and
  CJK are fine.
- **Concern:** the baked Inter subset is **ASCII only** (not
  Latin/Greek/Cyrillic as the brief assumed), so every accented string
  (é, ü …) uses the OS font (Segoe UI etc.), a slightly different look from
  Inter. Extending the subset needs the full Inter-Medium.ttf (not in the
  repo; no downloads allowed): widen `tools/subset-font.sh`'s ranges and the
  baked slot type (u8 → u16) in a later task.

### 4. RTL mirroring
- Decorated windows (Settings, History, Edit chrome, launcher, update, Linux
  codec dialog): the frame is drawn left to right and flipped in
  `Ui::finish` (`Fb::mirror` / `Fb::flip`); text and pictures are pre-flipped
  within their own boxes (`Fb::draw_text`, `paint::blit`), text fields as a
  region (`ui::clipped_ltr`), pointer events mirrored on entry
  (`ui::rtl_ev`). Result: rail on the right, labels right-aligned, controls
  and dialog buttons mirrored, switches mirrored (preview
  `settings-00-qps-rtl.png`). Icons mirror (as Windows' RTL layout does).
- Edit window: chrome mirrors, the toolbar + canvas region stays left to
  right (picture never mirrored), its pointer mapped back.
- Overlay toolbar: order reversed (tools start at the right), docked toolbar
  right-aligned in the Edit window. Linux tray menu `TextDirection` = rtl.
- Not mirrored yet: the overlay's recording panel rows and popover menus'
  text alignment (left-aligned in RTL) — phase 2 with real Arabic/Hebrew.

### 5. Pseudo-languages (test-only, not shipped)
- `qps` (accented, bracketed, 40 % longer) and `qps-rtl` (Hebrew letters,
  right to left, 40 % longer), generated from English in tests
  (`i18n::with_lang` is thread-local, so parallel tests don't interfere).
- `every_section_fits` now runs for en, qps, qps-rtl and also checks controls
  stay on their side of the rail and never overlap; new
  `dialogs_fit_in_long_languages` (all 8 Settings dialogs),
  `tooltips_fit_in_long_languages` (every toolbar tooltip on a 1280×720
  monitor at 1× and 1.5×), `right_to_left_runs_the_toolbar_backwards`,
  `right_to_left_mirrors_the_window`. All pass, no overflow.

## Numbers
- **Binary size** (Windows release, LTO, `opt-level = "z"`): 43df3dc
  2,492,416 B → i18n 2,546,176 B = **+52.5 KB** total for the whole phase
  (macros/catalog/loader/fetch/settings row/RTL/script support). Script
  support alone (uitext + OS face glue) is ≈4–5 KB on Windows. The English
  table is 7.9 KB compressed; key names live in it too (moving them out of a
  static array saved ~40 KB).
- **Per-frame cost** (Settings › General, release, 1.5×, warm caches,
  `language_frame_cost` bench): en 9.6 ms, qps 7.9 ms, qps-rtl (all text on
  the GDI path + frame flip) 9.8 ms — the OS path costs nothing measurable
  once cached; a cold string costs one GDI rasterisation.

## Tests
- Windows: `cargo test`: 1399 passed, 0 failed (80 ignored);
  `cargo clippy --all-targets -- -D warnings` clean; clippy also clean for
  `--target x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
- Linux docker (x11, worktree): all steps pass (unit 1353 passed, live x11,
  rec, sni, openh264). Wayland: see below.
- (One existing window test, `run_window_caption_buttons`, flaked once in a
  full run and passed alone.)

## Concerns / follow-ups
- Baked font ASCII-only (above).
- `NoDownloads` stub until datafiles lands: picking a language says
  "Downloading languages is not available in this build"; local files in
  `<data dir>/lang/` and the cache are already read.
- Language files are loaded once per process; a file replaced while running
  needs a restart (downloads are picked up immediately).
- Dates are translated piecewise (weekday/month abbreviations + patterns);
  phase 2 should check word order per language. Key caps not translated.
- `lang/en.txt` is grouped by area; phase 2 translators copy it to
  `lang/<tag>.txt` (set `@name`, `@dir`, `@plural`: rule names in
  `src/i18n/plural.rs` — one/other, zero_one_other, east_slavic, polish,
  czech_slovak, bcs, slovenian, romanian, latvian, lithuanian, arabic,
  hebrew, none).
