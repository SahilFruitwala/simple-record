#!/bin/bash
# End-to-end check of the SRT output (src/live/ts.rs, src/live/srt.rs) with
# real H.264, HEVC and AAC:
# - against FFmpeg's libsrt (the reference implementation) as the
#   listener: plain, AES-128, AES-256, HEVC, 5% packet loss, and a wrong
#   passphrase;
# - against MediaMTX (an independent SRT implementation, in Go) when a
#   `mediamtx` binary is on PATH or in $MEDIAMTX: published over SRT, read
#   back over RTSP.
# Needs ffmpeg with SRT (`ffmpeg -protocols | grep srt`), libx264 and
# libx265. Exits non-zero on any failure.
set -euo pipefail
cd "$(dirname "$0")/.."
ffmpeg -hide_banner -protocols 2>/dev/null | grep -qx '  srt' || { echo "ffmpeg has no SRT support"; exit 2; }
cargo build -q --release --manifest-path portable/Cargo.toml --bins
BIN=portable/target/release
WORK=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$WORK"' EXIT

# 8 s of 30 fps video (keyframe every second, access unit delimiters, no
# B-frames, like the live encoder) and AAC.
ffmpeg -v error -y -f lavfi -i testsrc2=size=640x360:rate=30 -t 8 -c:v libx264 -preset veryfast -g 30 -bf 0 \
  -pix_fmt yuv420p -x264-params aud=1 -f h264 "$WORK/v.h264"
ffmpeg -v error -y -f lavfi -i testsrc2=size=640x360:rate=30 -t 8 -c:v libx265 -preset veryfast -pix_fmt yuv420p \
  -x265-params aud=1:keyint=30:min-keyint=30:bframes=0:log-level=error -f hevc "$WORK/v.hevc"
ffmpeg -v error -y -f lavfi -i sine=frequency=440:sample_rate=48000 -t 8 -c:a aac -ac 2 -b:a 128k -f adts "$WORK/a.aac"

# Run "$@" in the background, stopped after $1 seconds (macOS has no
# `timeout`).
bg_for() {
  local secs=$1; shift
  "$@" &
  local pid=$!
  (sleep "$secs"; kill "$pid" 2>/dev/null || true) &
}

fail=0
# check NAME FILE [VIDEO-CODEC]: every frame arrived and decodes cleanly.
check() {
  local name=$1 file=$2 codec=${3:-h264}
  local frames
  frames=$(ffprobe -v error -count_frames -show_entries stream=codec_name,nb_read_frames -of csv=p=0 "$file" | grep . | sort -u | tr '\n' ' ')
  if [ "$frames" = "aac,376 $codec,240 " ] && ffmpeg -v error -i "$file" -f null - 2>"$WORK/decode.log" && [ ! -s "$WORK/decode.log" ]; then
    echo "ok   $name ($frames)"
  else
    echo "FAIL $name: frames [$frames]"; cat "$WORK/decode.log" 2>/dev/null || true; fail=1
  fi
}

# Muxer alone.
"$BIN/srt_e2e" "$WORK" "$WORK/direct.ts" 2>/dev/null
check "ts muxer" "$WORK/direct.ts"
"$BIN/srt_e2e" "$WORK" "$WORK/direct-hevc.ts" hevc 2>/dev/null
check "ts muxer hevc" "$WORK/direct-hevc.ts" hevc

port=19100
# send NAME CALLER-QUERY LISTENER-QUERY [CODEC [RELAY-LOSS-UP RELAY-LOSS-DOWN]]
send() {
  local name=$1 cq=$2 lq=$3 codec=${4:-h264} up=${5:-} down=${6:-}
  port=$((port + 2))
  local out="$WORK/$port.ts" target=$port
  bg_for 30 ffmpeg -v error -y -i "srt://127.0.0.1:$port?mode=listener&$lq" -c copy -f mpegts "$out" 2>/dev/null
  if [ -n "$up" ]; then
    target=$((port + 1))
    bg_for 30 "$BIN/lossy_relay" "127.0.0.1:$target" "127.0.0.1:$port" "$up" "$down" 2>/dev/null
  fi
  sleep 1
  local extra=()
  [ "$codec" = hevc ] && extra=(hevc)
  "$BIN/srt_e2e" "$WORK" "srt://127.0.0.1:$target?$cq" "${extra[@]}" 2>"$WORK/client.log" || { echo "FAIL $name: $(tail -1 "$WORK/client.log")"; fail=1; return; }
  sleep 2
  echo "     $(tail -1 "$WORK/client.log")"
  check "$name" "$out" "$codec"
}

