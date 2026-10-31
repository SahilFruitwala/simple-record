#!/usr/bin/env python3
"""Count clicks in a recording's audio tracks: pure tones have no energy above
6 kHz, so any burst there (past the AAC noise floor) is a discontinuity.

    scripts/audioclicks.py FILE.mov
"""
import array
import subprocess
import sys

path = sys.argv[1]
probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0", path], capture_output=True, text=True).stdout.split()
names = ["mix", "system", "mic"]
for n, _ in enumerate(probe):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-map", f"0:a:{n}", "-af", "highpass=f=6000:poles=2,highpass=f=6000:poles=2", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"], capture_output=True).stdout
    x = array.array("f", raw)[4800:-4800]
    clicks, last, peak = 0, -48000, 0.0
    for i, v in enumerate(x):
        a = abs(v)
        peak = max(peak, a)
        if a > 0.003 and i - last > 2400:
            clicks += 1
            last = i
    print(f"{names[n] if n < 3 else n:7} {len(x) / 48000:6.1f} s  clicks {clicks:3}  peak above 6 kHz {peak:.4f}")
