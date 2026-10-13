#!/usr/bin/env bash
# rustshot Linux live-test harness, run inside the rustshot-linux-test image
# (docker/linux-test/run.ps1, the linux-wayland CI job). See README.md.
#
#   run.sh x11      Xvfb + PipeWire: cargo test --release, the ignored live
#                   tests, then a real daemon (Settings, overlay, capture,
#                   a recording) with screenshots.
#   run.sh wayland  headless sway + xdg-desktop-portal-wlr + PipeWire: a
#                   ScreenCast session driven through the portal, the
#                   Wayland/portal live tests, grim screenshots.
#
# Environment: RS_SRC (repo, default /src), RS_REF (revision, default HEAD;
# "worktree" copies the working tree), RS_WORK (the copy, default /work;
# RS_COPY=0 works in RS_SRC instead), RS_OUT (outputs, default /out),
# CARGO_TARGET_DIR (default /target), RS_ONLY, RS_TEST_TIMEOUT,
# RS_GST_CHECK, RS_WLR_RENDERER (README.md).
set -uo pipefail

MODE="${1:-x11}"
SRC="${RS_SRC:-/src}"
WORK="${RS_WORK:-/work}"
OUTROOT="${RS_OUT:-/out}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/target}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$OUTROOT/$MODE"
LOGS="$OUT/logs"
SHOTS="$OUT/shots"
REC="$OUT/rec"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
rm -rf "$OUT"
mkdir -p "$LOGS" "$SHOTS" "$REC"
SUMMARY="$OUT/summary.txt"
: > "$SUMMARY"
FAILED=0
PIDS=()

log() { echo "== $*" | tee -a "$LOGS/harness.log"; }
# result NAME pass|fail|info DETAIL; a "fail" makes the run exit 1.
result() {
    printf '%-6s %-40s %s\n' "$2" "$1" "${3:-}" | tee -a "$SUMMARY"
    [ "$2" = fail ] && FAILED=1
    return 0
}
bg() { # bg LOGNAME CMD...: start a background service, remember its pid
    local name="$1"; shift
    "$@" >"$LOGS/$name.log" 2>&1 &
    PIDS+=($!)
}
wait_for() { # wait_for SECONDS CMD...: poll until CMD succeeds
    local t="$1"; shift
    local end=$((SECONDS + t))
    until "$@" >/dev/null 2>&1; do
        [ "$SECONDS" -ge "$end" ] && return 1
        sleep 0.25
    done
}
cleanup() {
    for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
    sleep 0.5
    for p in "${PIDS[@]}"; do kill -9 "$p" 2>/dev/null; done
}
trap cleanup EXIT

# ---------------------------------------------------------------- source --
# RS_REF: the git revision to test (default HEAD: what is committed, so a
# half-edited working tree does not break the run); RS_REF=worktree copies
# the working tree as it is.
GIT=(git -c safe.directory='*' -C "$SRC")
REF="${RS_REF:-HEAD}"
if [ "${RS_COPY:-1}" = 1 ] && [ "$SRC" != "$WORK" ]; then
    mkdir -p "$WORK"
    find "$WORK" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
    if [ "$REF" = worktree ]; then
        log "copying the working tree of $SRC to $WORK"
        tar -C "$SRC" --exclude=./target --exclude='./target-*' --exclude=./_flameshot \
            --exclude=./dist --exclude=./.git --exclude=./.claude --exclude=./.superpowers \
            --exclude=./docker-out -cf - . | tar -C "$WORK" -xf -
    else
        log "git archive $REF of $SRC to $WORK"
        "${GIT[@]}" archive --format=tar "$REF" | tar -C "$WORK" -xf - \
            || { echo "git archive $REF failed" >&2; exit 1; }
    fi
    cd "$WORK"
else
    cd "$SRC"
fi
{ echo "ref: $REF"; "${GIT[@]}" log -1 --oneline "$([ "$REF" = worktree ] && echo HEAD || echo "$REF")"
  [ "$REF" = worktree ] && "${GIT[@]}" status --short; } > "$OUT/commit.txt" 2>&1
log "rustc $(rustc -V 2>/dev/null); target $CARGO_TARGET_DIR"

# -------------------------------------------------------- session + sound --
start_session() {
    log "dbus session"
    eval "$(dbus-launch --sh-syntax)"
    PIDS+=("$DBUS_SESSION_BUS_PID")
    export DBUS_SESSION_BUS_ADDRESS
    log "pipewire, wireplumber, pipewire-pulse"
    bg pipewire pipewire
    wait_for 10 test -S "$XDG_RUNTIME_DIR/pipewire-0" || { result pipewire fail "no socket"; return; }
    bg wireplumber wireplumber
    bg pipewire-pulse pipewire-pulse
    if wait_for 15 pactl info; then
        # The CI job's devices: a null sink (system sound) and a null source (microphone).
        pactl load-module module-null-sink sink_name=rec_sink >/dev/null
        pactl set-default-sink rec_sink
        pactl load-module module-null-sink sink_name=rec_mic object.linger=1 \
            media.class=Audio/Source/Virtual >/dev/null
        pactl set-default-source rec_mic
        wait_for 10 sh -c 'pactl list short sources | grep -q rec_sink.monitor'
        pactl info > "$LOGS/pactl-info.txt" 2>&1
        pactl list short sinks > "$LOGS/pactl-sinks.txt" 2>&1
        pactl list short sources > "$LOGS/pactl-sources.txt" 2>&1
        result pipewire-pulse pass "$(grep 'Server Name' "$LOGS/pactl-info.txt" | cut -d: -f2-)"
    else
        result pipewire-pulse fail "pactl info never answered"
    fi
}

# ---------------------------------------------------------------- cargo ---
build() {
    log "cargo build --release (bins and tests)"
    if cargo build --release --bins --tests >"$LOGS/build.log" 2>&1; then
        result build pass
    else
        result build fail "see logs/build.log"; tail -40 "$LOGS/build.log"; exit 1
    fi
    BIN="$CARGO_TARGET_DIR/release/rustshot"
    cargo test --release --bin rustshot -- --list --ignored 2>/dev/null \
        | sed -n 's/: test$//p' | sort > "$LOGS/ignored-tests.txt"
}

