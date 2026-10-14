# Microsoft Store listing (Partner Center, en-US)

Text to paste into Partner Center for Rustshot 0.4.1. Each field's block is
the exact text. The limits are Partner Center's current Store listing
limits for MSIX desktop apps; recheck them in the form, which counts for
you. Everything here describes the **Store (MSIX) build**, which differs
from the installer and portable builds in three ways:

- **No Win+Shift+S / Print Screen takeover.** A packaged app can't register
  the `ms-screenclip:` handler (docs/guide.md, *Win+Shift+S and Print
  Screen*). The listing doesn't claim it.
- **The tray icon off doesn't mean 0 MB.** In the Store build the daemon
  keeps running hidden so the hotkeys still work.
- **No update check.** The Store handles updates, and *Check for updates*
  says so.

Related: [docs/STORES.md](../STORES.md) (the submission steps),
[PRIVACY.md](../../PRIVACY.md), `packaging/windows/AppxManifest.xml.in`.

---

## Product identity

| Field | Value | Limit |
|---|---|---|
| Product name (reserved) | `Rustshot` | 256 |
| Short title | `Rustshot` | 50 |
| Sort title | *(leave empty)* | 255 |
| Voice title | *(leave empty)* | 255 |
| Developed by | `PowerNapps` | 255 |

## Short description

Limit: 1,000 characters. Windows shows the opening part on the listing and
in search, so the first ~270 characters do the work. This text is 267
characters.

```text
Capture any part of your screen, mark it up and share it in seconds. Rustshot is a tiny, open-source screenshot and screen recording tool: annotations that stay editable, text recognition (OCR), Quick Redact, MP4/GIF recording with camera and microphone, and History.
```

## Description

Limit: 10,000 characters, plain text (no Markdown, HTML or links). This text
is about 3,450 characters.

```text
Rustshot is a fast, good-looking tool for screenshots, annotation and screen recording. Press Win+Shift+X, drag across what you want, mark it up and copy, save or share it. Rustshot is a single small native app with no web view and no account, and it is open source under the GPL.

CAPTURE
• A region, a window, a whole monitor or every screen. Hover a window to outline it, then click to take exactly that window, menu or tooltip.
• Resize handles, arrow-key nudging, a live size readout and a pixel magnifier that shows the color under the pointer.
• Freeform: draw a lasso instead of a rectangle; outside it the PNG is transparent.
• Trim to content cuts uniform margins off the selection with one key.
• A capture launcher with a 0–60 second delay, plus "accept on select" to copy or save the moment you let go.
• Multi-monitor aware, follows each display's scaling, and tone-maps HDR displays so captures look as they do on screen.

ANNOTATE AND EDIT
• Pencil, line, arrow, rectangle, ellipse, highlighter, text, callouts, numbered tags, pixelate, blur, invert, spotlight and magnifier.
• Annotations stay editable: select, move, resize, restyle, restack and duplicate them, with snapping and unlimited undo.
• Text in sans, serif or mono, with bold, italic, underline, strikethrough and alignment.
• The Edit window adds rotate, flip, high-quality resize and a canvas: padding, a solid color or a soft "presentation" gradient, rounded corners and a drop shadow. Every change can be undone; the original is kept.

COPY TEXT (OCR) AND QUICK REDACT
• Copy the text from any area with the OCR built into Windows. Nothing is uploaded.
• Text mode shows the words it found: drag to select, double-click a word, triple-click a line, then copy.
• Quick Redact (Shift+R) finds e-mail addresses, phone and card numbers, IBANs, IP addresses, URLs with tokens, API keys and passwords, and covers them all in one undo step.

RECORD
• Record a region, a window or a screen to MP4 (30 or 60 fps) or GIF.
• Add system sound, your microphone and a draggable camera bubble, with optional background blur computed on your PC.
• A 3-2-1 countdown, plus pause, mute and discard while recording.

SHARE
• Copy to the clipboard, or save as PNG, JPEG or BMP into dated folders with your own file-name pattern.
• Save As, Save to Desktop, Open with any app, the Windows share sheet, or drag the thumbnail into another app.
• Optional uploads to Imgur or to your own S3-compatible storage, with the link copied for you, only when you choose Upload.
• Pin a capture on top of every window to compare against it.

HISTORY
• Every capture is kept on your PC for 30 days (up to 2 GB) with its annotations still editable. Search, filter and reopen any of them.

MADE TO STAY OUT OF THE WAY
• Small and light: it uses a few megabytes of memory in the background and draws only the pixels that change.
• Dark and light themes that follow Windows, and any accent color.
• Every hotkey and editor key can be remapped. Defaults: Win+Shift+X region, Win+Shift+F full screen, Win+Shift+E window, Win+Shift+G record, Win+Shift+Z copy text, Win+Shift+H History.

PRIVACY
No account, no telemetry, no ads. Captures and recordings stay on your PC unless you choose Upload. The microphone and camera are used only while you record with them turned on.

Rustshot is free software under the GNU General Public License v3.0. Source code, issues and the user guide: github.com/nappsllc/rustshot
```

