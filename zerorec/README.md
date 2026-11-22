# ZeroRec (`zerorec`)

**ZeroRec** is a zero-copy, ultra-low-overhead screen recording and live streaming engine for macOS (Apple Silicon), extracted as a headless library and CLI package from `simple-record`.

It bypasses intermediate staging buffers, CPU-side memory copies, and external runtime dependencies like FFmpeg or OBS, interfacing directly with native Apple frameworks (`ScreenCaptureKit`, `VideoToolbox`, `CoreImage` on Metal, and `Security`).

---

## Benchmark: ZeroRec vs. FFmpeg

Tested on macOS (Apple Silicon M4 Pro), recording 60 fps video at 1.0x real time over 10 seconds (600 frames):

| Scenario | Engine | Mean CPU | Peak CPU | Max RAM (RSS) | Frames Captured |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **1080p60 H.264** | **ZeroRec** | **6.4%** | **7.9%** | **54.4 MB** | 600 / 600 (100%) |
| | FFmpeg | 81.6% | 89.1% | 141.2 MB | 600 / 600 (100%) |
| **4K60 HEVC** | **ZeroRec** | **16.4%** | **19.0%** | **99.2 MB** | 600 / 600 (100%) |
| | FFmpeg | 215.0% | 261.6% | 707.7 MB | 600 / 600 (100%) |

> [!NOTE]
> See [BENCHMARK.md](file:///Users/sahil/Coding/zerorec/BENCHMARK.md) for full details on methodology, the "Sleeping Parent" process-tree sampling fix, and [site/index.html](file:///Users/sahil/Coding/zerorec/site/index.html) for the interactive animated performance showcase.

### Why ZeroRec Is Dramatically Faster:
1. **Zero-Copy Pipeline**: `CVPixelBuffer`s are backed directly by hardware `IOSurface`s. Pixels never hit system RAM or get copied across user-space memory.
2. **Direct VideoToolbox**: Video is encoded in hardware without software format conversion loops.
3. **No Heavy Runtime**: Zero FFmpeg, Electron, or libobs dependencies.

---

## Build & Test

```sh
# Run the complete test suite (143 unit tests)
cargo test

# Build optimized release binary
cargo build --release

# Run the comparative performance benchmarks
python3 scripts/bench_compare.py
```

## CLI Usage

```sh
# Record the screen to 1080p60 H.264
target/release/zerorec record -r 1080p --fps 60 -o recording.mp4

# Record 4K60 HEVC with webcam overlay
target/release/zerorec record -r 4k --fps 60 --codec hevc --cam -o 4k-screencam.mp4

# Live stream over RTMP or SRT
target/release/zerorec record --stream rtmp://a.rtmp.youtube.com/live2/KEY
```
