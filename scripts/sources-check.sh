#!/bin/bash
# Records the test pattern with a scene of a video file (left half), a web
# page (right half) and a chroma-keyed video (a box keyed out over the
# page), then checks the pixels: the file's red, the page's blue, and the
# page showing through where the key removed green. Needs ffmpeg; no
# Screen Recording permission (the test pattern stands in for the screen).
# Usage: scripts/sources-check.sh [REC_BINARY]
set -euo pipefail
cd "$(dirname "$0")/.."
REC=${1:-target/debug/rec-rs}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
ffmpeg -v error -y -f lavfi -i color=c=red:size=320x180:rate=30 -t 2 -pix_fmt yuv420p "$WORK/red.mp4"
ffmpeg -v error -y -f lavfi -i color=c=0x00ff00:size=320x180:rate=30 -t 2 -pix_fmt yuv420p "$WORK/green.mp4"
cat > "$WORK/page.html" <<'HTML'
<!doctype html><html><body style="margin:0;background:#0000ff"></body></html>
HTML
cat > "$WORK/scene.json" <<JSON
{
  "name": "Sources",
  "layers": [
    {"content": "screen"},
    {"content": {"media": {"path": "$WORK/red.mp4", "looped": true}}, "rect": [0, 0, 0.5, 1], "fit": "cover"},
    {"content": {"browser": {"url": "$WORK/page.html"}}, "rect": [0.5, 0, 0.5, 1], "fit": "cover"},
    {"content": {"media": {"path": "$WORK/green.mp4", "looped": true}}, "rect": [0.6, 0.3, 0.3, 0.4], "fit": "cover",
     "effects": {"chroma_key": {"color": [0, 255, 0]}}}
  ]
}
JSON
"$REC" record -q --test-pattern --no-mic --no-system-audio -r 640x360 -f 30 --scene-file "$WORK/scene.json" -t 4 -o "$WORK/out.mov" 2>"$WORK/rec.log" || { cat "$WORK/rec.log"; exit 1; }
cat "$WORK/rec.log"
# The RGB of an 8x8 patch around (x, y) at 3 s.
rgb() {
  ffmpeg -v error -ss 3 -i "$WORK/out.mov" -frames:v 1 -vf "crop=8:8:$1:$2,scale=1:1" -f rawvideo -pix_fmt rgb24 - | od -An -tu1 | tr -s ' ' | sed 's/^ //'
}
fail=0
expect() {
  local name=$1 x=$2 y=$3 want=$4
  read -r r g b <<<"$(rgb "$x" "$y")"
  local ok=0
  case $want in
    red) [ "$r" -gt 180 ] && [ "$g" -lt 60 ] && [ "$b" -lt 60 ] && ok=1 ;;
    blue) [ "$b" -gt 180 ] && [ "$r" -lt 60 ] && [ "$g" -lt 60 ] && ok=1 ;;
  esac
  if [ $ok = 1 ]; then echo "ok   $name ($r $g $b)"; else echo "FAIL $name: want $want, got ($r $g $b)"; fail=1; fi
}
expect "video file" 156 176 red
expect "web page" 496 40 blue
expect "chroma key (page through keyed green)" 476 176 blue
exit $fail