## Product features

Up to 20, 200 characters each. These are 16, the longest is 83.

```text
Capture a region, window, monitor or every screen with one hotkey
Click to capture exactly one window, menu or tooltip
Freeform lasso selection with a transparent outside
Pixel magnifier with the color under the pointer
Trim the selection to its content with one key
14 annotation tools, from arrows and callouts to blur, spotlight and magnifier
Annotations stay editable: move, resize, restyle and undo without limit
Copy text from any area with Windows OCR; select words in Text mode
Quick Redact covers e-mails, phone and card numbers, keys and passwords in one step
Record MP4 or GIF with system sound, microphone and a camera bubble
Camera background blur computed on your PC
Rotate, flip, resize and add a presentation canvas in the Edit window
History keeps 30 days of captures, searchable and re-editable
Pin captures on top of every window
Optional upload to Imgur or your own S3-compatible storage
Dark and light themes, any accent color, every key remappable
```

## Search terms

Up to 7 terms, 30 characters each, and no more than 21 words across all of
them (these use 11). They don't name other products.

```text
screenshot
screen capture
screen recorder
snip
OCR text capture
annotate image
GIF recorder
```

## What's new in this version (0.4.1)

Limit: 1,500 characters. This text is about 340.

```text
• Smaller: the app is about 320 KB smaller, with Rustshot's own PNG encoder and decoder and compressed built-in data.
• Selection handles on annotations now have an even border at every display scale.
• Very large images (up to 200 megapixels) open safely, and a picture that is too big is refused cleanly instead of running out of memory.
```

If translations or the separate data files (language files, blur model)
ship in 0.4.1, add a line for each before submitting (still within 1,500
characters), for example:

```text
• Rustshot now speaks your language: choose it in Settings › General › Language, or follow Windows.
• Translations and the camera's background-blur model download only when you first need them.
```

## Category

**Primary category: Productivity. No subcategory.**

Why: people find screenshot and capture tools by task ("take a screenshot",
"record my screen", "copy text from an image"), and Productivity is where
the Store groups that kind of utility. Most of Rustshot is work-flow
features (OCR, redaction, sharing, history, hotkeys), not photo
management or video editing. **Photo & video** would be the fallback: it
fits the recording and image editing, but there the app competes with photo
editors and viewers, whose users want something else. Don't choose
"Developer tools" (too narrow) or "Utilities & tools" unless Productivity
is rejected. You can only pick one category, so pick Productivity.

## System requirements

Partner Center's **System requirements** section:

| Item | Minimum | Recommended |
|---|---|---|
| Keyboard | ✔ (global hotkeys) | |
| Mouse | ✔ (selecting areas, drawing) | |
| Touch screen | | |
| Camera | | ✔ (optional camera bubble) |
| Microphone | | ✔ (optional narration) |
| Memory | Not specified | Not specified |
| DirectX | Not specified | Not specified |
| Video memory | Not specified | Not specified |

