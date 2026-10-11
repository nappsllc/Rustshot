# Linux test harness for rustshot: X11 (Xvfb) and Wayland (headless sway +
# xdg-desktop-portal-wlr), with PipeWire/PulseAudio for sound.
# See docker/linux-test/README.md. Build context: this directory.
FROM rust:1.96@sha256:1f0dbad1df66647807e6952d1db85d0b2bda7606cb2139d82517e4f009967376

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        libx11-dev libxrandr-dev libxext-dev libxfixes-dev \
        xvfb xauth x11-utils x11-xserver-utils xdotool imagemagick \
        sway grim xwayland xdg-desktop-portal xdg-desktop-portal-wlr \
        pipewire pipewire-bin pipewire-pulse wireplumber pulseaudio-utils \
        libopus0 libpulse0 \
        dbus dbus-x11 libglib2.0-bin \
        python3 python3-dbus python3-gi \
        gstreamer1.0-tools gstreamer1.0-pipewire gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
        xclip wl-clipboard ffmpeg bzip2 jq procps psmisc fonts-dejavu ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# sway and xdg-desktop-portal-wlr from Debian unstable (sway 1.12, xdpw
# 0.8.4): with no GPU, sway renders with pixman (shm only), and trixie's
# xdpw 0.7.1 then refuses every ScreenCast Start ("unable to receive a valid
# format from wlr_screencopy": it wants a dmabuf format from screencopy v3).
# xdpw 0.8 captures shm buffers. Pinned low, so nothing else comes from sid,
# and to the versions the harness was made green with: once sid moves on,
# the image build fails instead of quietly testing against something else.
RUN echo "deb http://deb.debian.org/debian sid main" > /etc/apt/sources.list.d/sid.list \
    && printf 'Package: *\nPin: release a=unstable\nPin-Priority: 100\n' > /etc/apt/preferences.d/sid \
    && apt-get update \
    && apt-get install -y --no-install-recommends -t unstable 'sway=1.12-*' 'xdg-desktop-portal-wlr=0.8.4-*' \
    && rm -rf /var/lib/apt/lists/*

# xdg-desktop-portal (1.18+) exports the Screenshot portal only with an
# Access implementation (its permission dialog): xdg-desktop-portal-gtk,
# from trixie. `rustshot full` on Wayland and the screen cast's first
# picture of a still screen go through Screenshot.
RUN apt-get update \
    && apt-get install -y --no-install-recommends xdg-desktop-portal-gtk \
    && rm -rf /var/lib/apt/lists/*

# Non-root user (sway, PipeWire and the portal want a normal session).
# Named volumes mounted on /target and the cargo registry take this ownership.
RUN useradd -m -u 1000 -s /bin/bash tester \
    && mkdir -p /run/user/1000 /target /work /out /usr/local/cargo/registry \
    && chown tester:tester /run/user/1000 /target /work /out \
    && chmod 700 /run/user/1000 \
    && chmod -R a+rwX /usr/local/cargo
ENV XDG_RUNTIME_DIR=/run/user/1000 \
    CARGO_TARGET_DIR=/target \
    CARGO_TERM_COLOR=always
USER tester
WORKDIR /work
# The scripts come from the mounted repo (/src), so editing them needs no rebuild.
ENTRYPOINT ["bash", "/src/docker/linux-test/run.sh"]
CMD ["x11"]
