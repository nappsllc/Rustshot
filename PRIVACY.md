# Rustshot privacy policy

Rustshot does not collect, store or transmit personal data or telemetry.

- Screenshots stay on your computer unless you choose **Upload**. Upload sends
  only the selected image to imgur (https://imgur.com) using Rustshot's
  public client id; imgur's privacy policy then applies to that image. The
  returned link is copied to your clipboard and printed to the console.
  A recording is uploaded only when you click **Copy link** on the notice
  after it (MP4s through imgur's video upload, which imgur then transcodes).
  For each upload Rustshot keeps, on this computer only, the link, the
  *deletehash* imgur returns (the key that deletes the upload again), the
  time, the kind (image or video) and a 72 × 48 thumbnail: in the
  capture's History entry, or else in `uploads.json` (the last 50) beside
  the history folder (`%LOCALAPPDATA%\rustshot`, Linux
  `$XDG_DATA_HOME/rustshot`, macOS `~/Library/Application Support/rustshot`).
  The deletehash is never logged, shown or sent anywhere except in the one
  request **Delete from Imgur…** makes (`DELETE
  https://api.imgur.com/3/image/<deletehash>` with Rustshot's client id),
  which you confirm first; once deleted, the link and the record go.
- **Open with…** writes an unsaved capture to `rustshot-open` in your
  temporary folder for the app you pick (removed once a day old); nothing
  is sent anywhere.
- Settings are stored locally in `config.toml` in your user config folder.
- History (on by default; off in Settings › Capture › History): each capture
  is kept on this computer only, in `%LOCALAPPDATA%\rustshot\history`
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
- Screen recording: the screen, system sound, microphone and camera are
  captured only while a recording runs, and the camera also while its
  bubble is shown before one (Record popover with Camera on). Sound, the
  microphone and the camera are each recorded only when you turn them on;
  your operating system asks for the microphone, camera and (macOS,
  Wayland, Flatpak) screen permission first.
  Recordings (MP4 or GIF) are written straight to your save folder; they
  are never uploaded (Upload is for screenshots only), and Rustshot keeps
  no other copy. Settings › Recording lists the names of your microphones
  and cameras to choose from; it does not open them.
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
