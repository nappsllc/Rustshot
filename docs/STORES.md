# Publishing Rustshot

Push a tag that matches `Cargo.toml` (`git tag v0.1.0 && git push origin v0.1.0`).
`release.yml` builds everything and attaches it to a GitHub Release. Each store
job below runs only after its repository variable `PUBLISH_<X>` is `true`
(Settings › Secrets and variables › Actions). Set the switch last, after a
manual first submission where noted.

## In-app updates vs. stores
- Store builds never update themselves: an MSIX package (Microsoft Store or
  sideload), Flathub, Snap and the Mac App Store build have no daily check,
  and *Check for updates* says the store handles updates. Nothing to declare
  in the listings beyond that.
- winget installs the NSIS `setup.exe`, so it updates in-app like the
  installer (silent `setup.exe /S /RELAUNCH`); winget upgrades keep working.
- Direct downloads (installer, portable, AppImage, tarball) update in-app
  only from releases that carry `SHA256SUMS`: `release.yml` writes it from
  every asset. Keep that step, and never edit or re-upload an asset after
  publishing without regenerating `SHA256SUMS` (the updater refuses on a
  mismatch or missing entry).

## Before any store: screenshots
- [ ] Capture `docs/screenshots/overlay-dark.png` (and `overlay-light.png`) from
      the redesigned UI, 1440×900 or larger; commit to `main`.
- [ ] Add a `<release>` entry to packaging/linux/io.github.nappsllc.rustshot.metainfo.xml
      for every tag (newest first).

## Microsoft Store (MSIX)
1. Partner Center account (individual or company): https://partner.microsoft.com/dashboard
2. Reserve the name "Rustshot" › Product identity. Copy into repo **variables**:
   `MSIX_IDENTITY_NAME` (Package/Identity/Name), `MSIX_PUBLISHER` (CN=…),
   `MSIX_PUBLISHER_DISPLAY`, `MSSTORE_APP_ID` (Store ID), `MSSTORE_SELLER_ID`.
   Set the identity variables (`MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER`) before
   pushing the first tag: the release's .msix is built with them, and the
   `msstore` job fails if they are empty.