send "srt plain + stream id" "streamid=live/test&latency=200" "latency=200000"
send "srt aes-128" "passphrase=correct-horse-battery&pbkeylen=16" "passphrase=correct-horse-battery&pbkeylen=16"
send "srt aes-256" "passphrase=correct-horse-battery&pbkeylen=32" "passphrase=correct-horse-battery&pbkeylen=32"
send "srt hevc" "latency=200" "latency=200000" hevc
send "srt 5% loss" "latency=300" "latency=300000" h264 5 2

# A wrong passphrase must be refused, with that reason.
port=$((port + 2))
bg_for 10 ffmpeg -v error -y -i "srt://127.0.0.1:$port?mode=listener&passphrase=correct-horse-battery" -c copy -f mpegts "$WORK/x.ts" 2>/dev/null
sleep 1
if "$BIN/srt_e2e" "$WORK" "srt://127.0.0.1:$port?passphrase=wrong-horse-battery" 2>"$WORK/client.log"; then
  echo "FAIL wrong passphrase was accepted"; fail=1
elif grep -q "wrong passphrase" "$WORK/client.log"; then
  echo "ok   wrong passphrase refused"
else
  echo "FAIL wrong passphrase: $(tail -1 "$WORK/client.log")"; fail=1
fi

# MediaMTX: publish over SRT (stream ID publish:PATH), read back over RTSP
# while it plays. The reader joins mid-stream, so it checks a decodable
# stretch of both tracks rather than every frame.
MTX=${MEDIAMTX:-$(command -v mediamtx || true)}
if [ -n "$MTX" ]; then
  cat > "$WORK/mediamtx.yml" <<'YML'
logLevel: warn
rtsp: yes
rtspAddress: 127.0.0.1:18554
rtmp: no
hls: no
webrtc: no
srt: yes
srtAddress: 127.0.0.1:18890
paths:
  all_others:
YML
  bg_for 40 "$MTX" "$WORK/mediamtx.yml" >"$WORK/mtx.log" 2>&1
  sleep 2
  for codec in h264 hevc; do
    extra=()
    [ "$codec" = hevc ] && extra=(hevc)
    "$BIN/srt_e2e" "$WORK" "srt://127.0.0.1:18890?streamid=publish:e2e$codec" "${extra[@]}" 2>"$WORK/client.log" &
    pub=$!
    sleep 2
    ffmpeg -v error -y -rtsp_transport tcp -i "rtsp://127.0.0.1:18554/e2e$codec" -c copy -t 4 -f mpegts "$WORK/mtx-$codec.ts" 2>"$WORK/rtsp.log" || true
    wait $pub || true
    frames=$(ffprobe -v error -count_frames -show_entries stream=codec_name,nb_read_frames -of csv=p=0 "$WORK/mtx-$codec.ts" 2>/dev/null | grep . | sort -u | tr '\n' ' ' || true)
    v=$(echo "$frames" | grep -o "$codec,[0-9]*" | cut -d, -f2 || true)
    a=$(echo "$frames" | grep -o "aac,[0-9]*" | cut -d, -f2 || true)
    if [ "${v:-0}" -ge 60 ] && [ "${a:-0}" -ge 90 ] && ffmpeg -v error -i "$WORK/mtx-$codec.ts" -f null - 2>"$WORK/decode.log" && [ ! -s "$WORK/decode.log" ]; then
      echo "ok   mediamtx srt $codec ($frames)"
    else
      echo "FAIL mediamtx srt $codec: frames [$frames]"
      tail -3 "$WORK/client.log" "$WORK/rtsp.log" "$WORK/mtx.log" "$WORK/decode.log" 2>/dev/null || true
      fail=1
    fi
  done
else
  echo "skip mediamtx (no mediamtx binary; set MEDIAMTX or put it on PATH)"
fi
exit $fail