Additional system requirements (free text, 200 characters each):

Minimum:

```text
Windows 10 version 2004 (build 19041) or later, 64-bit (x64). Copy text (OCR) needs a Windows OCR language, installed with your display language.
```

Recommended:

```text
Windows 11. A camera and microphone are optional, for recordings with a camera bubble or narration.
```

The package declares `TargetDeviceFamily Windows.Desktop`, `MinVersion
10.0.19041.0` and x64 only, so Partner Center fills in the OS and
architecture itself; Arm64 PCs run it under Windows 11's x64 emulation.

## Store listing URLs and legal

| Field | Value | Limit |
|---|---|---|
| Privacy policy URL | `https://github.com/nappsllc/rustshot/blob/main/PRIVACY.md` | — |
| Website | `https://powernapps.net` | — |
| Support contact info | `https://github.com/nappsllc/rustshot/issues` | — |
| Copyright and trademark info | `© 2026 PowerNapps` | 200 |

Additional license terms (limit 10,000 characters):

```text
Rustshot is free software, licensed under the GNU General Public License, version 3 only (GPL-3.0-only): https://www.gnu.org/licenses/gpl-3.0.html

The source code, the license text and the third-party notices (Inter font, SIL Open Font License 1.1; Lucide icons, ISC; MediaPipe Selfie Segmentation model, Apache-2.0) are at https://github.com/nappsllc/rustshot (LICENSE and THIRD_PARTY_NOTICES.md).
```

## Screenshots

Desktop, PNG. The Store wants 1366 × 768 or larger and allows up to 10;
these are 1920 × 1080. Captions can be up to 200 characters. Upload them in
this order: the first is the one search results show.

All of them are Rustshot's own renders (the preview tests), not captures of
a real desktop: the overlay shots are drawn over a sample inbox on a
neutral wallpaper (`src/editor/store_tests.rs`), and the History, Edit and
Settings windows are placed on the same wallpaper. To make them again:

```bash
RUSTSHOT_PREVIEW_DIR=<dir> cargo test --bin rustshot -- --include-ignored store_screenshot preview_settings_pngs
python tools/store-screenshots.py <dir>
```

| # | File | Caption |
|---|---|---|
| 1 | [01-overlay-dark.png](screenshots/01-overlay-dark.png) | Mark up a capture right where you took it: highlights, boxes, arrows, numbered tags and callouts. |
| 2 | [02-overlay-light.png](screenshots/02-overlay-light.png) | Light theme. Circle it, blur a name, zoom in with the magnifier and add a note. |
| 3 | [03-text-mode.png](screenshots/03-text-mode.png) | Text mode: Windows OCR finds the words in the area. Drag across them and copy just the text you need. |
| 4 | [04-quick-redact.png](screenshots/04-quick-redact.png) | Quick Redact hides e-mails, phone and card numbers, IP addresses and keys in one step (Blur style shown). |
| 5 | [05-recording-setup.png](screenshots/05-recording-setup.png) | Record MP4 or GIF with system sound, your microphone and a camera bubble you can drag anywhere. |
| 6 | [06-history.png](screenshots/06-history.png) | History keeps every capture for 30 days, with its annotations still editable. |
| 7 | [07-edit-canvas.png](screenshots/07-edit-canvas.png) | The Edit window: rotate, resize, and put a capture on a presentation canvas with rounded corners and a shadow. |
| 8 | [08-settings.png](screenshots/08-settings.png) | Every hotkey can be changed in Settings. |

## Restricted capability: runFullTrust

Partner Center › Submission options › *Restricted capabilities*: the text
for **runFullTrust**.

```text
Rustshot is a desktop screenshot and screen recording tool packaged from a Win32 app (Windows.FullTrustApplication). Full trust is needed to: register global hotkeys (RegisterHotKey) that work while other apps have focus; capture the screen with GDI and DXGI Desktop Duplication, including other apps' windows and every monitor; draw its capture overlay as a topmost layered window over the whole desktop; show a notification-area (tray) icon; record system sound through WASAPI loopback; recognize text with the Windows.Media.Ocr API on the user's selection; and write captures to the user's Pictures folder. Nothing is captured until the user presses a hotkey or chooses Capture, and nothing leaves the PC unless the user chooses Upload.
```

