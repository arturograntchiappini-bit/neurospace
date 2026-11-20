#!/bin/sh
# Regenerates docs/images/*.png: real screenshots of the Linux release of Neurospace
# on the demo vault (docs/demo-vault), taken inside a container under Xvfb so no window
# ever appears on your screen. The app is driven only with xdotool, inside the container.
#
# Needs the Linux release tarball (neurospace-<version>-linux-x86_64.tar.gz, see
# scripts/linux-release.sh) in a folder you mount read-only at /dist, and this repository
# mounted at /src. Run from the repository root.
#
# PowerShell:
#   docker run --rm -v "<FOLDER WITH THE TARBALL>:/dist:ro" -v "${PWD}:/src" debian:bookworm sh /src/scripts/screenshots.sh
# Linux shell:
#   docker run --rm -v "<FOLDER WITH THE TARBALL>:/dist:ro" -v "$PWD:/src" debian:bookworm sh /src/scripts/screenshots.sh
#
# The glass is the simulated one (Mica exists only on Windows). Coordinates below are for
# the 1440x900 window; the graph is shot in a 1920x1200 window so «Fit graph» zooms far
# enough for the note names to show (they appear from 150 %).
set -eu

OUT=/src/docs/images
APP=/opt/neurospace
VAULT=/work/demo-vault
export DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 NS_STATE_DIR=/tmp/ns-state

apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq xvfb x11-apps xdotool netpbm pngquant \
    libgl1-mesa-dri libegl1 libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 >/dev/null

mkdir -p "$APP" "$OUT" "$NS_STATE_DIR" /work
tar -xzf /dist/neurospace-*-linux-x86_64.tar.gz -C "$APP" --strip-components=1
rm -rf "$VAULT"
cp -r /src/docs/demo-vault "$VAULT"

Xvfb :99 -screen 0 1920x1200x24 >/dev/null 2>&1 &
until [ -S /tmp/.X11-unix/X99 ]; do sleep 0.2; done

PID=""
WIN=""

# stop_app: end the running app, if any.
stop_app() {
    [ -z "$PID" ] || kill "$PID" 2>/dev/null || true
    PID=""
    sleep 1
}

# start_app THEME [VAULT]: settings first, then the app in a 1440x900 window at 0,0.
start_app() {
    stop_app
    last_vault=null
    [ -z "${2:-}" ] || last_vault="\"$2\""
    printf '{"last_vault":%s,"theme":"%s","reduce_transparency":false,"reduce_motion":true,"zoom":1.0}\n' \
        "$last_vault" "$1" >"$NS_STATE_DIR/app.json"
    "$APP/neurospace" ${2:+"$2"} >/tmp/app.log 2>&1 </dev/null &
    PID=$!
    WIN=$(xdotool search --sync --onlyvisible --pid "$PID" | head -n 1)
    xdotool windowmove "$WIN" 0 0 windowsize "$WIN" 1440 900
    sleep 3
}

key() { xdotool windowfocus --sync "$WIN" key --clearmodifiers "$@"; }
click() { xdotool mousemove "$1" "$2" click 1; sleep 1; }
park_mouse() { xdotool mousemove "$1" "$2"; sleep 1; }

# shot NAME WIDTH HEIGHT [LEFT TOP]: crop the screen, 8-bit PNG.
shot() {
    xwd -root -silent | xwdtopnm 2>/dev/null \
        | pnmcut -left "${4:-0}" -top "${5:-0}" -width "$2" -height "$3" \
        | pnmdepth 255 | pnmtopng >"$OUT/$1.png" 2>/dev/null
}

# open_note QUERY: quick open, type, take the first match.
open_note() {
    key ctrl+o
    sleep 1
    xdotool type --delay 80 "$1"
    sleep 1
    key Return
    sleep 2
}

# show_graph NAME: the whole graph with labels, in a 1920x1200 window.
show_graph() {
    xdotool windowsize "$WIN" 1920 1200
    sleep 2
    key ctrl+g
    sleep 3
    click 408 100
    park_mouse 1500 1100
    shot "$1" 1920 1200
}

# Welcome screen, cropped around its card.
start_app light
shot welcome 900 600 270 124

# Light theme: rendered note, quick open, graph.
start_app light "$VAULT"
click 76 145
open_note roadmap
key ctrl+e
park_mouse 1100 800
shot preview-light 1440 900
key ctrl+o
sleep 1
xdotool type --delay 80 re
sleep 1
shot quick-open 1440 900
key Escape
sleep 1
show_graph graph-light

# Dark theme: editor with the side panel, sync window (nothing is created), graph.
start_app dark "$VAULT"
click 76 171
open_note zettel
park_mouse 1100 800
shot editor-dark 1440 900
click 1319 28
shot sync-window 1440 900
key Escape
sleep 1
show_graph graph-dark

stop_app

# 256 colours keep each file small; text stays sharp on these flat surfaces.
for f in "$OUT"/*.png; do
    pngquant --quality=70-100 --strip --force --output "$f" "$f"
done

# Self-check: every capture exists and stays within 300 KB.
for name in welcome editor-dark preview-light graph-dark graph-light quick-open sync-window; do
    size=$(stat -c %s "$OUT/$name.png")
    [ "$size" -gt 10000 ] && [ "$size" -le 307200 ] || { echo "$name.png: bad size $size" >&2; exit 1; }
done
chown -R "$(stat -c %u:%g /src)" "$OUT"
ls -l "$OUT"
