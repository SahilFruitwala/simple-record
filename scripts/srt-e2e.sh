#!/bin/bash
# End-to-end check of the SRT output against FFmpeg's libsrt (the reference
# implementation) as the listener: plain, AES-128, AES-256, 5% packet loss,
# and a wrong passphrase. Runs on Linux or macOS with an ffmpeg built with
# libsrt (`ffmpeg -protocols | grep srt`). Exits non-zero on any failure.
set -euo pipefail
cd "$(dirname "$0")/.."
ffmpeg -hide_banner -protocols 2>/dev/null | grep -qx '  srt' || { echo "ffmpeg has no SRT support"; exit 2; }
cargo build -q --release --manifest-path portable/Cargo.toml --bins
BIN=portable/target/release
WORK=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$WORK"' EXIT

# 8 s of 30 fps H.264 (keyframe every second, access unit delimiters) and AAC.
ffmpeg -v error -y -f lavfi -i testsrc2=size=640x360:rate=30 -t 8 -c:v libx264 -preset veryfast -g 30 -bf 0 \
  -pix_fmt yuv420p -x264-params aud=1 -f h264 "$WORK/v.h264"
ffmpeg -v error -y -f lavfi -i sine=frequency=440:sample_rate=48000 -t 8 -c:a aac -ac 2 -b:a 128k -f adts "$WORK/a.aac"

fail=0
# The received file must hold every frame and decode without errors.
check() {
  local name=$1 file=$2
  local frames
  frames=$(ffprobe -v error -count_frames -show_entries stream=codec_name,nb_read_frames -of csv=p=0 "$file" | grep . | sort -u | tr '\n' ' ')
  if [ "$frames" = "aac,376 h264,240 " ] && ffmpeg -v error -i "$file" -f null - 2>"$WORK/decode.log" && [ ! -s "$WORK/decode.log" ]; then
    echo "ok   $name ($frames)"
  else
    echo "FAIL $name: frames [$frames]"; cat "$WORK/decode.log" || true; fail=1
  fi
}

# Muxer alone.
"$BIN/srt_e2e" "$WORK" "$WORK/direct.ts" 2>/dev/null
check "ts muxer" "$WORK/direct.ts"

port=19100
# send NAME CALLER-QUERY LISTENER-QUERY [RELAY-LOSS-UP RELAY-LOSS-DOWN]
send() {
  local name=$1 cq=$2 lq=$3 up=${4:-} down=${5:-}
  port=$((port + 2))
  local out="$WORK/$port.ts" target=$port
  timeout 30 ffmpeg -v error -y -i "srt://127.0.0.1:$port?mode=listener&$lq" -c copy -f mpegts "$out" 2>/dev/null &
  if [ -n "$up" ]; then
    target=$((port + 1))
    timeout 30 "$BIN/lossy_relay" "127.0.0.1:$target" "127.0.0.1:$port" "$up" "$down" 2>/dev/null &
  fi
  sleep 1
  "$BIN/srt_e2e" "$WORK" "srt://127.0.0.1:$target?$cq" 2>"$WORK/client.log" || { echo "FAIL $name: $(tail -1 "$WORK/client.log")"; fail=1; return; }
  sleep 2
  echo "     $(tail -1 "$WORK/client.log")"
  check "$name" "$out"
}

send "srt plain + stream id" "streamid=live/test&latency=200" "latency=200000"
send "srt aes-128" "passphrase=correct-horse-battery&pbkeylen=16" "passphrase=correct-horse-battery&pbkeylen=16"
send "srt aes-256" "passphrase=correct-horse-battery&pbkeylen=32" "passphrase=correct-horse-battery&pbkeylen=32"
send "srt 5% loss" "latency=300" "latency=300000" 5 2

# A wrong passphrase must be refused, with that reason.
port=$((port + 2))
timeout 10 ffmpeg -v error -y -i "srt://127.0.0.1:$port?mode=listener&passphrase=correct-horse-battery" -c copy -f mpegts "$WORK/x.ts" 2>/dev/null &
sleep 1
if "$BIN/srt_e2e" "$WORK" "srt://127.0.0.1:$port?passphrase=wrong-horse-battery" 2>"$WORK/client.log"; then
  echo "FAIL wrong passphrase was accepted"; fail=1
elif grep -q "wrong passphrase" "$WORK/client.log"; then
  echo "ok   wrong passphrase refused"
else
  echo "FAIL wrong passphrase: $(tail -1 "$WORK/client.log")"; fail=1
fi
exit $fail
