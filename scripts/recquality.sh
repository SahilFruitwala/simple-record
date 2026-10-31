#!/bin/sh
# Recording-encoder quality: encode lossless clips through the recording writer
# in real time and score them with VMAF and PSNR-Y.
#
#   scripts/recquality.sh WxH FPS [CASE...]
#
# CASE is NAME:CODEC:BPP[:ENV=1...], where BPP is bits per pixel per frame.
# Sources: the text card (scripts/testcard.swift) scrolling, and ffmpeg's
# testsrc2 (moving graphics, text and gradients over the whole frame).
set -eu
cd "$(dirname "$0")/.."
SIZE=$1; FPS=$2; shift 2
W=${SIZE%x*}; H=${SIZE#*x}
OUT="${REC_TEST_DIR:-$HOME/Movies/simple-record-tests}/recquality"
mkdir -p "$OUT"
[ -f "$OUT/card.png" ] || swift scripts/testcard.swift "$OUT/card.png"
[ -f "$OUT/scroll-$SIZE-$FPS.mkv" ] || ffmpeg -v error -loop 1 -framerate "$FPS" -i "$OUT/card.png" -t 4 \
  -vf "scale=$W:$H:flags=lanczos,scroll=v=0.0028,format=bgra" -c:v ffv1 "$OUT/scroll-$SIZE-$FPS.mkv"
[ -f "$OUT/motion-$SIZE-$FPS.mkv" ] || ffmpeg -v error -f lavfi -i "testsrc2=s=$SIZE:r=$FPS" -t 4 \
  -vf format=bgra -c:v ffv1 "$OUT/motion-$SIZE-$FPS.mkv"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/simple-record-target}"
cargo test --release --no-run -q 2>/dev/null
for src in scroll motion; do
  for c in "$@"; do
    name=${c%%:*}; rest=${c#*:}; codec=${rest%%:*}; rest=${rest#*:}; bpp=${rest%%:*}
    envs=""; case "$rest" in *:*) envs=$(echo "${rest#*:}" | tr ':' ' ') ;; esac
    f="$OUT/$src-$SIZE-$FPS-$name.mov"
    bitrate=$(awk "BEGIN{printf \"%d\", $W*$H*$FPS*$bpp}")
    env $envs REC_W_W="$W" REC_W_H="$H" REC_W_FPS="$FPS" REC_W_CODEC="$codec" REC_W_BITRATE="$bitrate" \
      REC_W_SRC="$OUT/$src-$SIZE-$FPS.mkv" REC_W_OUT="$f" \
      cargo test --release -q -- --ignored --exact writer::tests::encode_file >/dev/null 2>&1
    ff="[0:v]setpts=N/($FPS*TB),setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709,format=yuv420p,split=2[d1][d2];"
    ff="$ff[1:v]setpts=N/($FPS*TB),format=rgb24,scale=in_range=pc:out_range=tv:out_color_matrix=bt709:flags=accurate_rnd+full_chroma_int,format=yuv420p,split=2[r1][r2];"
    ff="$ff[d1][r1]psnr[p];[d2][r2]libvmaf=n_threads=8[v]"
    ffmpeg -v info -i "$f" -i "$OUT/$src-$SIZE-$FPS.mkv" -filter_complex "$ff" -map "[p]" -f null - -map "[v]" -f null - 2>"$f.metrics" >/dev/null || true
    psnr=$(grep -o 'PSNR y:[0-9.inf]*' "$f.metrics" | tail -1 | cut -d: -f2)
    vmaf=$(grep -o 'VMAF score: [0-9.]*' "$f.metrics" | tail -1 | awk '{print $3}')
    size=$(wc -c <"$f")
    printf '%-7s %-16s bpp %-5s PSNR-Y %-7s VMAF %-7s %s Mb/s\n' "$src" "$name" "$bpp" "$psnr" "$vmaf" \
      "$(echo "$size" | awk '{printf "%.1f", $1*8/4/1e6}')"
  done
done