# cargo_tests NAME FILTER...: the ignored tests matching any FILTER, one thread.
cargo_tests() {
    local name="$1"; shift
    log "ignored tests: $name ($*)"
    timeout "${RS_TEST_TIMEOUT:-600}" cargo test --release --bin rustshot -- --ignored \
        --test-threads=1 --nocapture "$@" >"$LOGS/$name.log" 2>&1
    local rc=$?
    local line; line="$(grep -a 'test result:' "$LOGS/$name.log" | tail -1)"
    if [ $rc -eq 0 ] && grep -aq 'test result: ok' "$LOGS/$name.log"; then
        result "$name" pass "$line"
    else
        result "$name" fail "rc=$rc $line"
        grep -a -E '^test .* FAILED$|panicked at' "$LOGS/$name.log" | head -20 | sed 's/^/       /' | tee -a "$SUMMARY"
    fi
    grep -a -o 'SKIP:.*' "$LOGS/$name.log" | sort -u | sed 's/^/       /' | tee -a "$SUMMARY"
    grep -a -o -E 'FFPROBE .*|OH264 .*|PULSE .*|WAYLAND .*|CAM .*' "$LOGS/$name.log" | sort -u | sed 's/^/       /' >> "$SUMMARY"
}

# no_wayland_skips NAME: in Wayland mode the portal tests must really run;
# one that printed "SKIP: [wayland]" (no display, portal or libpipewire)
# fails the run.
no_wayland_skips() {
    local n; n="$(grep -a -c -F 'SKIP: [wayland]' "$LOGS/$1.log")"
    if [ "${n:-0}" -gt 0 ]; then
        result "$1-ran" fail "$n live_rec_wayland_* test(s) skipped: $(grep -a -o 'SKIP: \[wayland\].*' "$LOGS/$1.log" | sort -u | tr '\n' ' ')"
    else
        result "$1-ran" pass "no portal test skipped"
    fi
}

unit_tests() {
    log "cargo test --release"
    if cargo test --release >"$LOGS/unit.log" 2>&1; then
        result unit-tests pass "$(grep -a 'test result:' "$LOGS/unit.log" | head -1)"
    else
        result unit-tests fail "see logs/unit.log"
        grep -a -E '^test .* FAILED$|panicked at|^error' "$LOGS/unit.log" | head -20 | sed 's/^/       /' | tee -a "$SUMMARY"
    fi
}

# The ignored tests CI does not run on Linux (minus benches, previews,
# network, folder pickers and cameras): informational.
extra_tests() {
    local ran='^(live_monitor_capture|live_desktop_capture|live_window_lifecycle|interactive_window_renders_rect|.*window_smoke|live_rec_linux_.*|live_oh264_.*|live_sni_.*)'
    local skip='(wayland|bench|perf_|render_|preview|write_glyph|manual_|live_download|live_imgur|live_portable_update|holder_child|probe_recording_file|leak|live_cam_)'
    local extra
    extra="$(sed 's/.*:://' "$LOGS/ignored-tests.txt" | grep -v -E "$ran" | grep -v -E "$skip" | sort -u | tr '\n' ' ')"
    sed 's/^/       ignored test compiled on Linux: /' "$LOGS/ignored-tests.txt" >> "$SUMMARY"
    if [ -n "${extra// }" ]; then
        local t
        for t in $extra; do
            RS_TEST_TIMEOUT=120 cargo_tests "extra-$t" --exact "$(grep -m1 -E "(^|::)$t\$" "$LOGS/ignored-tests.txt")"
        done
        # Informational: do not gate on tests CI never ran.
        sed -i 's/^fail   extra-/info   extra-FAIL-/' "$SUMMARY"
    fi
}

shot_x() { import -window root "$SHOTS/$1.png" 2>>"$LOGS/import.log" && echo "       shot $1.png" >> "$SUMMARY"; }

# ------------------------------------------------------------------ x11 ---
# The overlay is up: a mapped (viewable) full-screen window; xwininfo -tree
# lists unmapped windows too.
overlay_up() {
    local id
    for id in $(xwininfo -root -tree 2>/dev/null | awk 'index($0, " 1920x1080+0+0 ") {print $1}'); do
        xwininfo -id "$id" 2>/dev/null | grep -q 'IsViewable' && return 0
    done
    return 1
}
# A desktop with something on it (and a second picture to switch to).
set_bg() { display -window root "/tmp/bg$1.png" 2>>"$LOGS/import.log"; }
make_bg() {
    convert -size 1920x1080 gradient:'#1d3557-#457b9d' -fill white -pointsize 64 \
        -annotate +120+180 "rustshot X11 harness" -fill '#e63946' -draw 'rectangle 300,300 900,700' \
        -fill '#f1faee' -draw 'circle 1400,600 1400,800' /tmp/bg1.png 2>>"$LOGS/import.log"
    convert /tmp/bg1.png -fill '#2a9d8f' -draw 'rectangle 500,400 1100,800' \
        -fill '#e9c46a' -draw 'circle 1300,500 1300,650' /tmp/bg2.png 2>>"$LOGS/import.log"
}
# The recording bar's Stop button: the bar is the mapped window between
# 60x30 and 600x160 px; Stop is its last button (bar.rs: MARGIN 22, PAD 6,
# 32 px buttons at scale 1).
# The post-capture notice: the mapped "Rustshot notice" window (its id).
notice_id() {
    local id
    for id in $(xwininfo -root -tree 2>/dev/null | awk '/"Rustshot notice"/ {print $1}'); do
        xwininfo -id "$id" 2>/dev/null | grep -q 'IsViewable' && { echo "$id"; return 0; }
    done
    return 1
}
notice_gone() { ! xwininfo -id "$1" 2>/dev/null | grep -q IsViewable; }
bar_stop_xy() {
    xwininfo -root -tree 2>/dev/null \
        | sed -n 's/.* \([0-9][0-9]*\)x\([0-9][0-9]*\)+[-0-9]*+[-0-9]*  *+\([-0-9]*\)+\([-0-9]*\)$/\1 \2 \3 \4/p' \
        | awk '$1 > 60 && $1 < 600 && $2 > 30 && $2 < 160 { print $3 + $1 - 22 - 6 - 16, $4 + int($2 / 2); exit }'
}

