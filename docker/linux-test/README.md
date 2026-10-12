# Linux test harness (Docker)

Runs rustshot's Linux live tests for real on a Windows (or any) machine with
Docker, and leaves screenshots, recordings and logs to look at.

```powershell
.\docker\linux-test\run.ps1 -Mode x11       # Xvfb + PipeWire
.\docker\linux-test\run.ps1 -Mode wayland   # headless sway + xdg-desktop-portal-wlr
.\docker\linux-test\run.ps1 -Mode shell     # a shell in the image (repo at /src)
.\docker\linux-test\run.ps1 -Mode x11 -Ref worktree   # uncommitted edits too
```

The image (`rustshot-linux-test`, ~3 GB: Debian trixie from the pinned
`rust:1.96` digest, plus sway 1.12 and xdg-desktop-portal-wlr 0.8.4 from
Debian unstable, pinned to those versions, see the Dockerfile) is built on
first use; `-Rebuild`
rebuilds it. The repo is mounted read-only at `/src`; `run.sh` is read from
the mount (editing it needs no rebuild) and puts `git archive <Ref>` (default
`HEAD`, i.e. committed code, so a half-edited tree does not break the run)
in `/work`. Build output is in the named volume `rustshot-target`, the cargo
registry in `rustshot-cargo` (both on Docker's disk; `docker volume rm
rustshot-target` frees it). Outputs go to `-Out` (default
`%USERPROFILE%\rustshot-docker-out`), one folder per mode:

| path                  | what                                                |
|-----------------------|-----------------------------------------------------|
| `<mode>/summary.txt`  | one line per step: `pass` / `fail` / `info`         |
| `<mode>/commit.txt`   | the revision tested                                 |
| `<mode>/logs/`        | every test run, every service, `ignored-tests.txt`  |
| `<mode>/shots/`       | screenshots (`import -window root` / `grim`)        |
| `<mode>/rec/`         | the recording (MP4, a GIF of it, `ffprobe` JSON)    |
| `<mode>/saved/`       | what the daemon saved (capture PNG, recording MP4)  |
| `x11/previews/`       | the bar and camera bubble preview PNGs              |

The exit code is 1 when any `fail` line is in the summary.

Environment knobs (passed through by `run.ps1`): `RS_ONLY=daemon` (x11:
skip the cargo tests), `RS_ONLY=portal` (wayland: stop after the portal
check), `RS_TEST_TIMEOUT` (seconds per test group, default 600),
`RS_GST_CHECK=1` (wayland: also read a picture with GStreamer on the
portal's fd; xdpw 0.8.4 currently dies on that), `RS_WLR_RENDERER`
(default `pixman`), `RS_WL_ANIM=0` (wayland: no background animation, so
every screen cast sees a still screen).

## x11

dbus session, PipeWire + WirePlumber + pipewire-pulse with a null sink
(`rec_sink`, system sound) and a virtual source (`rec_mic`), Xvfb `:99` at
1920x1080x24. Then:

1. `cargo build --release --bins --tests`, `cargo test --release`.
2. The ignored live tests CI runs on Linux, one thread each
   (`live_monitor_capture`, `live_desktop_capture`, `live_window_lifecycle`,
   `interactive_window_renders_rect`, `*window_smoke`, `live_rec_linux_*`,
   `live_oh264_*`; the last downloads OpenH264 from Cisco), then
   `live_cam_linux_*` (SKIP without a camera) and the preview PNGs.
3. Every other ignored test compiled on Linux (benches, previews, network,
   folder pickers, cameras, Wayland excluded) as `extra-*`: informational,
   never fails the run.
4. A real daemon: `RUSTSHOT_INSTANCE=docker-x11-<pid>`, its own
   `XDG_CONFIG_HOME` (`x11/daemon-config.toml`) with unusual hotkeys
   (capture `Ctrl+Alt+Shift+Y`, record `Ctrl+Alt+Shift+U`, quit
   `Ctrl+Alt+Shift+J`; not `Ctrl+Alt+F<n>`, which X turns into VT switches)
   and `record = "Ctrl+R"` in `[shortcuts]`, saving into `x11/saved`. Steps:
   the Settings window (`rustshot settings`); the capture hotkey (else a
   second launch) and Ctrl+A, Ctrl+S, then the notice (`notice-window`: the
   mapped override-redirect "Rustshot notice" window, typed NOTIFICATION,
   bottom-centre; `notice-fades`: unmapped again; screenshot
   `04c-notice.png`); the one-shot `rustshot gui` and
   Escape; a recording: the record hotkey (else the overlay, Ctrl+A,
   Ctrl+R), Enter, a tone into the null sink and a changing desktop, then the
   hotkey (else a click on the bar's Stop), checked with `ffprobe` and a full
   decode; the quit hotkey; last, a mouse drag (400,300 to 1200,700) in a
   one-shot overlay and Ctrl+S: the saved PNG must be 800x400. Each hotkey
   is pressed through XTEST (`xdotool key`) and is its own pass/fail step.
5. Window snapping: an `xmessage` window at 640x360+300+200; in a one-shot
   overlay the pointer hovers it, a click (no drag) and Ctrl+S must save
   exactly the window (its size from `xwininfo`, border included:
   `snap-click-save`); then, over its button, Tab, click, Ctrl+S must save
   one of its parts (`snap-tab-part`).

## wayland

dbus + the PipeWire stack, then `sway` on the headless wlroots backend
(pixman renderer, one 1920x1080 output `HEADLESS-1`, XWayland on, the
background changing twice a second so screen casts have damage),
`xdg-desktop-portal-wlr` (`chooser_type=none`, `output_name=HEADLESS-1`) and
`xdg-desktop-portal` (`portals.conf` prefers wlr; the Access portal from
`xdg-desktop-portal-gtk`, without which xdg-desktop-portal 1.18+ does not
export Screenshot). Then:

1. `portal_screencast.py` drives `org.freedesktop.portal.ScreenCast`
   (CreateSession, SelectSources monitor, Start, OpenPipeWireRemote) with no
   user, holds the session, and `pw-cli ls Node` must list its node.
2. `grim` screenshots.
3. `live_rec_wayland_*` and `live_cam_linux_*` (and any ignored test named
   wayland/portal/pipewire/screencast), among them a recording whose share
   is stopped mid-way (it must end saying "Screen sharing was stopped" and
   keep a whole file). A portal test that skips ("SKIP: [wayland]": no
   display, portal or libpipewire) fails the run (`live-wayland-ran`).
4. The screen cast tests again with the animation stopped: xdpw sends no
   picture at all for a still screen, so the source starts from a
   Screenshot-portal picture and repeats it at the recording's rate.
5. `rustshot full` in that session must write a 1920x1080 PNG (through
   the Screenshot portal; X11 `GetImage` of the XWayland root fails).

Why sway and xdpw from unstable: without a GPU sway renders with pixman (shm
buffers only), and trixie's xdpw 0.7.1 (Ubuntu 24.04 has 0.7 too) refuses
every Start with "unable to receive a valid format from wlr_screencopy" (it
wants a dmabuf format). Docker Desktop's `/dev/dri` is `vgem`, where GBM
cannot allocate, so a GLES2 sway does not help either.

## CI

The `linux-wayland` job in `.github/workflows/ci.yml` builds this image on
`ubuntu-latest` and runs `wayland` mode in it (the runner's own packages hit
the xdpw 0.7 problem above), then `RS_ONLY=daemon x11` on the same build
volume (hotkeys, capture, recording, overlay drag). It is skipped on tags, so it never gates a
release; its outputs are uploaded as `linux-wayland-out`.

## Without the wrapper

```sh
docker build -t rustshot-linux-test docker/linux-test
docker run --rm --init --shm-size=1g -v "$PWD:/src:ro" -v "$HOME/rs-out:/out" \
  -v rustshot-target:/target -v rustshot-cargo:/usr/local/cargo/registry \
  rustshot-linux-test x11
```
