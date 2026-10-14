# Rustshot privacy policy

Rustshot does not collect, store or transmit personal data or telemetry.

- Screenshots stay on your computer unless you choose **Upload**. Upload sends
  only the selected image to the destination chosen in Settings › Upload:
  - **Imgur** (the default, https://imgur.com) using Rustshot's public
    client id; imgur's privacy policy then applies to that image.
  - **S3-compatible storage** you configure (Amazon S3, Cloudflare R2,
    MinIO, …): the image goes to your endpoint and bucket only, signed
    with your access key; that provider's terms then apply. Rustshot
    talks to no other server for it.

  The returned link is copied to your clipboard and printed to the console.
  A recording is uploaded only when you click **Copy link** on the notice
  after it, or, if you turn on *Upload recordings automatically*, as soon as
  it is saved (to Imgur, MP4s through its video upload, which imgur then
  transcodes; or to your bucket).
  For each upload Rustshot keeps, on this computer only, the link, where it
  went (for Imgur the *deletehash* it returns, the key that deletes the
  upload again; for S3 the endpoint, region, bucket and object key), the
  time, the kind (image or video) and a 72 × 48 thumbnail: in the
  capture's History entry, or else in `uploads.json` (the last 50) beside
  the history folder (`%LOCALAPPDATA%\rustshot-data`, Linux
  `$XDG_DATA_HOME/rustshot`, macOS `~/Library/Application Support/rustshot`).
  The deletehash is never logged, shown or sent anywhere except in the one
  request **Delete upload…** makes (`DELETE
  https://api.imgur.com/3/image/<deletehash>` with Rustshot's client id);
  for an S3 upload it is one signed `DELETE` of the object to your
  endpoint. You confirm first; once deleted, the link and the record go.
- S3 credentials: the endpoint, region, bucket, access key id, public URL
  base and key prefix are in `config.toml`. The **secret access key** is
  never written to `config.toml`, `uploads.json`, History or any log: it is
  kept in your operating system's credential store (Windows Credential
  Manager, as the generic credential
  `rustshot:s3:<bucket>@<endpoint host>/<access key id>`; on Linux the
  Secret Service, e.g. GNOME Keyring or KWallet — without one Rustshot does
  not save it at all; on macOS the login Keychain) and read from there when
  an upload is signed. Each upload's record keeps the access key **id** it
  used (not the secret), so Delete upload… finds the right secret. The
  secret is used only to compute request signatures and is never sent
  anywhere itself. On Linux, Rustshot talks to the Secret Service with
  its `plain` session algorithm: the secret crosses your local session
  bus (a socket only your user can open) unencrypted, as most apps that
  use the Secret Service do; the keyring itself stores it encrypted.
  On Linux and macOS the signed request's URL and headers go to `curl` on
  its standard input when there is no body to stream there (a delete, a
  whole file it reads itself); otherwise in a file only you can read, in
  a private folder (`$XDG_RUNTIME_DIR/rustshot-curl`), deleted as soon as
  curl is done (a leftover from a crash is removed by the next upload).
  It is never on curl's command line. **Test connection** uploads one tiny
  `rustshot-test-<random>.txt` to your bucket and deletes it again.
- **Open with…** writes an unsaved capture to a private `rustshot-open`
  folder (Windows `%TEMP%`, Linux `$XDG_RUNTIME_DIR` or the cache folder,
  readable by you only) for the app you pick (removed once a day old);
  nothing is sent anywhere.
- Settings are stored locally in `config.toml` in your user config folder.
- History (on by default; off in Settings › Capture › History): each capture
  is kept on this computer only, in `%LOCALAPPDATA%\rustshot-data\history`
  (Linux: `$XDG_DATA_HOME/rustshot/history`, macOS:
  `~/Library/Application Support/rustshot/history`) — the unannotated image,
  its annotations, a thumbnail, and details such as the time, size, source
  window title, saved path and upload link. Entries older than 30 days, or
  past the 2 GB cap, are deleted automatically; **Clear history** deletes all
  of them. Nothing in History is uploaded or synced.
- Update check (direct downloads only; off with `check_updates = false` or in
  Settings): at most once a day, and when you choose *Check for updates*, the
  daemon sends one anonymous request to the GitHub Releases API
  (`api.github.com`) for the latest release. Store, Flathub and Snap installs
  do not check. If you choose **Update**, the new version and its
  `SHA256SUMS` are downloaded from `github.com/nappsllc/rustshot/releases`
  (GitHub redirects the download to its own servers,
  `objects.githubusercontent.com` / `release-assets.githubusercontent.com`).
  These requests carry no data about you or your screenshots; GitHub sees
  your IP address and a `rustshot/<version>` user agent, as with any download.
- Data files on demand (every install, store builds included; off with
  `download_data = false` or Settings › General › *Download language and
  blur files*): the camera bubble's background-blur model (when you first
  choose a blur) and a language file (when you choose a language, or on
  first run when your system language has a translation) are downloaded
  with that release's `SHA256SUMS` from
  `github.com/nappsllc/rustshot/releases/download/v<version>/` (GitHub's
  asset servers as above), checked against the SHA-256 built into
  Rustshot, and kept in `%LOCALAPPDATA%\rustshot-data\{models,lang}`
  (Linux: `$XDG_DATA_HOME/rustshot/…`, macOS: `~/Library/Application
  Support/rustshot/…`). When this version's release does not have the
  file (a build not released yet), one anonymous request to the GitHub
  Releases API (`api.github.com`) asks for the newest release, and the
  file is taken from there only if it has the same SHA-256. The requests
  carry nothing but the file's name and a `rustshot/<version>` user agent;
  no camera picture, screenshot or setting is ever sent. Downloads stay
  off until the config file is read (a broken config file: off).
  Uninstalling keeps these files with the history unless you tick *Also
  remove capture history and downloaded files*.