# ------------------------------------------------------------ the tray ---
# A StatusNotifierWatcher (sni_watcher.py: what a desktop's tray runs) for
# the daemon's tray icon to register with.
SNI_LOG=
WATCHER_PID=
start_watcher() { # start_watcher LOGNAME
    SNI_LOG="$OUT/sni-registered.txt"
    touch "$SNI_LOG"
    bg "$1" python3 "$HERE/sni_watcher.py" "$SNI_LOG"
    WATCHER_PID="${PIDS[-1]}"
    if wait_for 10 sh -c 'gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.NameHasOwner org.kde.StatusNotifierWatcher | grep -q true'; then
        result "$1" pass "org.kde.StatusNotifierWatcher up"
    else
        result "$1" fail "no watcher on the bus; see logs/$1.log"
    fi
}
sni_layout() { gdbus call --session -d "$1" -o /MenuBar -m com.canonical.dbusmenu.GetLayout -- 0 -1 '[]' 2>&1; }
sni_has() { sni_layout "$1" | grep -qF "'$2'"; } # sni_has SERVICE LABEL
sni_tip() { gdbus call --session -d "$1" -o /StatusNotifierItem -m org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierItem ToolTip 2>&1; }
sni_labels() { grep -o "'label': <'[^']*'>" | sed "s/'label': <'\(.*\)'>/\1/" | paste -sd, -; }

# The daemon's icon: registered with the watcher, its menu (GetLayout) the
# shared model, the tooltip; then the watcher restarts (the panel came
# back) and the icon registers again.
sni_steps_idle() {
    local svc="$1" lay miss="" want n
    if wait_for 10 grep -q "^$svc " "$SNI_LOG"; then
        result sni-register pass "$svc"
    else
        result sni-register fail "$svc did not register; see logs/sni-watcher.log, logs/daemon.log"
        return
    fi
    lay="$(sni_layout "$svc")"
    echo "$lay" > "$LOGS/sni-layout-idle.txt"
    for want in "Capture" "Settings" "Check for updates" "Quit Rustshot"; do
        grep -qF "'$want" <<<"$lay" || miss="$miss $want"
    done
    if [ -z "$miss" ]; then result sni-menu pass "GetLayout: $(sni_labels <<<"$lay")"
    else result sni-menu fail "missing:$miss; see logs/sni-layout-idle.txt"; fi
    if sni_tip "$svc" | grep -q "'Rustshot'"; then result sni-tooltip pass "Rustshot"
    else result sni-tooltip fail "$(sni_tip "$svc" | cut -c1-160)"; fi
    n="$(grep -c "^$svc " "$SNI_LOG")"
    kill "$WATCHER_PID" 2>/dev/null; wait "$WATCHER_PID" 2>/dev/null
    start_watcher sni-watcher-2
    if wait_for 10 sh -c "[ \$(grep -c '^$svc ' '$SNI_LOG') -gt $n ]"; then
        result sni-reregister pass "registered again with the new watcher"
    else
        result sni-reregister fail "not registered with the restarted watcher"
    fi
}

