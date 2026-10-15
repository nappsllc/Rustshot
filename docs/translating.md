# Translating Rustshot

Rustshot's UI is in English (`lang/en.txt`, built into the app) and in the
languages listed in [i18n/languages.md](../i18n/languages.md), one plain
text file each: `lang/<tag>.txt`. Every release publishes those files as
`lang-<tag>.txt`; the app downloads the one it needs the first time that
language is picked (or is the system's language). Translating means
editing one text file; no Rust, no tools beyond a text editor. Testing the
layout needs a Rust toolchain (see [Fit tests](#fit-tests)).

Before you start, read [i18n/glossary.md](../i18n/glossary.md): the terms,
the tone, placeholders, plurals and key names.

## The file

UTF-8 (no BOM needed, UTF-16 is refused), one entry per line, the same
keys as `lang/en.txt`. Keep `lang/en.txt`'s order and its `#` section
comments, so a diff against English lines up.

```
# Rustshot UI strings: Deutsch (German).
# machine translated — review welcome
# Translators: @github-handle
# Terms: capture = Aufnahme; overlay = Overlay; pin = anheften; History = Verlauf
@name = Deutsch
@dir = ltr
@plural = one_other

# Settings window
settings.section.general = Allgemein
settings.section.capture = Aufnahme
...
history.count[one] = {n} Aufnahme
history.count[other] = {n} Aufnahmen
```

- `key = value`. Spaces around `=` and at the ends of the value are
  dropped; write `\s` for a space that must stay at an edge.
- `#` starts a comment line (only at the start of a line; a `#` inside a
  value is text, so `#ffffff` is fine).
- Escapes: `\n` new line, `\t` tab, `\s` space, `\\` backslash. No real line
  breaks inside a value; no other control characters.
- `{placeholders}` exactly as English has them (glossary §2). A value with
  a placeholder English does not have, or a plain value missing one, is
  ignored (English shows) and reported.
- Plurals: one line per form of your rule, `key[one]`, `key[few]`,
  `key[many]`, `key[other]`… (`zero`, `one`, `two`, `few`, `many`,
  `other`), always with `[other]`. The forms per language are in
  languages.md.
- `key =` with nothing after it keeps English on purpose (a brand name, a
  term you leave in English). A key left out does the same, but write it
  out so reviewers can tell a choice from a gap.
- Metadata: `@name` — the language's name in itself, as in languages.md
  (it is what the language menu shows); `@dir` — `ltr` or `rtl`;
  `@plural` — the rule name from languages.md (`one_other`,
  `zero_one_other`, `east_slavic`, `polish`, `czech_slovak`, `bcs`,
  `slovenian`, `romanian`, `latvian`, `lithuanian`, `arabic`, `hebrew`,
  `none`). An unknown rule name quietly means `one_other`.
- A file may be split: `lang/de.txt` plus `lang/de.<part>.txt` beside it
  are read as one. Keep to one file unless it becomes unwieldy.
- Lines that are wrong (unknown key, a plain key written as plural, bad
  placeholders) are skipped and listed; they never stop the app. The app
  prints them as `rustshot: language file …` on stderr when it loads the
  file, and the test `catalogs_agree_with_english` fails on them.

### The header: "machine translated — review welcome"

Every file starts with a comment block:

1. `# Rustshot UI strings: <name in itself> (<English name>).`
2. The state of the translation, one of:
   - `# machine translated — review welcome` — written or drafted by a
     machine, not yet read through by a fluent speaker. Every file starts
     here.
   - `# reviewed by @handle (<version>) — review welcome` — a fluent
     speaker went through the whole file for that release.
   Keep the em dash and the words "review welcome" either way: a review
   of the review is welcome too.
3. `# Translators:` the GitHub handles of whoever wrote or reviewed it.
4. `# Terms:` your choices for the glossary's main terms (capture,
   screenshot, recording, overlay, annotation, object, pin, History, Edit
   window, Quick Redact, crop, trim, upload, shortcut, camera bubble), so
   a reviewer can check consistency at a glance.

A partly reviewed file stays "machine translated" until the whole file
is read; say what was reviewed in the pull request instead.

## Try it in the app

The language must be one this build knows: a build lists the languages
whose `lang/<tag>.txt` was in its source tree.

**A language that is already in a release.** Point `RUSTSHOT_DATA_DIR` at a
folder of your own (an absolute path) with your copy in `lang/<tag>.txt`;
that file is read over the downloaded one, key by key. The app looks at
the file at most once a second and switches when it changes, so you can
edit and watch:

```powershell
# Windows (PowerShell). Quit the running Rustshot first (tray › Quit
# Rustshot): the variable only reaches a Rustshot started from this shell.
mkdir C:\rs-lang\lang
copy lang\de.txt C:\rs-lang\lang\de.txt
$env:RUSTSHOT_DATA_DIR = "C:\rs-lang"
& "$env:LOCALAPPDATA\rustshot\rustshot.exe"   # the installer's location; or your own build
```

```bash
# Linux / macOS
mkdir -p ~/rs-lang/lang && cp lang/de.txt ~/rs-lang/lang/
RUSTSHOT_DATA_DIR=~/rs-lang rustshot
```

Then Settings › General › Language › your language (or
`language = "de"` in config.toml). Pointing `RUSTSHOT_DATA_DIR` at a clone
of the repository works too: its `lang/` is used as it is.

**A new language** (not in any release yet): add `lang/<tag>.txt` to a
clone (with `@name`, `@dir`, `@plural`), build from source (README ›
Building from source) and run that build with `RUSTSHOT_DATA_DIR` set to
the clone. Until the file is released the app cannot download it, which
is why the override is needed.

Things to look at: every Settings section (and its dialogs: Reset to
defaults…, Change shortcuts…, hiding the tray icon), the overlay toolbar
tooltips and the Text mode / Quick Redact messages, History (empty, with
captures, a search, the Delete and Clear dialogs), the Edit window and its
Resize… and Canvas… dialogs, the recording panel and bar, the tray menu,
the notices after a copy, save, upload. Right-to-left languages: the
whole window should be mirrored, text right-aligned.

## Fit tests

The layout tests render every Settings section and dialog, the overlay
toolbar tooltips, History, the Edit window, the capture launcher, the
update dialog, the Linux MP4 encoder dialog, the notices and the recording
panel, at several UI scales, and fail when a control leaves its window,
two controls overlap, or a button's label had to be cut. They always run
in English and three pseudo-languages (40 % longer than English, accented,
Hebrew and Arabic letters). `RUSTSHOT_FIT_LANGS` adds real catalogs: a
comma-separated list of tags, each read from the source tree's
`lang/<tag>.txt` (the file must be there when the tests are built), or
`all` for every file there.

```powershell
# Windows (PowerShell), from the repository
$env:RUSTSHOT_FIT_LANGS = "de"            # or "de,fi,ru"
cargo test --release fit                   # every *fit* test, ~20 s after the build
```

```bash
RUSTSHOT_FIT_LANGS=de,fi cargo test --release fit
```

The tests that matter (all match `fit`): `every_section_fits`,
`dialogs_fit_in_long_languages` (Settings), `tooltips_fit_in_long_languages`
(overlay toolbar), `panel_text_fits_in_long_languages` and
`grip_hint_fits_in_long_languages` (recording panel),
`details_panel_fits_in_long_languages` (History's details panel),
`fits_in_long_languages` (History, Edit window, launcher, update and
encoder dialogs), `notices_fit_in_long_languages`. Also run
`cargo test --release catalogs_agree_with_english` (your file reads
without a problem). A failure names the language, the screen and the
rectangle, or the label that was cut (`label cut: "Original size"`):
shorten that string and run again.

With `RUSTSHOT_PREVIEW_DIR` set to a folder, the tests also write what they
drew as PNG files (`settings-dialog-<n>-<tag>.png`, `history-…-<tag>.png`,
`edit-…-<tag>.png`, `update-…`, `launcher-<tag>.png`; and every Settings
section as `settings-<nn>-<tag>.png` with
`cargo test --release preview_settings_pseudo_pngs`). Look at them: the
tests catch overlaps and cut labels, not a translation that reads badly
in its place.

Notes:
- Run Arabic, Hindi and Thai on Windows (or macOS): Linux draws those
  scripts without a shaper (Arabic letters unjoined, Indic and Thai marks
  not reordered), much wider than they really are, so its results are not
  meaningful for them. Hebrew, CJK, Cyrillic, Greek and Latin are fine
  everywhere.
- Text in scripts other than ASCII Latin is drawn by the OS's fonts
  (Segoe UI and its fallbacks on Windows), so results can differ a little
  between machines; leave some room.
- A debug build (`cargo test fit` without `--release`) works too, about
  ten times slower.

## Pull requests

1. One language per pull request, titled `lang: <tag> <what>` (`lang: de
   first translation`, `lang: pt-BR review of Settings`, `lang: ja fixes`).
2. Only `lang/<tag>.txt` (and, for a term that needs explaining,
   `i18n/glossary.md`). Strings in `lang/en.txt` are changed by the
   developers; if English is unclear or wrong, open an issue or say so in
   the pull request.
3. Before you open it: `cargo test --release catalogs_agree_with_english`
   and `RUSTSHOT_FIT_LANGS=<tag> cargo test --release fit` pass. If you
   cannot build Rust, say so: CI runs the catalog test, and a maintainer
   will run the fit tests.
4. In the description: who wrote it (machine, you, both), what you
   reviewed, and terms you were unsure of. The header says "machine
   translated — review welcome" unless the whole file was reviewed (above).
5. When English changes, new keys show English in your language until
   someone adds them; a key whose English placeholders changed falls back
   to English and is reported by `catalogs_agree_with_english`, so CI shows
   what to update. Releases do not wait for translations.
6. A new language (one not in languages.md) needs someone willing to keep
   it up to date; open an issue first. Its `@plural` rule must exist in
   `src/i18n/plural.rs` (most do). The Microsoft Store package declares
   every language too: add the tag's Store code (Microsoft's "supported
   languages" list) to `$storeLang` in `packaging/windows/msix.ps1`, or
   the MSIX build stops.

Files are published under the project's license (Apache-2.0); by sending
a translation you agree to that.