The microphone and webcam are ordinary device capabilities (no restricted
review), but certification reads this too: see the notes below.

## Notes for certification

Partner Center › Submission options › *Notes for certification*. Keep it
under about 2,000 characters. This text is about 1,990.

```text
No sign-in, account or test credentials are needed. Rustshot is a screenshot and screen recording tool that runs in the background.

HOW TO TEST
1. Launch Rustshot from Start. It starts in the background with an icon in the notification area. The tray menu has Capture…, History… and Settings…. "Rustshot Settings" in Start opens Settings directly.
2. Press Win+Shift+X. The capture overlay covers the screen: drag a region (or click a window), draw with the toolbar (R rectangle, A arrow, T text), then Ctrl+C copies, Enter or Ctrl+S saves to Pictures\rustshot, Esc cancels.
3. Other default hotkeys: Win+Shift+F full screen (saved at once), Win+Shift+E window, Win+Shift+Z copy text from an area (Windows OCR), Win+Shift+G start/stop recording, Win+Shift+H History, Ctrl+Alt+Shift+Q quit. If another app already holds a key, Rustshot moves it to a spare key and says which in a notification; all keys can be changed in Settings › Shortcuts.
4. Recording: in the overlay press 2 (Video), choose Region/Window/Screen and Start recording; stop with Win+Shift+G or the pill. The file is saved next to the screenshots.

PERMISSIONS
• Microphone and camera are off by default. They are used only when the user turns on Microphone or Camera in the recording panel (the camera also while its bubble is shown before recording); Windows asks for permission the first time.
• The startup task "Rustshot hotkeys" starts the background process at sign-in so the hotkeys work; it can be turned off in Windows Settings › Apps › Startup.

NETWORK (internetClient)
• No telemetry and no update check in the Store build (the Store updates it).
• Uploads go to Imgur or the user's own S3-compatible storage only when the user chooses Upload (Ctrl+U) or Copy link on a recording.
• Language files and the camera background-blur model are downloaded on demand from github.com (the project's releases) when the user first needs them. Each file is checked against a SHA-256 built into the app; Settings › General › "Download language and blur files" turns this off. Nothing else is downloaded except an upload the user starts.

Privacy policy: https://github.com/nappsllc/rustshot/blob/main/PRIVACY.md
```

## Age rating (IARC questionnaire)

Category: **"Utility, Productivity, Communication, or Other"** (not a game,
not a social network). Suggested answers:

| Question (paraphrased) | Answer | Why |
|---|---|---|
| Violence, blood, fear, sexual content, nudity, crude humor, profanity, drugs, alcohol, tobacco, gambling | No | The app has no content of its own; it shows only what the user captures. |
| Does the app let users interact or exchange content with other users (text, voice, images)? | **Yes**, to be safe | Upload puts a user's capture on Imgur, or on their own storage, at a public link that they can share. There's no chat, feed or other users' content inside the app, but the upload is user-generated content leaving the device. Answering No could be read as hiding that, and Yes doesn't change the age rating. |
| Does the app share the user's location with other users? | No | |
| Does the app allow purchases of digital goods? | No | Free, no in-app purchases. |
| Does the app offer unrestricted internet access (a browser or search engine)? | No | It only opens the project's own links (GitHub, the privacy policy) in the user's browser. |
| Does the app collect or share personal information? | No | No account and no telemetry. Uploads happen only when the user chooses them and go only to the host the user picked (see PRIVACY.md). |

Expected result: **IARC 3+ / ESRB Everyone / PEGI 3**, with the interactive
element *Users Interact* (or *Shares Info*, depending on the authority) for
the upload. If a rating authority adds an element you didn't expect, that's
the reason; the age rating stays the same.