# While recording: the recording's items, the tooltip's time, a menu click
# (Event) that pauses, and the while-recording chord that resumes.
sni_steps_recording() {
    local svc="$1" lay miss="" want
    lay="$(sni_layout "$svc")"
    echo "$lay" > "$LOGS/sni-layout-recording.txt"
    for want in "Stop recording" "Pause recording" "Hide recording controls" "Discard recording"; do
        grep -qF "'$want'" <<<"$lay" || miss="$miss $want"
    done
    if [ -z "$miss" ]; then result sni-menu-recording pass "$(sni_labels <<<"$lay")"
    else result sni-menu-recording fail "missing:$miss; see logs/sni-layout-recording.txt"; fi
    if sni_tip "$svc" | grep -q "recording"; then result sni-tooltip-recording pass "$(sni_tip "$svc" | grep -o "Rustshot[^']*")"
    else result sni-tooltip-recording fail "$(sni_tip "$svc" | cut -c1-160)"; fi
    gdbus call --session -d "$svc" -o /MenuBar -m com.canonical.dbusmenu.Event -- 2 clicked '<0>' 0 >>"$LOGS/sni-event.log" 2>&1
    if wait_for 3 sni_has "$svc" "Resume recording"; then result sni-event-pause pass "Event(2, clicked): paused"
    else result sni-event-pause fail "the menu does not say Resume after Event; see logs/sni-event.log"; fi
    xdotool key ctrl+alt+shift+i
    if wait_for 3 sni_has "$svc" "Pause recording"; then result rec-chord-pause pass "Ctrl+Alt+Shift+I (while recording): resumed"
    else result rec-chord-pause fail "the while-recording chord did not resume"; fi
}

x11_daemon() {
    local cfgdir; cfgdir="$(mktemp -d)"
    local save="$OUT/saved"
    mkdir -p "$cfgdir/rustshot" "$save"
    # Unusual hotkeys, so nothing else could be bound to them (no Ctrl+Alt+F<n>:
    # X turns those into XF86Switch_VT_<n>). Ctrl+R is the overlay's Record
    # (no default chord): the route that needs no global hotkey.
    cat > "$cfgdir/rustshot/config.toml" <<EOF
save_path = "$save"
save_subfolder = false
capture_hotkey = "Ctrl+Alt+Shift+Y"
record_hotkey = "Ctrl+Alt+Shift+U"
quit_hotkey = "Ctrl+Alt+Shift+J"
check_updates = false
rec_countdown = false
rec_format = "mp4"
rec_system_audio = true
rec_linux_encoder = "ffmpeg"

[shortcuts]
record = "Ctrl+R"
rec_pause = "Ctrl+Alt+Shift+I"
EOF
    cp "$cfgdir/rustshot/config.toml" "$OUT/daemon-config.toml"
    export XDG_CONFIG_HOME="$cfgdir" RUSTSHOT_INSTANCE="docker-$MODE-$$" RUSTSHOT_FFMPEG_LOG=1
    log "daemon (instance $RUSTSHOT_INSTANCE)"
    "$BIN" config --check >"$LOGS/config-check.log" 2>&1 \
        && result config-check pass || result config-check fail "see logs/config-check.log"
    make_bg; set_bg 1
    "$BIN" daemon >"$LOGS/daemon.log" 2>&1 &
    local dpid=$!
    PIDS+=("$dpid")
    sleep 2
    kill -0 "$dpid" 2>/dev/null && result daemon-start pass "pid $dpid" || { result daemon-start fail "exited; see logs/daemon.log"; return; }
    shot_x 01-desktop
    local svc="org.kde.StatusNotifierItem-$dpid-1"
    sni_steps_idle "$svc"

    # Settings window, opened in the running daemon.
    "$BIN" settings >"$LOGS/settings-cmd.log" 2>&1 &
    local spid=$!
    if wait_for 10 sh -c 'xdotool search --name "[Ss]ettings" | grep -q .'; then
        sleep 1.5
        shot_x 02-settings
        local w; w="$(xdotool search --name "[Ss]ettings" | head -1)"
        result settings-window pass "\"$(xdotool getwindowname "$w")\" $(xwininfo -id "$w" | awk '/-geometry/ {print $2}')"
        xdotool windowclose "$w" 2>/dev/null
    else
        shot_x 02-settings-missing
        xwininfo -root -tree > "$LOGS/xwininfo-settings.txt" 2>&1
        result settings-window fail "no Settings window within 10 s; see logs/xwininfo-settings.txt"
    fi
    wait "$spid" 2>/dev/null
    sleep 1

    # The capture hotkey; when it does nothing, a second launch (the
    # daemon's "capture" message) opens the overlay instead.
    log "capture hotkey"
    xdotool key ctrl+alt+shift+y
    if wait_for 3 overlay_up; then
        result capture-hotkey pass "overlay up"
    else
        result capture-hotkey fail "Ctrl+Alt+Shift+Y (XTEST) opened no overlay; using a second launch"
        timeout 5 "$BIN" >>"$LOGS/second-launch.log" 2>&1
        wait_for 5 overlay_up && result capture-second-launch pass "overlay up" \
            || result capture-second-launch fail "no overlay"
    fi
    sleep 1
    shot_x 03-overlay
    # Ctrl+A (whole screen), not a drag: a drag is its own step at the end.
    xdotool key ctrl+a
    sleep 1
    shot_x 04-selection
    xdotool key ctrl+s
    # The notice: a saved screenshot gets the thumbnail card ("Saved ·
    # <file>" under the picture): override-redirect, typed NOTIFICATION,
    # bottom-right of the screen (Xvfb has no panels), up ~6 s.
    if wait_for 3 notice_id; then
        local nid; nid="$(notice_id)"
        shot_x 04c-notice
        local ninfo; ninfo="$(xwininfo -id "$nid")"
        echo "$ninfo" > "$LOGS/notice-xwininfo.txt"
        xprop -id "$nid" >> "$LOGS/notice-xwininfo.txt" 2>&1
        local nx ny nw nh
        nx="$(echo "$ninfo" | awk '/Absolute upper-left X/ {print $4}')"
        ny="$(echo "$ninfo" | awk '/Absolute upper-left Y/ {print $4}')"
        nw="$(echo "$ninfo" | awk '/Width:/ {print $2}')"
        nh="$(echo "$ninfo" | awk '/Height:/ {print $2}')"
        local rx=$((nx + nw)) by=$((ny + nh))
        if echo "$ninfo" | grep -q 'Override Redirect State: yes'             && xprop -id "$nid" _NET_WM_WINDOW_TYPE | grep -q NOTIFICATION             && [ "$rx" -eq 1920 ] && [ "$by" -eq 1080 ]; then
            result notice-window pass "${nw}x${nh}+${nx}+${ny}"
        else
            result notice-window fail "${nw}x${nh}+${nx}+${ny}; see logs/notice-xwininfo.txt"
        fi
        if wait_for 9 notice_gone "$nid"; then
            result notice-fades pass "unmapped after its time"
        else
            result notice-fades fail "still up after 9 s"
        fi
    else
        shot_x 04c-notice-missing
        xwininfo -root -tree > "$LOGS/xwininfo-notice.txt" 2>&1
        result notice-window fail "no notice window within 3 s of Ctrl+S; see logs/xwininfo-notice.txt"
    fi
    if wait_for 10 sh -c "ls '$save'/*.png"; then
        local png; png="$(ls "$save"/*.png | head -1)"
        result capture-save pass "$(basename "$png") $(identify "$png" | awk '{print $3}')"
        cp "$png" "$SHOTS/04b-saved-capture.png"
    else
        result capture-save fail "no PNG in $save after Ctrl+S"
        xdotool key Escape
    fi
    sleep 1

    # One-shot overlay (`rustshot gui`), Escape closes it.
    log "rustshot gui"
    RUSTSHOT_INSTANCE="docker-gui-$$" "$BIN" gui >"$LOGS/gui.log" 2>&1 &
    local gpid=$!
    wait_for 5 overlay_up; sleep 1
    shot_x 05-gui-overlay
    xdotool key Escape
    if wait_for 5 sh -c "! kill -0 $gpid"; then result gui-oneshot pass "Escape closed it"; else
        result gui-oneshot fail "still running after Escape"; kill "$gpid"; fi
    sleep 1

    # A recording: the record hotkey (else the overlay and Ctrl+R), a
    # selection, Enter, ~4 s with the picture changing and a tone playing,
    # then the hotkey again (else the bar's Stop).
    log "recording"
    if ! kill -0 "$dpid" 2>/dev/null; then
        wait "$dpid"; result daemon-alive fail "the daemon died before the recording (exit $?); restarted"
        "$BIN" daemon >>"$LOGS/daemon.log" 2>&1 &
        dpid=$!; PIDS+=("$dpid"); sleep 2
    fi
    ( for _ in 1 2 3 4 5 6 7 8; do
        ffmpeg -loglevel error -f lavfi -i "sine=frequency=880:duration=1" -f wav - | paplay; done ) \
        >>"$LOGS/paplay.log" 2>&1 &
    local route=hotkey
    xdotool key ctrl+alt+shift+u
    if wait_for 3 overlay_up; then
        result record-hotkey pass "record overlay up"
        sleep 0.5; shot_x 06-record-overlay
        xdotool key ctrl+a
    else
        result record-hotkey fail "Ctrl+Alt+Shift+U (XTEST) opened no overlay; using the overlay's Ctrl+R"
        route=overlay
        timeout 5 "$BIN" >>"$LOGS/second-launch.log" 2>&1
        wait_for 5 overlay_up; sleep 0.5
        xdotool key ctrl+a
        sleep 0.5
        shot_x 06-overlay-for-record
        xdotool key ctrl+r
    fi
    sleep 1.5
    shot_x 07-record-popover
    xdotool key Return
    sleep 1.5
    local i
    for i in 1 2 1 2; do set_bg "$i"; sleep 0.4; done
    shot_x 08-recording-border-and-bar
    xwininfo -root -tree > "$LOGS/xwininfo-recording.txt" 2>&1
    sni_steps_recording "$svc"
    sleep 1
    if [ "$route" = hotkey ]; then
        xdotool key ctrl+alt+shift+u
    else
        local xy; xy="$(bar_stop_xy)"
        if [ -n "$xy" ]; then
            # shellcheck disable=SC2086
            xdotool mousemove $xy; sleep 0.4; shot_x 08b-bar-stop-hover; xdotool click 1
            echo "       clicked the bar's Stop at $xy" >> "$SUMMARY"
        else
            result record-bar fail "no bar window found; see logs/xwininfo-recording.txt"
        fi
    fi
    if wait_for 30 sh -c "ls '$save'/*.mp4"; then
        sleep 2
        local mp4; mp4="$(ls -t "$save"/*.mp4 | head -1)"
        cp "$mp4" "$REC/recording.mp4"
        ffprobe -v error -show_format -show_streams -of json "$mp4" > "$REC/recording.ffprobe.json" 2>&1
        local dur; dur="$(jq -r '.format.duration' "$REC/recording.ffprobe.json")"
        local streams; streams="$(jq -r '[.streams[] | .codec_type + ":" + .codec_name + (if .width then "@" + (.width|tostring) + "x" + (.height|tostring) else "" end)] | join(",")' "$REC/recording.ffprobe.json")"
        ffmpeg -v error -i "$mp4" -f null - >"$LOGS/recording-decode.log" 2>&1
        ffmpeg -v error -y -ss 1.5 -i "$mp4" -frames:v 1 "$SHOTS/09-recording-frame.png" 2>>"$LOGS/recording-decode.log"
        ffmpeg -v error -y -i "$mp4" -vf "fps=8,scale=480:-1" "$REC/recording.gif" 2>>"$LOGS/recording-decode.log"
        local vol; vol="$(ffmpeg -hide_banner -i "$mp4" -af volumedetect -vn -f null - 2>&1 | grep -o 'mean_volume: .*')"
        if [ -s "$LOGS/recording-decode.log" ]; then
            result recording fail "via $route; decode errors: logs/recording-decode.log ($dur s, $streams)"
        else
            result recording pass "via $route: $dur s, $streams, $vol"
        fi
    else
        shot_x 09-record-no-file
        result recording fail "via $route: no MP4 within 30 s; see logs/daemon.log"
        xdotool key Escape
    fi
    sleep 1
    shot_x 10-after-recording

    # The quit hotkey ends the daemon.
    xdotool key ctrl+alt+shift+j
    if wait_for 5 sh -c "! kill -0 $dpid"; then result quit-hotkey pass; else
        result quit-hotkey fail "Ctrl+Alt+Shift+J (XTEST): daemon still running; killed"
        kill "$dpid"; fi

    # A drag in the overlay (a one-shot run, so a crash takes nothing else
    # down), then Ctrl+S: the saved PNG is the dragged 800x400.
    log "overlay drag"
    local before; before="$(ls "$save"/*.png 2>/dev/null | wc -l)"
    RUSTSHOT_INSTANCE="docker-drag-$$" "$BIN" gui >"$LOGS/gui-drag.log" 2>&1 &
    gpid=$!
    wait_for 5 overlay_up; sleep 0.5
    xdotool mousemove 400 300 mousedown 1 mousemove 600 400 mousemove 800 500 mousemove 1200 700
    sleep 0.3
    xdotool mouseup 1
    sleep 1
    if kill -0 "$gpid" 2>/dev/null; then
        shot_x 11-drag-selection
        result overlay-drag pass "alive after the drag"
        xdotool key ctrl+s
        if wait_for 10 sh -c "[ \$(ls '$save'/*.png 2>/dev/null | wc -l) -gt $before ]"; then
            local dpng; dpng="$(ls -t "$save"/*.png | head -1)"
            local dsz; dsz="$(identify -format '%wx%h' "$dpng" 2>/dev/null)"
            cp "$dpng" "$SHOTS/11b-saved-drag.png"
            if [ "$dsz" = 800x400 ]; then result drag-save pass "$(basename "$dpng") $dsz"
            else result drag-save fail "$(basename "$dpng") is $dsz, not the dragged 800x400"; fi
        else
            result drag-save fail "no new PNG in $save after the drag and Ctrl+S"
        fi
        wait_for 5 sh -c "! kill -0 $gpid" || { xdotool key Escape; wait_for 5 sh -c "! kill -0 $gpid" || kill "$gpid"; }
    else
        wait "$gpid"; local drc=$?
        result overlay-drag fail "rustshot gui died during a mouse drag (exit $drc$([ $drc = 139 ] && echo ', SIGSEGV'))"
    fi
    snap_steps "$save"
    unset XDG_CONFIG_HOME RUSTSHOT_INSTANCE
}

# Window snapping: an xmessage test window (no window manager here, so its
# rect is its geometry plus border); a one-shot overlay; hover, click,
# Ctrl+S: the saved PNG is exactly the window. Then over its button, Tab
# (into the part under the pointer), click, Ctrl+S: a part's size.
snap_steps() {
    local save="$1" before n gpid
    log "window snapping"
    xmessage -title rs-snap-test -geometry 640x360+300+200 -buttons "okay:0" "Rustshot snapping test window" \
        >"$LOGS/xmessage.log" 2>&1 &
    local xpid=$!
    PIDS+=("$xpid")
    if ! wait_for 5 xwininfo -name rs-snap-test; then result snap-window fail "no xmessage window"; return; fi
    sleep 0.5
    local info; info="$(xwininfo -name rs-snap-test)"
    echo "$info" > "$LOGS/snap-xwininfo.txt"
    xwininfo -name rs-snap-test -tree >> "$LOGS/snap-xwininfo.txt" 2>&1
    local wx wy ww wh bw
    wx=$(echo "$info" | awk '/Absolute upper-left X/ {print $4}')
    wy=$(echo "$info" | awk '/Absolute upper-left Y/ {print $4}')
    ww=$(echo "$info" | awk '/Width:/ {print $2}')
    wh=$(echo "$info" | awk '/Height:/ {print $2}')
    bw=$(echo "$info" | awk '/Border width:/ {print $3}')
    local want="$((ww + 2 * bw))x$((wh + 2 * bw))"
    result snap-window info "xmessage at $wx,$wy ${ww}x${wh} border $bw: expecting $want"

    before="$(ls "$save"/*.png 2>/dev/null | wc -l)"
    RUSTSHOT_INSTANCE="docker-snap-$$" "$BIN" gui >"$LOGS/gui-snap.log" 2>&1 &
    gpid=$!
    wait_for 5 overlay_up; sleep 0.5
    xdotool mousemove $((wx + ww / 2)) $((wy + wh / 3)); sleep 0.5
    shot_x 12-snap-hover
    xdotool click 1; sleep 0.7
    shot_x 12b-snap-clicked
    xdotool key ctrl+s
    if wait_for 10 sh -c "[ \$(ls '$save'/*.png 2>/dev/null | wc -l) -gt $before ]"; then
        local png; png="$(ls -t "$save"/*.png | head -1)"
        local sz; sz="$(identify -format '%wx%h' "$png" 2>/dev/null)"
        cp "$png" "$SHOTS/12c-saved-snap.png"
        if [ "$sz" = "$want" ]; then result snap-click-save pass "$(basename "$png") $sz = the window"
        else result snap-click-save fail "$(basename "$png") is $sz, not the window's $want"; fi
    else
        result snap-click-save fail "no new PNG after hover, click and Ctrl+S"
    fi
    wait_for 5 sh -c "! kill -0 $gpid" || { xdotool key Escape; wait_for 5 sh -c "! kill -0 $gpid" || kill "$gpid"; }

    # The parts: every subwindow's size (outer, border included).
    local parts; parts="$(xwininfo -name rs-snap-test -tree | awk '/^ +0x/ { for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+x[0-9]+\+/) { split($i, g, /[x+]/); print g[1] "x" g[2] } }' | sort -u | tr '\n' ' ')"
    # The button: the smallest subwindow (xmessage's widgets have no
    # names); its centre from the absolute position xwininfo prints.
    before="$(ls "$save"/*.png 2>/dev/null | wc -l)"
    RUSTSHOT_INSTANCE="docker-snap2-$$" "$BIN" gui >"$LOGS/gui-snap2.log" 2>&1 &
    gpid=$!
    wait_for 5 overlay_up; sleep 0.5
    local bx by; read -r bx by < <(xwininfo -name rs-snap-test -tree | awk '/^ +0x/ { s = ""; a = ""; for (i = 1; i <= NF; i++) { if ($i ~ /^[0-9]+x[0-9]+[+-]/) s = $i; else if ($i ~ /^[+-][0-9]+[+-][0-9]+$/) a = $i } if (s != "" && a != "") { split(s, g, /[x+]/); split(substr(a, 2), q, /[+]/); print g[1] * g[2], q[1] + int(g[1] / 2), q[2] + int(g[2] / 2) } }' | sort -n | head -1 | cut -d" " -f2-)
    if [ -z "$bx" ]; then bx=$((wx + ww / 2)); by=$((wy + wh - 20)); fi
    xdotool mousemove "$bx" "$by"; sleep 0.4
    xdotool key Tab; sleep 0.4
    shot_x 13-snap-tab-part
    xdotool click 1; sleep 0.5
    xdotool key ctrl+s
    if wait_for 10 sh -c "[ \$(ls '$save'/*.png 2>/dev/null | wc -l) -gt $before ]"; then
        local png; png="$(ls -t "$save"/*.png | head -1)"
        local sz; sz="$(identify -format '%wx%h' "$png" 2>/dev/null)"
        cp "$png" "$SHOTS/13b-saved-part.png"
        # A child geometry is inside its parent (relative position), so its
        # size is what to compare; xwininfo prints sizes without the border.
        local ok=0 p w h
        for p in $parts; do
            w=${p%x*}; h=${p#*x}
            for b in 0 1 2; do [ "$sz" = "$((w + 2 * b))x$((h + 2 * b))" ] && ok=1; done
        done
        # Not the window, nor its form (all of it but the border).
        if [ "$ok" = 1 ] && [ "${sz%x*}" -lt "$((ww - 4))" ]; then result snap-tab-part pass "$(basename "$png") $sz, a part (parts: $parts)"
        else result snap-tab-part fail "$(basename "$png") is $sz; parts are: $parts"; fi
    else
        result snap-tab-part fail "no new PNG after Tab, click and Ctrl+S"
    fi
    wait_for 5 sh -c "! kill -0 $gpid" || { xdotool key Escape; wait_for 5 sh -c "! kill -0 $gpid" || kill "$gpid"; }
    kill "$xpid" 2>/dev/null
}

mode_x11() {
    start_session
    log "Xvfb :99 1920x1080x24"
    bg xvfb Xvfb :99 -screen 0 1920x1080x24 -nolisten tcp
    export DISPLAY=:99
    wait_for 15 xdpyinfo || { result xvfb fail; exit 1; }
    xdpyinfo > "$LOGS/xdpyinfo.txt"
    result xvfb pass "$(grep dimensions "$LOGS/xdpyinfo.txt" | xargs)"
    start_watcher sni-watcher
    build
    # RS_ONLY=daemon skips the cargo tests (iterating on the daemon part).
    if [ "${RS_ONLY:-}" != daemon ]; then
        unit_tests
        export RUSTSHOT_TEST_AUTOCLOSE_MS=1000 RUSTSHOT_FFMPEG_LOG=1
        cargo_tests live-x11-capture-windows live_monitor_capture live_desktop_capture \
            live_window_lifecycle interactive_window_renders_rect window_smoke
        cargo_tests live-rec-linux live_rec_linux_
        cargo_tests live-sni live_sni_
        XDG_DATA_HOME="$(mktemp -d)" cargo_tests live-oh264 live_oh264_
        extra_tests
        # The camera tests (SKIP: without a /dev/video* or the camera portal) and
        # the bubble/bar preview PNGs (informational).
        cargo_tests live-cam-linux live_cam_linux_
        mkdir -p "$OUT/previews"
        RUSTSHOT_PREVIEW_DIR="$OUT/previews" cargo_tests previews render_bubble_previews render_recording_previews
        sed -i "s/^fail   previews/info   previews-FAIL/" "$SUMMARY"
        unset RUSTSHOT_TEST_AUTOCLOSE_MS
    fi
    x11_daemon
}

# -------------------------------------------------------------- wayland ---
mode_wayland() {
    start_session
    export XDG_CURRENT_DESKTOP=sway XDG_SESSION_TYPE=wayland
    local cfg="$HOME/.config"
    mkdir -p "$cfg/sway" "$cfg/xdg-desktop-portal" "$cfg/xdg-desktop-portal-wlr"
    cat > "$cfg/sway/config" <<'EOF'
output HEADLESS-1 resolution 1920x1080 position 0 0 bg #2e5c8a solid_color
default_border pixel 2
xwayland enable
EOF
    cat > "$cfg/xdg-desktop-portal/portals.conf" <<'EOF'
[preferred]
default=wlr;gtk
org.freedesktop.impl.portal.ScreenCast=wlr
org.freedesktop.impl.portal.Screenshot=wlr
org.freedesktop.impl.portal.Access=gtk
EOF
    cp "$cfg/xdg-desktop-portal/portals.conf" "$cfg/xdg-desktop-portal/sway-portals.conf"
    cat > "$cfg/xdg-desktop-portal-wlr/config" <<'EOF'
[screencast]
output_name=HEADLESS-1
chooser_type=none
max_fps=30
EOF
    log "sway (headless, pixman)"
    WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER="${RS_WLR_RENDERER:-pixman}" \
        bg sway sway -d -c "$cfg/sway/config"
    if ! wait_for 15 sh -c 'ls "$XDG_RUNTIME_DIR"/wayland-[0-9] 2>/dev/null | grep -q .'; then
        result sway fail "no Wayland socket; see logs/sway.log"; tail -30 "$LOGS/sway.log"; exit 1
    fi
    WAYLAND_DISPLAY="$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-[0-9] | head -1)")"
    export WAYLAND_DISPLAY
    export SWAYSOCK; SWAYSOCK="$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock 2>/dev/null | head -1)"
    wait_for 10 swaymsg -t get_outputs
    swaymsg -t get_outputs > "$LOGS/sway-outputs.json" 2>&1
    result sway pass "$WAYLAND_DISPLAY $(jq -r '.[] | .name + " " + (.current_mode.width|tostring) + "x" + (.current_mode.height|tostring)' "$LOGS/sway-outputs.json" | xargs)"
    # The activation environment for the portal and its backend.
    dbus-update-activation-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE \
        XDG_RUNTIME_DIR SWAYSOCK 2>>"$LOGS/harness.log"

    log "xdg-desktop-portal-wlr, xdg-desktop-portal"
    local xdpw xdp
    xdpw="$(ls /usr/libexec/xdg-desktop-portal-wlr /usr/lib/xdg-desktop-portal-wlr 2>/dev/null | head -1)"
    xdp="$(ls /usr/libexec/xdg-desktop-portal /usr/lib/xdg-desktop-portal 2>/dev/null | head -1)"
    bg xdpw "$xdpw" -l DEBUG
    wait_for 10 sh -c 'busctl --user list 2>/dev/null | grep -q org.freedesktop.impl.portal.desktop.wlr || gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.NameHasOwner org.freedesktop.impl.portal.desktop.wlr | grep -q true'
    bg xdp "$xdp" -v -r
    if wait_for 15 gdbus introspect --session -d org.freedesktop.portal.Desktop -o /org/freedesktop/portal/desktop; then
        gdbus introspect --session -d org.freedesktop.portal.Desktop -o /org/freedesktop/portal/desktop \
            > "$LOGS/portal-introspect.txt" 2>&1
        if grep -q 'org.freedesktop.portal.ScreenCast' "$LOGS/portal-introspect.txt"; then
            result portal-up pass "ScreenCast interface exported"
        else
            result portal-up fail "no ScreenCast interface; see logs/xdp.log"
        fi
        if grep -q 'org.freedesktop.portal.Screenshot' "$LOGS/portal-introspect.txt"; then
            result portal-screenshot pass "Screenshot interface exported"
        else
            result portal-screenshot fail "no Screenshot interface (needs an Access implementation); see logs/xdp.log"
        fi
    else
        result portal-up fail "org.freedesktop.portal.Desktop not on the bus"
    fi

    # A ScreenCast session through the portal, with no chooser.
    log "portal ScreenCast session"
    # Damage: the background changes twice a second (a static headless output
    # gives a screen cast nothing new to send). RS_WL_ANIM=0: none at all,
    # every test sees a still screen. Either way the screen cast tests run
    # again on a still screen below (the source repeats its last picture).
    ANIM_PID=
    if [ "${RS_WL_ANIM:-1}" != 0 ]; then
        ( c=0; while sleep 0.5; do c=$((1 - c)); swaymsg output HEADLESS-1 bg "$([ $c = 1 ] && echo "#2e5c8a" || echo "#8a2e5c")" solid_color >/dev/null 2>&1; done ) &
        ANIM_PID=$!
        PIDS+=("$ANIM_PID")
    else
        result screen-animation info "off (RS_WL_ANIM=0): a still screen"
    fi
    local ready="$OUT/portal-nodes.txt"
    # RS_FRAME_PNG: the script also pulls one picture through the portal's
    # remote (GStreamer pipewiresrc on its fd), an independent consumer.
    RS_FRAME_PNG="$([ "${RS_GST_CHECK:-0}" = 1 ] && echo "$SHOTS/03-portal-frame-gstreamer.png")" python3 "$HERE/portal_screencast.py" 4 "$ready" >"$LOGS/portal-screencast.log" 2>&1 &
    local ppid=$!
    if wait_for 30 test -s "$ready"; then
        local node; node="$(awk '{print $1}' "$ready")"
        sleep 1
        pw-cli ls Node > "$LOGS/pw-nodes.txt" 2>&1
        pw-dump > "$LOGS/pw-dump.json" 2>/dev/null
        if grep -q "id $node," "$LOGS/pw-nodes.txt"; then
            result portal-screencast pass "node $node: $(grep -A12 "id $node," "$LOGS/pw-nodes.txt" | grep -m1 'node.name' | xargs)"
        else
            result portal-screencast fail "node $node not in pw-cli ls Node"
        fi
        grim "$SHOTS/02-during-screencast.png" 2>>"$LOGS/grim.log" && echo "       shot 02-during-screencast.png" >> "$SUMMARY"
    else
        result portal-screencast fail "no stream from Start; see logs/portal-screencast.log"
        tail -5 "$LOGS/portal-screencast.log" | sed 's/^/       /' | tee -a "$SUMMARY"
    fi
    wait "$ppid"; local prc=$?
    [ "$prc" -eq 0 ] || result portal-session-close fail "portal_screencast.py exit $prc"
    if [ "${RS_GST_CHECK:-0}" != 1 ]; then
        result portal-frames info "skipped (RS_GST_CHECK=1: GStreamer on the portal fd; xdg-desktop-portal-wlr 0.8.4 then dies: ext_image_copy_capture \"session already has a frame object\")"
    elif [ -s "$SHOTS/03-portal-frame-gstreamer.png" ]; then
        result portal-frames pass "pipewiresrc on the portal fd: shots/03-portal-frame-gstreamer.png"
    else
        result portal-frames fail "pipewiresrc on the portal fd got no picture: $(grep -a 'PORTAL gst' "$LOGS/portal-screencast.log" | tail -1 | cut -c1-200)"
    fi

    grim "$SHOTS/01-desktop.png" 2>>"$LOGS/grim.log" && result grim pass "shots/01-desktop.png" \
        || result grim fail "see logs/grim.log"

    # RS_ONLY=portal stops here (iterating on the session).
    [ "${RS_ONLY:-}" = portal ] && return
    build
    # The Wayland/portal live tests, when there are any.
    local wl
    wl="$(sed 's/.*:://' "$LOGS/ignored-tests.txt" | grep -E '^live_rec_wayland_|^live_cam_linux_|wayland|portal|pipewire|screencast' | tr '\n' ' ')"
    if [ -n "${wl// }" ]; then
        # shellcheck disable=SC2086
        RUSTSHOT_TEST_AUTOCLOSE_MS=1000 RUSTSHOT_FFMPEG_LOG=1 cargo_tests live-wayland $wl
        no_wayland_skips live-wayland
    else
        result live-wayland info "no ignored test named *wayland*/*portal*/*pipewire*/*screencast* yet"
    fi
    # A still screen: the compositor sends one picture and then nothing;
    # the frames must still come at the recording's rate.
    if [ -n "$ANIM_PID" ]; then
        kill "$ANIM_PID" 2>/dev/null; wait "$ANIM_PID" 2>/dev/null
        swaymsg output HEADLESS-1 bg "#2e5c8a" solid_color >/dev/null 2>&1
        sleep 1
        cargo_tests live-wayland-still live_rec_wayland_portal_frames live_rec_wayland_portal_gif_of_an_area
        no_wayland_skips live-wayland-still
    fi

    # rustshot itself in this session (XWayland is up for X11 clients).
    export DISPLAY; DISPLAY="$(grep -o -m1 'DISPLAY=:[0-9]*' "$LOGS/sway.log" | cut -d= -f2)"
    [ -n "$DISPLAY" ] || DISPLAY=:0
    log "rustshot full on Wayland (DISPLAY=$DISPLAY)"
    timeout 30 "$BIN" full --path "$SHOTS/03-rustshot-full.png" >"$LOGS/rustshot-full.log" 2>&1
    local frc=$?
    # Through the Screenshot portal (GetImage of the XWayland root fails
    # with BadMatch): a 1920x1080 PNG and exit 0.
    local fsz; fsz="$(identify -format '%wx%h' "$SHOTS/03-rustshot-full.png" 2>/dev/null)"
    if [ $frc -eq 0 ] && [ "$fsz" = 1920x1080 ]; then
        result rustshot-full pass "Screenshot portal: $fsz"
    else
        result rustshot-full fail "rc=$frc size '${fsz:-no file}': $(tail -2 "$LOGS/rustshot-full.log" | tr '\n' ' ')"
    fi
}

case "$MODE" in
    x11) mode_x11 ;;
    wayland) mode_wayland ;;
    *) echo "usage: run.sh x11|wayland" >&2; exit 2 ;;
esac

echo
echo "===== $MODE summary ($OUT/summary.txt)"
cat "$SUMMARY"
exit "$FAILED"
