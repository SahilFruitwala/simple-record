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
# scene NAME LAYERS...: record the test pattern under these layers.
scene() {
  local name=$1; shift
  local layers='{"content": "screen"}'
  for l in "$@"; do layers="$layers, $l"; done
  printf '{"name": "%s", "layers": [%s]}' "$name" "$layers" > "$WORK/$name.json"
  REC_DEBUG_LAYERS=${DEBUG_LAYERS:-} "$REC" record -q --test-pattern --no-mic --no-system-audio -r 640x360 -f 30 --scene-file "$WORK/$name.json" -t 4 -o "$WORK/$name.mov" 2>"$WORK/$name.log" || { cat "$WORK/$name.log"; exit 1; }
  grep -v IOServiceMatching "$WORK/$name.log" || true
}
RED='{"content": {"media": {"path": "'$WORK'/red.mp4", "looped": true}}, "rect": [0, 0, 0.5, 1], "fit": "cover"}'
PAGE='{"content": {"browser": {"url": "'$WORK'/page.html"}}, "rect": [0.5, 0, 0.5, 1], "fit": "cover"}'
KEYED='{"content": {"media": {"path": "'$WORK'/green.mp4", "looped": true}}, "rect": [0.6, 0.3, 0.3, 0.4], "fit": "cover", "effects": {"chroma_key": {"color": [0, 255, 0]}}}'
scene video "$RED"
scene page "$PAGE"
scene video_page "$RED" "$PAGE"
scene video_keyed "$RED" "$KEYED"
DEBUG_LAYERS=1 scene all "$RED" "$PAGE" "$KEYED"
# The RGB of an 8x8 patch around (x, y) at 2.5 s of recording $1 (between
# the test pattern's white flashes).
rgb() {
  ffmpeg -v error -ss 2.5 -i "$WORK/$1.mov" -frames:v 1 -vf "crop=8:8:$2:$3,scale=1:1" -f rawvideo -pix_fmt rgb24 - | od -An -tu1 | tr -s ' ' | sed 's/^ //'
}
# A coarse picture of the frame (4 rows of 8 patches), for diagnosis.
grid() {
  echo "     $1:"
  for y in 20 110 200 290; do
    line=""
    for x in 20 100 180 260 340 420 500 580; do line="$line [$(rgb "$1" $x $y)]"; done
    echo "      $line"
  done
}
fail=0
expect() {
  local name=$1 file=$2 x=$3 y=$4 want=$5
  read -r r g b <<<"$(rgb "$file" "$x" "$y")"
  local ok=0
  case $want in
    red) [ "$r" -gt 180 ] && [ "$g" -lt 60 ] && [ "$b" -lt 60 ] && ok=1 ;;
    blue) [ "$b" -gt 180 ] && [ "$r" -lt 60 ] && [ "$g" -lt 60 ] && ok=1 ;;
    pattern) { [ "$r" -lt 180 ] || [ "$g" -gt 60 ] || [ "$b" -gt 60 ]; } && { [ "$b" -lt 180 ] || [ "$r" -gt 60 ] || [ "$g" -gt 60 ]; } && ok=1 ;;
  esac
  if [ $ok = 1 ]; then echo "ok   $name ($r $g $b)"; else echo "FAIL $name: want $want, got ($r $g $b)"; grid "$file"; fail=1; fi
}
expect "video file" video 156 176 red
expect "test pattern beside the video" video 496 176 pattern
expect "web page" page 496 40 blue
expect "test pattern beside the page" page 156 176 pattern
expect "video beside a page" video_page 156 176 red
expect "page beside a video" video_page 496 40 blue
expect "video beside a keyed video" video_keyed 156 176 red
expect "keyed video (pattern through keyed green)" video_keyed 476 176 pattern
expect "video file under a page" all 156 176 red
expect "web page beside a video" all 496 40 blue
expect "chroma key (page through keyed green)" all 476 176 blue
exit $fail
