#!/usr/bin/env bash
# Starts a headless PipeWire graph for the engine's Linux device tests: no
# sound card needed. Prints `export` lines to eval (the environment the tests
# need to reach the server):
#
#   eval "$(tools/ci/headless-pipewire.sh)"
#   SAINE_TEST_LOOPBACK_INPUT=saine-loop-source ctest --test-dir engine/build
#
# The graph: a session D-Bus, pipewire and wireplumber, a stereo null sink
# ("saine-null-sink", the default output) and pw-loopback from that sink's
# monitor to a virtual source ("saine-loop-source"), so an input opened on
# the source hears exactly what was played. The null sink is driven by the
# system clock like a real device, so the engine's streams run in real time.
#
# Needs: pipewire pipewire-bin wireplumber dbus (Debian/Ubuntu package names).
# Logs go to $XDG_RUNTIME_DIR/*.log. Stop with: kill $(cat $XDG_RUNTIME_DIR/saine-pw.pids)
set -euo pipefail

runtime="${XDG_RUNTIME_DIR:-}"
if [ -z "$runtime" ] || [ ! -w "$runtime" ]; then
    runtime="$(mktemp -d "${TMPDIR:-/tmp}/saine-pw.XXXXXX")"
fi
chmod 700 "$runtime"
export XDG_RUNTIME_DIR="$runtime"
pids="$runtime/saine-pw.pids"
: > "$pids"

if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
    export DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus"
    dbus-daemon --session --address="$DBUS_SESSION_BUS_ADDRESS" --fork --print-pid >> "$pids"
fi

pipewire > "$runtime/pipewire.log" 2>&1 &
echo $! >> "$pids"
for _ in $(seq 50); do [ -S "$runtime/pipewire-0" ] && break; sleep 0.1; done
[ -S "$runtime/pipewire-0" ] || { echo "pipewire did not start" >&2; cat "$runtime/pipewire.log" >&2; exit 1; }
wireplumber > "$runtime/wireplumber.log" 2>&1 &
echo $! >> "$pids"

# object.linger keeps the nodes after pw-cli exits.
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=saine-null-sink
    node.description="Saine Null Sink" media.class=Audio/Sink object.linger=true
    audio.position=[FL FR] }' > /dev/null
pw-loopback -n saine-loopback \
    --capture-props='stream.capture.sink=true target.object=saine-null-sink node.passive=true' \
    --playback-props='media.class=Audio/Source node.name=saine-loop-source
        node.description="Saine Loopback Source" audio.position=[FL,FR]' \
    > "$runtime/pw-loopback.log" 2>&1 &
echo $! >> "$pids"

# Wait until wireplumber has made the null sink the default output and the
# loopback source exists.
for _ in $(seq 100); do
    status="$(wpctl status 2>/dev/null || true)"
    if echo "$status" | grep -q '\*.*Saine Null Sink' && echo "$status" | grep -q 'Saine Loopback Source'; then
        echo "export XDG_RUNTIME_DIR='$runtime'"
        echo "export DBUS_SESSION_BUS_ADDRESS='$DBUS_SESSION_BUS_ADDRESS'"
        exit 0
    fi
    sleep 0.1
done
echo "the headless PipeWire graph did not come up" >&2
wpctl status >&2 || true
tail -20 "$runtime"/*.log >&2 || true
exit 1