3. First submission by hand: upload `rustshot-<v>-x64.msix` from the release,
   listing text, screenshots, age rating (all ready to paste in
   [docs/store/msstore-listing.md](store/msstore-listing.md), screenshots in
   `docs/store/screenshots/`), privacy policy URL
   `https://github.com/nappsllc/rustshot/blob/main/PRIVACY.md`, and the
   `runFullTrust` justification ("desktop screenshot and screen recording
   tool: global hotkeys, screen capture") and the microphone/webcam
   justification (see [Recording permissions](#recording-permissions-what-to-declare-and-why)).
4. Azure AD app for the Store API (Partner Center › Account settings › User
   management › Azure AD applications, role Manager). **Secrets**:
   `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`, `MSSTORE_CLIENT_SECRET`.
5. Optional testers' build: self-signed cert with subject = `MSIX_PUBLISHER`,
   exported as base64 PFX → secrets `MSIX_SIGN_PFX`, `MSIX_SIGN_PASSWORD`.
   Export the public certificate as `rustshot-sideload.cer` for testers. They
   must trust it in an elevated PowerShell, in `Cert:\LocalMachine\TrustedPeople`
   (`CurrentUser\TrustedPeople` is not honoured by `Add-AppxPackage`):
   `Import-Certificate -FilePath rustshot-sideload.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople`
6. Set `PUBLISH_MSSTORE=true`.

Note: the unsigned `rustshot-<v>-x64.msix` attached to GitHub Releases is the
Store upload package; end users cannot install it directly.

The package's Start entries (`packaging/windows/AppxManifest.xml.in`):
*Rustshot* (Application `Rustshot`: a plain launch runs the hotkey daemon,
or starts a capture when one runs; it also has the `rustshot.exe` alias and
the startup task "Rustshot hotkeys", TaskId `RustshotDaemon`) and *Rustshot
Settings* (`RustshotSettings`, `settings`). Every entry needs a Start
entry: Partner Center refuses an application with `AppListEntry="none"`
(a "headless" app) unless the app has the *HeadlessAppBypass* waiver.
0.6.5's package was refused for its hidden `RustshotDaemon` entry (the
startup task's, with `daemon`); from 0.6.6 the task is on `Rustshot`.
Keep the TaskId: Windows keeps the user's on/off choice for the startup
task under it, across updates.

## winget
1. Fork https://github.com/microsoft/winget-pkgs under the account that owns the token.
2. Classic PAT with `public_repo` → secret `WINGET_TOKEN`.
3. First manifest by hand: `wingetcreate new <setup.exe URL from the release>`
   — id `nappsllc.rustshot`, installer type `nullsoft`, scope `user`; submit.
4. After it merges, set `PUBLISH_WINGET=true`.

## Known issues
- Screenshots and global hotkeys on Linux are X11-only; Wayland sessions
  need XWayland (stated in metainfo). Screen recording works on Wayland
  through the ScreenCast portal.

## Recording permissions (what to declare, and why)
Rustshot records the screen (MP4 or GIF), optionally with system sound, the
microphone and a camera bubble. Everything is captured only while a
recording runs (the camera also while its bubble is shown before one),
saved locally, never uploaded (PRIVACY.md). Use these justifications in the
submission notes:

| Store | Declared | Justification |
|---|---|---|
| Microsoft Store (MSIX) | `DeviceCapability` `microphone`, `webcam` (`packaging/windows/AppxManifest.xml.in`) | "Screen recordings can include the user's microphone (off by default) and a camera bubble (off by default); both only while recording, when the user turns them on." System sound (WASAPI loopback) and the screen (desktop duplication) need no capability; `runFullTrust` already covers them ("desktop screenshot and screen recording tool: global hotkeys, screen capture"). |
| Mac App Store | `com.apple.security.device.audio-input`, `com.apple.security.device.camera` (`packaging/macos/mas.entitlements`), with `NSMicrophoneUsageDescription` and `NSCameraUsageDescription` in Info.plist (`bundle.sh`) | Same text as above. The screen comes through ScreenCaptureKit and the Screen Recording privacy permission (no entitlement); system sound through ScreenCaptureKit's audio. App Privacy stays "Data not collected": recordings never leave the Mac. |
| Developer ID (dmg, Homebrew) | `packaging/macos/devid.entitlements` (audio-input, camera; no sandbox), signed with `--options runtime` in `ci.yml` | Without them the hardened runtime denies the microphone and camera silently. |
| Flathub | `--socket=pulseaudio`, `--talk-name=org.kde.StatusNotifierWatcher` (the tray icon) | System sound (the output's monitor source) and the microphone, read with libpulse-simple while recording (PipeWire's Pulse server works). The screen and the camera come through xdg-desktop-portal (ScreenCast, Camera), which ask the user; no `--device=all` and no PipeWire socket (the portals hand over the stream). |
| Snap Store | plugs `audio-record`, `pulseaudio` (sound), `camera` (bubble, V4L2 on X11), `desktop` (portals), `unity7` (the StatusNotifierItem tray icon; auto-connected) | `audio-record` and `camera` are not auto-connected: ask for auto-connection in the forum request with the text above, or users run `snap connect rustshot:audio-record` / `rustshot:camera`. No `screencast-legacy` (see Snap Store below). |
| AUR / deb | optional dependencies only | `ffmpeg` (MP4 with ffmpeg), `libpulse` / `pulseaudio-utils` (sound), `libpipewire` (Wayland, camera portal), `opus` and `bzip2` (MP4 with OpenH264), `curl` (the OpenH264 download). |

MP4 on Linux without ffmpeg (Flatpak, Snap, or no ffmpeg installed): the
first MP4 recording offers to download Cisco's OpenH264 (on the user's
request, from `ciscobinary.openh264.org`, checked against a SHA-256 pinned
in the source, kept in the app's data folder). Rustshot never bundles
OpenH264. The Flatpak does not add the `org.freedesktop.Platform.ffmpeg-full`
extension (it has only the libraries, not the `ffmpeg` command Rustshot
runs) or the runtime's `openh264` extension (a different OpenH264 build
than the one pinned and verified in the source); revisit both if Rustshot
ever links libavcodec or accepts the runtime's library. The snap stages
`libpulse0`, `libopus0`, `libpipewire-0.3-0t64` (+ modules), `bzip2` and
`curl` for the same reasons, and does not stage ffmpeg (size).

## Flathub
1. Wait for the screenshots, then open a PR to https://github.com/flathub/flathub
   (branch `new-pr`) with `io.github.nappsllc.rustshot.yml` copied from
   `packaging/flatpak/`, its `dir` source replaced by
   `type: git, url: https://github.com/nappsllc/rustshot.git, tag: v<x.y.z>, commit: <sha>`,
   plus `cargo-sources.json`. Flathub verifies `io.github.nappsllc` via the GitHub org.
2. After acceptance, update the Flathub repo per release (tag + commit +
   regenerated `cargo-sources.json`) or add `x-checker-data`.
3. Regenerate sources whenever `Cargo.lock` changes:
   `python flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json`.

## Snap Store
1. https://snapcraft.io account; `snapcraft register rustshot`.
2. `snapcraft export-login --snaps=rustshot --channels=edge,stable --acls=package_upload -`
   → secret `SNAPCRAFT_STORE_CREDENTIALS`.
3. Set `PUBLISH_SNAP=true`. Builds land on `edge`; promote with
   `snapcraft release rustshot <rev> stable`.
4. Plugs: `x11`, `network`, `home`, `desktop`, `pulseaudio` (auto-connected),
   `camera` and `audio-record` (`snap connect rustshot:camera`,
   `snap connect rustshot:audio-record` unless the store grants
   auto-connection; see [Recording permissions](#recording-permissions-what-to-declare-and-why)).
   Recording on Wayland and the camera there go through xdg-desktop-portal
   (ScreenCast, Screenshot, Camera), reached through the `desktop` plug; the
   snap does not plug `screencast-legacy` (a privileged interface that needs
   a store review and is only for apps that bypass the portal).

## AUR
1. https://aur.archlinux.org account with an SSH key; private key → secret `AUR_SSH_KEY`.
2. First push creates the package: the release job clones
   `ssh://aur@aur.archlinux.org/rustshot-bin.git` (empty repo is fine).
3. Set `PUBLISH_AUR=true`.

## Mac App Store
1. Apple Developer Program membership (https://developer.apple.com/programs/).
2. Identifiers › App ID `io.github.nappsllc.rustshot` (platform macOS).
3. Certificates: "Apple Distribution" and "Mac Installer Distribution"
   (shows as "3rd Party Mac Developer Installer"). Export both as .p12 with one
   password. **Secrets**: `MAS_APP_CERT_P12`, `MAS_INSTALLER_CERT_P12` (base64),
   `MAS_CERT_PASSWORD`, `APPLE_TEAM_ID`.
4. Profiles › Mac App Store profile for the App ID → secret
   `MAS_PROVISION_PROFILE` (base64).
5. App Store Connect › new macOS app with that bundle id; privacy URL as above;
   App Privacy: "Data not collected".
6. App Store Connect API key (Users and Access › Integrations, role App
   Manager) → secrets `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_KEY_P8` (file contents).
7. Before the first upload, run the sandboxed build locally and verify:
   capture works after granting Screen Recording, hotkeys fire, Save to
   Pictures works, **Upload works** (it spawns `/usr/bin/curl`; if blocked,
   the uploader must move to NSURLSession before submitting).
8. Set `PUBLISH_MAS=true`; submit each uploaded build for review in App Store Connect.

## Developer ID (dmg) — needed for Homebrew
Secrets `APPLE_CERT_P12`, `APPLE_CERT_PASSWORD`, `APPLE_ID`, `APPLE_TEAM_ID`,
`APPLE_APP_PASSWORD` (see the "Sign and notarize" step in `ci.yml`).

## Homebrew
1. Create public repo `nappsllc/homebrew-tap` (users run `brew install --cask nappsllc/tap/rustshot`).
2. Fine-grained PAT with Contents: read/write on that repo → secret `HOMEBREW_TAP_TOKEN`.
3. Requires the notarized dmg (Developer ID secrets above). Set `PUBLISH_HOMEBREW=true`.
