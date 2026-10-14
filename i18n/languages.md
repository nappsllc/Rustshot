# Rustshot languages

35 languages: English (the source, `lang/en.txt`, built in) and the 34
below. Each is one file, `lang/<tag>.txt`, published with every release as
`lang-<tag>.txt` and downloaded by the app the first time it is needed.
How to write and test one: [docs/translating.md](../docs/translating.md).
Terms: [glossary.md](glossary.md).

## Tags

The tag is the file name, and it is what `language = "<tag>"` in config.toml
and Settings › General › Language use. Write it the BCP 47 way: language in
lower case, script in title case, region in upper case (`pt-BR`, `zh-Hans`,
`sr-Latn`). The app compares tags without regard to case, but the file name
is also the release asset's name, so keep the spelling below.

A tag is a language only, with a script or a region only where one language
needs two files (Portuguese, Chinese) or one writing system must be chosen
(Serbian). No `de-DE`, `fr-FR`, `es-ES`: one German, one French, one Spanish.

### How the system's language picks a file

With `language = ""` (the default) the app asks the OS for its preferred UI
languages, best first (Windows `GetUserPreferredUILanguages`, macOS
`CFLocaleCopyPreferredLanguages`, Linux `LANGUAGE` / `LC_ALL` /
`LC_MESSAGES` / `LANG`), and takes the first one that matches
(`src/i18n/mod.rs`, `best_in`):

1. The tag is normalized: the encoding and modifier dropped (`pt_BR.UTF-8`,
   `sr_RS@latin`), `_` read as `-`, case ignored. `C` and `POSIX` are
   skipped.
2. An exact match with a file's tag (`pt-BR` = `pt-BR`, `zh-hant` =
   `zh-Hant`).
3. The special cases:
   - `zh-*` with `Hant`, `TW`, `HK` or `MO` → `zh-Hant`; any other `zh`
     (`zh-CN`, `zh-SG`, `zh-Hans-CN`, plain `zh`) → `zh-Hans`.
   - `pt-PT` → `pt-PT`; any other `pt` (`pt-BR`, plain `pt`, `pt-AO`,
     `pt-MZ`…) → `pt-BR`.
   - `no`, `nn` (Nynorsk), `nb-NO` → `nb`.
   - `sr` and `sh` in any script or region (`sr-Latn-RS`, `sr-Cyrl-RS`,
     `sr_RS@latin`) → `sr-Latn` (the only Serbian file).
4. The first file with the same language subtag (`de-AT`, `de-CH` → `de`;
   `es-MX`, `es-419` → `es`; `fr-CA` → `fr`; `ar-EG` → `ar`; `he-IL` → `he`).
5. Otherwise the next preferred language; English when none matches.

What the OSes report, for reference: Windows `de-DE`, `pt-BR`, `pt-PT`,
`zh-CN`, `zh-TW`, `zh-HK`, `nb-NO`, `sr-Latn-RS`, `he-IL`; macOS `de`,
`pt-BR`, `pt-PT`, `zh-Hans-CN`, `zh-Hant-TW`, `nb`, `sr-Latn`; Linux
`de_DE.UTF-8`, `pt_BR.UTF-8`, `zh_TW.UTF-8`, `nb_NO.UTF-8`,
`sr_RS.UTF-8@latin`. All of these land on the files below.

Known gaps (not worth a file yet): Serbian Cyrillic users get the Latin
file; African and Asian Portuguese (`pt-AO`, `pt-MZ`, `pt-TL`), whose CLDR
parent is `pt-PT`, get `pt-BR`.

## The 34 languages

`@name` is exactly what the file's `@name` line says and what the language
menu shows. `@plural` is the rule name the loader knows
(`src/i18n/plural.rs`); an unknown name silently means `one_other`, so copy
it from here. Forms are the plural lines the file needs for each plural key.