- Screen recording: the screen, system sound, microphone and camera are
  captured only while a recording runs, and the camera also while its
  bubble is shown before one (Record popover with Camera on). Sound, the
  microphone and the camera are each recorded only when you turn them on;
  your operating system asks for the microphone, camera and (macOS,
  Wayland, Flatpak) screen permission first.
  Recordings (MP4 or GIF) are written straight to your save folder; they
  are uploaded only as described above (**Copy link**, or *Upload
  recordings automatically*), and Rustshot keeps no other copy. Settings › Recording lists the names of your microphones
  and cameras to choose from; it does not open them.
- Show clicks & keys (off by default; Settings › Recording): only while a
  recording runs, Rustshot listens to the mouse buttons and keys system-wide
  (Windows: low-level mouse and keyboard hooks; X11: XInput2 raw events;
  macOS: a listen-only event tap, which needs the Input Monitoring
  permission) to draw a ring where you click and the last shortcuts into
  the video's frames. The listening starts with the recording's first frame
  and stops with the recording. Keys are never written to a file, a log,
  the console or anywhere else, and never sent: each one is kept in memory
  for the 1.8 s it is shown, then dropped. By default only chords with Ctrl,
  Alt or Win (Super, Cmd) are drawn, so typed text never is; with *All
  keys* every key is drawn, except (Windows) while the focused element is a
  password field, which Rustshot asks UI Automation about (the element's
  *IsPassword*; nothing else about it is read). On Wayland other apps'
  input cannot be seen, so nothing is drawn.
- Trim after recording: the trimmed copy replaces the recording in its
  folder; nothing leaves your computer. On Windows the frames from the new
  start to the next key frame are decoded and encoded again locally.
- Scrolling capture: Rustshot scrolls only the window under the selection
  (Windows: wheel messages to that window, or UI Automation's scroll
  pattern of the element there; X11: wheel button presses while the pointer
  is over that window, put back where it was at the end) and grabs the
  selected region after each step. The stitched picture is kept like any
  capture (History, or your save folder).
- OpenH264 (Linux without ffmpeg, only if you choose **Download OpenH264**
  when you first record MP4): Rustshot downloads Cisco's OpenH264 library
  once from Cisco (`http://ciscobinary.openh264.org/`, plain HTTP as Cisco
  serves it; the file is checked against a SHA-256 built into Rustshot) and
  keeps it in `$XDG_DATA_HOME/rustshot/openh264/` (usually
  `~/.local/share/rustshot/openh264/`; inside a Flatpak or Snap sandbox,
  the app's own data folder). The request carries no
  data about you or your recordings; Cisco sees your IP address, as with any
  download.
- Rustshot makes no other network requests.

Questions: https://github.com/nappsllc/rustshot/issues