| Tag | Language | `@name` | `@dir` | `@plural` | Forms | Script |
|---|---|---|---|---|---|---|
| `ar` | Arabic | العربية | rtl | `arabic` | zero one two few many other | Arabic |
| `bg` | Bulgarian | Български | ltr | `one_other` | one other | Cyrillic |
| `cs` | Czech | Čeština | ltr | `czech_slovak` | one few other | Latin |
| `da` | Danish | Dansk | ltr | `one_other` | one other | Latin |
| `de` | German | Deutsch | ltr | `one_other` | one other | Latin |
| `el` | Greek | Ελληνικά | ltr | `one_other` | one other | Greek |
| `es` | Spanish | Español | ltr | `one_other` | one other | Latin |
| `fi` | Finnish | Suomi | ltr | `one_other` | one other | Latin |
| `fr` | French | Français | ltr | `zero_one_other` | one other | Latin |
| `he` | Hebrew | עברית | rtl | `hebrew` | one two other | Hebrew |
| `hi` | Hindi | हिन्दी | ltr | `zero_one_other` | one other | Devanagari |
| `hr` | Croatian | Hrvatski | ltr | `bcs` | one few other | Latin |
| `hu` | Hungarian | Magyar | ltr | `one_other` | one other | Latin |
| `id` | Indonesian | Bahasa Indonesia | ltr | `none` | other | Latin |
| `it` | Italian | Italiano | ltr | `one_other` | one other | Latin |
| `ja` | Japanese | 日本語 | ltr | `none` | other | Japanese |
| `ko` | Korean | 한국어 | ltr | `none` | other | Hangul |
| `lt` | Lithuanian | Lietuvių | ltr | `lithuanian` | one few other | Latin |
| `nb` | Norwegian Bokmål | Norsk bokmål | ltr | `one_other` | one other | Latin |
| `nl` | Dutch | Nederlands | ltr | `one_other` | one other | Latin |
| `pl` | Polish | Polski | ltr | `polish` | one few many other | Latin |
| `pt-BR` | Portuguese (Brazil) | Português (Brasil) | ltr | `zero_one_other` | one other | Latin |
| `pt-PT` | Portuguese (Portugal) | Português (Portugal) | ltr | `one_other` | one other | Latin |
| `ro` | Romanian | Română | ltr | `romanian` | one few other | Latin |
| `ru` | Russian | Русский | ltr | `east_slavic` | one few many other | Cyrillic |
| `sl` | Slovenian | Slovenščina | ltr | `slovenian` | one two few other | Latin |
| `sr-Latn` | Serbian (Latin) | Srpski (latinica) | ltr | `bcs` | one few other | Latin |
| `sv` | Swedish | Svenska | ltr | `one_other` | one other | Latin |
| `th` | Thai | ไทย | ltr | `none` | other | Thai |
| `tr` | Turkish | Türkçe | ltr | `one_other` | one other | Latin |
| `uk` | Ukrainian | Українська | ltr | `east_slavic` | one few many other | Cyrillic |
| `vi` | Vietnamese | Tiếng Việt | ltr | `none` | other | Latin |
| `zh-Hans` | Chinese (Simplified) | 简体中文 | ltr | `none` | other | Han (Simplified) |
| `zh-Hant` | Chinese (Traditional) | 繁體中文 | ltr | `none` | other | Han (Traditional) |

Notes on the forms:

- `[other]` is always written, even where the rule never picks it for
  whole numbers (Russian, Ukrainian, Polish use one/few/many for counts;
  `other` is the loader's fallback for a missing form, and English's after
  that).
- `zero_one_other` puts 0 and 1 in `[one]` (French, Brazilian Portuguese,
  Hindi: "0 capture"); there is no `[zero]` line for them.
- Arabic uses all six forms; it is the only language in this list with a
  `[zero]` line (0 items).
- Languages with no plural (`none`) write only `[other]`.

Left out of the 35 (first candidates when someone offers to maintain one):
Slovak (`sk`, `czech_slovak`), Estonian (`et`, `one_other`), Latvian (`lv`,
`latvian`). Their rules are already in `plural.rs`; adding one is a new
file and nothing else.

## Who translates what: four groups

For four parallel translators (or translation passes), about the same
amount of difficulty each: every group has one or two scripts that need
the OS text path and a careful look at the fit previews, and related
languages stay together so their terms agree (the two Portugueses, the
two Chinese, the Scandinavian three, Serbian/Croatian/Slovenian, Russian
and Ukrainian).

| Group | Languages | Scripts | Watch out for |
|---|---|---|---|
| A (9) | `de`, `nl`, `sv`, `nb`, `da`, `fi`, `ru`, `uk`, `ja` | Latin, Cyrillic, Japanese | The longest strings (German, Finnish, Russian, Dutch compounds): the fit tests fail first here. Russian/Ukrainian one/few/many. Japanese: no spaces, full-width punctuation, no plural. |
| B (9) | `fr`, `es`, `it`, `pt-BR`, `pt-PT`, `ro`, `el`, `ar`, `he` | Latin, Greek, Arabic, Hebrew | Both right-to-left languages: test the mirrored previews on Windows (Linux draws Arabic unshaped). Arabic six plural forms. pt-BR vs pt-PT vocabulary (tela/ecrã, arquivo/ficheiro, baixar/transferir). French spaces before `: ; ? !`. |
| C (8) | `pl`, `cs`, `hu`, `sr-Latn`, `hr`, `sl`, `bg`, `hi` | Latin, Cyrillic, Devanagari | Slavic plurals (Polish many, Slovenian two) and case endings around placeholders. Serbian/Croatian/Slovenian terms kept apart (not copied across). Hindi: Devanagari shaping on Windows only; English technical terms often kept. |
| D (8) | `tr`, `lt`, `id`, `vi`, `ko`, `th`, `zh-Hans`, `zh-Hant` | Latin, Hangul, Thai, Han | Turkish dotted/dotless i (upper-cased headings: `i18n::upper` handles it). Thai has no spaces between words (line breaks). zh-Hant is Taiwan usage, not a character conversion of zh-Hans (屏幕/螢幕, 文件/檔案, 视频/影片). Korean spacing. |

Each group's translator owns the group's files end to end: the
translation, the "machine translated — review welcome" header, the fit
tests in those languages (docs/translating.md), and the glossary entries
for them if a term needs a note.
