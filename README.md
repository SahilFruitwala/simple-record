# rec-rs

A simple, fast, reliable screen and camera recorder for macOS, written in Rust, as an alternative
to OBS for recording YouTube videos. It records the screen, system audio, microphone and webcam
into one `.mov` or `.mp4` that plays in QuickTime and imports into Final Cut, Premiere and
DaVinci Resolve. **Scenes** lay out the screen, the webcam, other displays, images and text,
from nine ready-made templates (Presenter, Side by Side, Reaction, Vertical for Shorts…) or
your own, and switch live with animated transitions. It can also keep a **replay buffer** of
the last seconds and **stream** over RTMP/RTMPS (YouTube, Twitch, any server), SRT or WHIP
(WebRTC) to several destinations at once, from one shared hardware encode, with or without
recording.

It uses only Apple frameworks:

- **ScreenCaptureKit** for display and window capture, system audio and the microphone. Frames
  are IOSurface-backed `CVPixelBuffer`s that are never copied. The screen is captured at its
  native resolution (a 5K display at 5120x2880) and scaled to the output by a Lanczos filter in
  Metal Performance Shaders, which is much sharper than ScreenCaptureKit's own scaling (see
  [Picture quality](PERFORMANCE.md#picture-quality-2026-10-12)).
- **AVCaptureSession** for the webcam, and optionally the microphone.
- **CoreImage on Metal** for compositing scenes: layers with shapes, borders and shadows,
  gradients, images and text, and transitions.
- **VideoToolbox through AVAssetWriter** for hardware H.264, HEVC or ProRes 422, plus AAC
  audio.
- **VideoToolbox and AudioConverter directly** for the live encode (H.264, AAC, Opus) that feeds
  the replay buffer and streams. RTMP is a built-in client (TLS through the system's
  Security framework); SRT and its MPEG-TS muxer are built in too (AES encryption through the
  RustCrypto crates); WHIP uses [str0m](https://github.com/algesten/str0m) (MIT/Apache) for
  WebRTC with Apple CryptoKit. Stream keys live in the macOS Keychain.
- **RNNoise** (the pure-Rust [nnnoiseless](https://github.com/jneem/nnnoiseless), BSD) for mic
  noise suppression, next to a built-in low cut, noise gate and compressor.

There is no FFmpeg, OBS or Electron at runtime. The UI is egui on wgpu in the paper-rs style:
transparent titlebar and the Inter font.

It needs macOS 15 or later on Apple Silicon. It was developed and tested on macOS 27 on an M4
Pro MacBook Pro.

The code is a clean-room implementation, licensed MIT OR Apache-2.0. OBS (GPL-2.0) was studied
for its architecture only and no OBS code was copied. See
[analysis/OBS_ARCHITECTURE.md](analysis/OBS_ARCHITECTURE.md).

## Screenshots

![Rec main window: live preview of the recorded screen, scene picker, sources, record button and level meters](docs/screenshots/main.png)

*The main window: a live preview of exactly what gets recorded (here a window showing a test
card), the scene picker (Screen, Screen + Cam, Camera), sources, audio, replay buffer and
stream, record/pause, the timer and level meters.*

| Screen + Cam, squircle with a white border | Screen + Cam, circle moved off the corner |
|---|---|
| ![Camera overlay as a squircle with a white border, snapped to the bottom-right corner](docs/screenshots/cam-squircle.png) | ![Camera overlay as a circle with a yellow border, dragged to a custom position](docs/screenshots/cam-circle.png) |

*Drag the camera anywhere in the preview, or snap it to a corner. It can be a circle, rounded
rectangle, square, pill or squircle, with adjustable roundness, border and shadow. Every change
is live and baked into the recording. The camera here is a virtual camera with a blank feed
(these two are from an earlier build; the overlay controls are unchanged).*

| Settings | Menu bar |
|---|---|
| ![Settings window: output folder, format, resolution, frame rate, codec, bitrate, audio and camera options](docs/screenshots/settings.png) | ![Menu bar menu: Start Recording (⇧⌘R), Pause, Show Rec Window, Settings, Reveal Last Recording, Open Recordings Folder, Quit](docs/screenshots/menu.png) |

## Reverse-engineered origin

rec-rs is an independent, clean-room-style reimplementation. It was reverse engineered from
[OBS Studio](https://obsproject.com) by studying its architecture and behavior with
[REA](https://github.com/morluto/rea) and Claude Code. It is not affiliated with or endorsed by
the OBS Project. No original OBS code or assets are included in this repository.

## Build

```sh
./scripts/build.sh          # cargo build --release, then assemble and sign dist/Rec.app
dist/rec-rs --help          # dist/rec is a short alias
```

Build output goes to `~/.cache/rec-rs-target` (`CARGO_TARGET_DIR`), never into the repo,
because the repo is synced between machines. The script signs `dist/Rec.app` with your Apple
Development identity, so the macOS privacy grants survive rebuilds. Override it with
`REC_SIGN_IDENTITY`.

Tests:

```sh
cargo test                       # unit tests: pause timeline, mixer, CLI, settings, scenes
scripts/selftest.sh              # records through every pipeline and verifies each file
scripts/selftest.sh --no-screen  # skip the cases that need Screen Recording
scripts/soak.sh 1800             # 30-minute 4K60 soak with CPU/memory sampling and ffprobe checks
scripts/synccheck.py FILE        # A/V offset of a flash + beep recording
```

Test recordings go to `~/Movies/rec-rs-tests/`.

The platform-independent parts (scene layouts, shapes, mic filters, shortcuts, MPEG-TS, SRT)
also build and test on any OS, Linux included, from `portable/`; the SRT output is checked end
to end against FFmpeg's libsrt by a script. CI (`.github/workflows/ci.yml`) runs both on
Linux, and builds, lints and tests the app on macOS:

```sh
cargo test --manifest-path portable/Cargo.toml
scripts/srt-e2e.sh               # needs ffmpeg with SRT (ffmpeg -protocols | grep srt)
```

## Permissions (one time)

```sh
dist/rec-rs permissions --request
```

Then enable **Rec** under **System Settings → Privacy & Security → Screen & System Audio
Recording**. Microphone and camera are approved from the system prompts.

- `rec-rs` re-launches itself with responsibility disclaimed. macOS therefore asks about and
  remembers the signed **Rec** app rather than your terminal.
- If macOS asks whether Rec may "bypass the system private window picker", click **Allow**.
- `dist/rec-rs permissions` shows the current status.

## The app

```sh
open dist/Rec.app          # or: dist/rec-rs   (or: dist/rec-rs gui [--settings] [--start-recording] [--window ID])
```

The **main window** contains:

- **Live preview** of exactly what will be recorded, including the webcam overlay, edge to
  edge in the source's shape. Rec's own windows are excluded from capture. It is rendered at
  the preview's exact size in pixels (2x on Retina) with the Lanczos scaler and drawn 1:1,
  without copying frames through the CPU. Settings has a low-power option (half resolution, 15 fps),
  and nothing is drawn while the window is hidden or the screen is locked.
- **Scene**: a gallery of miniatures drawn from the real layouts.
  - Templates: **Screen**, **Screen + Camera** (picture-in-picture), **Camera**,
    **Presenter** (the screen inset on a gradient with rounded corners and a shadow, the
    camera over a corner), **Side by Side**, **Camera + Screen**, **Split**, **Reaction** (the
    camera full frame, the screen in a corner) and **Vertical** (for Shorts and Reels: the
    screen on top, the camera below, on a 9:16 frame).
  - In the picture-in-picture scenes, **drag the camera in the preview** to place it
    anywhere; released near a corner, it snaps there and stays in that corner when you resize
    it. Shape: circle, rounded rectangle, square, pill or squircle, with a roundness slider.
    Size, border (width and color), drop shadow and mirroring are in the sidebar.
  - **Custom scenes**: New Scene starts from the scene shown. The Layers card adds the screen,
    the camera, other displays, images (a logo, a frame), text (a name tag, with a box) and
    color layers; shows, hides, reorders and removes them; and sets each one's box, fit, shape,
    roundness, border and shadow. Drag layers in the preview to move them and their corners to
    resize them; they snap to the frame's edges and center. Export a scene as JSON for
    `record --scene-file`, and import it back.
  - **Frame**: the screen's own shape (no bars), or 16:9, 9:16, 1:1 or 4:5, with a gradient
    background (Settings) behind layers that don't cover it.
  - Scene switches are live, also while recording (provided the recording started with the
    sources the scene needs), with a **Move** (layers glide to their new places), **Fade** or
    **Cut** transition. Everything is baked into the recording by the CoreImage compositor, so
    there is no extra encode; layer outlines are rendered once per size and style, so moving a
    layer only changes a transform.
- **Sources**:
  - Display.
  - Entire display, a region of it (Area > Select…: drag over the preview), every window of
    one app, or a single window.
  - Camera, at 720p, 1080p or 4K, optionally also saved alone to `…-camera.mov` on the
    recording's timeline for editing.
  - Microphone, with live noise suppression, noise gate, compressor and low cut.
  - System audio.
- **Record / Stop** (⇧⌘R from anywhere by default), **Pause / Resume**, the recording timer,
  and **level meters** for system audio and mic with peak hold.
- **Reveal Last** and **Open Folder**.
- **Replay buffer**: keep the last 15–300 s (default 60) in memory, shown with its memory use.
  **Save Replay** or **⇧⌘S** from anywhere writes it to an MP4 in the output folder without
  stopping anything. ⇧⌘S is only taken while the buffer is on, because it is Save As in most
  apps.
- **Stream**: YouTube, Twitch or a custom RTMP/RTMPS, SRT or WHIP URL, and **+ Add
  destination** for more at once (one encode feeds them all, within the strictest limits of
  all of them). The stream key is typed once and stored in the Keychain (per destination URL),
  never in the settings file or logs. **Go Live** / **Stop Stream**, with each stream's
  duration, bitrate and dropped frames, and a LIVE badge.

  The replay buffer and the stream run with or without a recording. Starting or stopping a
  recording, or closing the window, doesn't interrupt them: the capture keeps running for them
  and short gaps while the app swaps capture sessions are bridged (the last frame repeats,
  silence fills the audio).

The **settings window** (⌘, or the Settings… button) covers:

- Output folder and container (`.mov` or `.mp4`).
- Resolution: 720p, 1080p, 1440p, 4K or native. The frame always has the shape of the display
  or window (4K of a 16:10 display is 3456x2160): no black bars, nothing cropped.
- Frame rate, codec (auto, H.264, HEVC, ProRes), bitrate, and HDR (experimental).
- Cursor and click highlighting.
- Scenes: frame shape, background gradient, transition and its length.
- Audio devices, track layout, gains, and the mic filters' thresholds, ratio and makeup gain.
- Shortcuts: click an action and press the keys. Start/stop recording, pause, save replay, go
  live and next/previous scene.
- Camera device, plus the overlay: shape, roundness, size, border, shadow, corner presets and
  mirroring.
- Live encode for the replay buffer and streams: resolution, video bitrate, CBR, keyframe
  interval and audio bitrate (locked while streaming).

Settings persist in `~/Library/Application Support/rec-rs/settings.json`.

Rec is a regular app with a Dock icon and an app menu (Settings… ⌘,, Hide ⌘H, Quit ⌘Q, Window ▸
Minimize, Close, Show Rec Window). Closing the main window keeps Rec running, in the Dock and the
menu bar, and stops the preview. Clicking the Dock icon brings the window back. Quitting from
the Dock, the menu or a logout finalizes a running recording first.

The **menu bar item** shows a red dot and the elapsed time while recording. Its menu has
Start/Stop, Pause/Resume, Save Replay, Go Live/Stop Streaming, Show Window, Settings, Reveal
Last, Open Folder and Quit. While streaming without recording it shows LIVE.

You can also control a running recording, from the app or the CLI, from any shell:

```sh
rec-rs pause      # SIGUSR1
rec-rs resume     # SIGUSR2
rec-rs stop       # finalizes, prints the file
rec-rs save-replay  # SIGINFO: write the replay buffer to an MP4
```

Quitting the app or sending it SIGTERM mid-recording finalizes the file first.

The shortcuts are registered system-wide while Rec runs (⇧⌘R records by default; a browser's
hard reload therefore won't see it until Rec quits). If another app already owns one, Settings
says so.

## Command line

```sh
# the canonical example
rec-rs record --display 1 --mic default --system-audio --cam --out file.mp4 --duration 10

# 4K60 HEVC, screen + webcam, Ctrl-C (or `rec-rs stop`) to finish
rec-rs record -r 4k --cam

# one window at native resolution, camera-only scene, ProRes
rec-rs record --window "Safari" -r native --codec prores
rec-rs record --scene cam -o talking-head.mov
```

The default output is `~/Movies/rec-YYYY-MM-DD-HHMMSS.mov`. A live status line shows the
recorded time, frame counts and peak levels. `--duration` is exact: it is counted in output
frames, and pauses don't count.

| Option | Default | Notes |
|---|---|---|
| `-o, --out, --output` | `~/Movies/rec-….mov` | `.mov` or `.mp4`. |
| `-d, --display N` | `1` | Display number from `rec-rs devices` (1 = main). |
| `--window W` | | Window ID or app/title substring. |
| `--region X,Y,W,H` | | Crop a display region (points from top-left). |
| `-r, --resolution` | `1080p` | `720p`, `1080p`, `1440p`, `4k`, `native`, `WxH`: the largest frame with the display's (or window's) shape that fits that size, e.g. 1670x1080 for a 4112x2658 display. Never black bars, never cropped, never upscaled. |
| `-f, --fps` | `60` | 24, 25, 30, 50, 60, 120. |
| `--codec` | auto | `h264` up to 1080p, `hevc` above, `prores` (422, `.mov`). |
| `--bitrate` | auto | Mb/s. About 20 for 1080p60 H.264, about 50 for 4K60 HEVC. |
| `--system-audio` / `--no-system-audio` | on | |
| `--mic DEVICE` / `--no-mic` | `default` | Unique ID or name substring. |
| `-w, --cam, --webcam [DEVICE]` | off | `default` or ID or name. Turns on the Screen + Cam scene. |
| `--scene` | `screen-cam` with a camera, else `screen` | `screen`, `screen-cam`, `cam`, `presenter`, `side-by-side`, `cam-screen`, `split`, `reaction`, `vertical`. |
| `--scene-file FILE` | | A custom scene (JSON, as the app exports it). |
| `--canvas` | `source` | Frame shape: `source`, `16:9`, `9:16`, `1:1`, `4:5`. A preset resolution names the short side (1080p of 9:16 is 1080x1920). |
| `--background` | dark gradient | Behind layers: `RRGGBB` or `RRGGBB,RRGGBB` (top, bottom). |
| `--transition`, `--transition-ms` | `move`, `400` | Scene switch animation: `cut`, `fade`, `move`. |
| `--app APP` | | Every window of an app (bundle ID or name) on the display. |
| `--camera-resolution` | `720p` | `720p`, `1080p`, `4k` (the largest the camera supports up to it). |
| `--camera-file` | | Also write the webcam alone to `<output>-camera.mov`, same timeline. |
| `--low-cut`, `--denoise` | off | Mic: 80 Hz high-pass; RNNoise noise suppression. |
| `--gate [DB]`, `--compressor [DB]` | off (`-45`, `-20`) | Mic: noise gate threshold; compressor threshold (3:1, 6 dB makeup). |
| `--hdr` | off | HDR recording (experimental): 10-bit HEVC or ProRes, BT.2020 PQ. No replay or streams. |
| `--pip-size`, `--pip-corner`, `--pip-shape`, `--no-mirror` | `0.22`, bottom-right, rounded | Shape: `circle`, `rounded`, `square`, `pill`, `squircle` (or `rect`). |
| `--pip-pos X,Y` | | Center the overlay anywhere, as fractions of the frame from the top-left (overrides `--pip-corner`). |
| `--pip-roundness`, `--pip-border`, `--pip-border-color`, `--pip-shadow` | `0.2`, `0`, `ffffff`, off | Roundness 0–1; border width in pixels at 1080p; hex color; soft drop shadow. |
| `--audio-tracks` | `both` | `both` = mix plus system and mic stems; `mix`; `stems`. |
| `--system-gain`, `--mic-gain` | `0` | dB, in the mix track. |
| `--system-audio-offset-ms`, `--mic-offset-ms` | output latency, `0` | Sync offsets. System audio is shifted by the output device's latency by default (see Output format). |
| `--no-cursor`, `--clicks` | cursor shown | |
| `-t, --duration` | | Seconds of output. |
| `--mic-backend` | `sck` | `sck` (ScreenCaptureKit) or `avcapture`. |
| `--test-pattern [--noise]` | | Synthetic source: the frame flashes white and the "system audio" beeps once a second, for self-tests and sync checks. |
| `--replay SECS` | off | Keep the last 15–300 s of the live encode in memory; `rec-rs save-replay` writes them to an MP4 next to the output. |
| `--stream DEST` | | Stream to `youtube`, `twitch`, an `rtmp://`/`rtmps://` URL, an `srt://host:port?streamid=…&passphrase=…&latency=MS` URL or a WHIP `http(s)://` URL. Repeatable: one encode feeds every destination. |
| `--no-record` | | Write no file (replay buffer and/or streaming only). |
| `--live-resolution` | `1080p` | Live encode bounding size: the frame keeps its shape and is never above the capture. |
| `--stream-bitrate` | platform preset | Live video kb/s. By default the platform's recommendation for the size and frame rate: YouTube 2160p60 45 Mb/s H.264 or 35 HEVC (2160p30 35/25, 1440p 24/16, 1080p 12/10); Twitch at most 6 Mb/s and 1080p; others like YouTube. |
| `--stream-codec` | `auto` | `auto` (HEVC when every destination is YouTube), `h264` or `hevc` (HEVC Main10 over enhanced RTMP or WHIP H.265; a server that refuses it gets H.264). |
| `--keyframe` | `2` | Live keyframe interval in seconds (0.5–4). |
| `--vbr` | CBR | Average bitrate instead of constant. |
| `--audio-bitrate` | `192` | Live AAC/Opus kb/s. |

Other commands:

- `rec-rs devices` lists displays, windows, cameras and mics.
- `rec-rs probe FILE` checks a file with AVFoundation, QuickTime's engine, and decodes every
  sample.
- `rec-rs permissions [--request]` shows or requests the privacy grants.
- `rec-rs stop`, `rec-rs pause` and `rec-rs resume` control a running recording.
- `rec-rs save-replay` saves the replay buffer of the running recorder (CLI or app).
- `rec-rs stream-key DEST [--delete]` stores a stream key in the Keychain (read from the
  terminal with echo off). `REC_STREAM_KEY` overrides it for one run.

### Replay buffer and streaming

```sh
rec-rs stream-key youtube                         # once; key goes to the Keychain
rec-rs record --stream youtube --replay 60        # record, stream and keep a replay buffer
rec-rs record --no-record --stream rtmp://localhost/live --stream http://localhost:8889/live/whip
rec-rs record --stream 'srt://ingest.example.com:9000?streamid=live/me&passphrase=correct-horse&latency=300'
```

- **One encode.** Every composited frame goes once through a real-time VideoToolbox session,
  together with its own audio mix (AAC; Opus for WHIP). The replay buffer and every stream share
  it, and it keeps running while a recording is paused.
  - Video is H.264 High or HEVC Main10, with no B-frames, a keyframe every 2 s, and CBR or
    average bitrate. The bitstream carries BT.709 limited-range tags.
  - A live size smaller than the capture (record 4K, stream 1080p) is scaled with the same
    Lanczos scaler.
  - HEVC goes out as enhanced RTMP (`hvc1`), as YouTube accepts, or as WHIP H.265. If any
    destination can't take HEVC, the encode switches to H.264.
- **Replay buffer.** Packets are kept from a keyframe, trimmed a GOP at a time. A saved clip
  passes the video through and re-encodes the audio from the decoded AAC, so QuickTime and
  ffmpeg agree on its sync. 60 s at 6 Mb/s uses about 45 MB.
- **RTMP/RTMPS.** Handshake, AMF0 connect/publish, FLV tags, TLS for `rtmps://`. When the
  network falls behind, video is dropped a GOP at a time (queue over 3 s). Reconnects with
  backoff (1 s up to 30 s).
- **SRT.** MPEG-TS (PAT/PMT before each keyframe, PCR on the video, H.264 or HEVC and AAC)
  in 1316-byte payloads over SRT's HSv5 caller handshake, with the stream ID, latency (200 ms
  by default) and, with a passphrase, AES-CTR encryption (key derived with PBKDF2 and wrapped
  with RFC 3394, 16/24/32-byte keys via `pbkeylen`). Lost packets the receiver reports (NAK)
  are resent; packets too late to play are dropped. The Keychain key, if any, is the stream ID
  when the URL has none. Tested on Linux against FFmpeg's libsrt as the listener: all frames
  decode, also encrypted (AES-128 and AES-256), and through a relay dropping 5% of the data
  and 2% of the control packets (all recovered by retransmission). A wrong passphrase is
  reported as such.
- **WHIP.** SDP offer POSTed with the key as a Bearer token, ICE/DTLS/SRTP on one UDP socket,
  H.264 + Opus. Packets are sent at their capture time and the session starts with audio and
  video aligned, so servers that time tracks by arrival (like MediaMTX) stay in sync. The
  session is DELETEd on stop; a server silent for 5 s triggers a reconnect.
- **Tested** against a local MediaMTX (`brew install mediamtx`), RTMP, RTMPS and WHIP at
  once from one encode: A/V offset in the server's own recordings −0.5 ms (RTMP) and within
  −13 ms (WHIP, of which 6.5 ms is the Opus pre-skip MediaMTX writes into its MP4). Restarting
  the server reconnects within a few seconds. `scripts/selftest.sh` runs these checks when
  `mediamtx` is installed. YouTube and Twitch use the same RTMPS path but were not streamed
  to in testing.

## Output format

| Track | Contents |
|---|---|
| 1 video | H.264 High, HEVC Main (`hvc1`) or ProRes 422. Constant frame rate, BT.709 tagged, keyframe every 2 s. |
| 2 audio | **Mix** of system and mic: AAC-LC 48 kHz stereo, 256 kb/s. The default track, so players and uploaders use it. |
| 3 audio | System audio stem: AAC 48 kHz stereo, 256 kb/s. |
| 4 audio | Mic stem: AAC 48 kHz mono, 192 kb/s. |

- **Track grouping.** With one audio source there is a single audio track. The audio tracks
  share an alternate group with the mix as default, so QuickTime plays the mix and editors see
  every track.
- **Constant frame rate.** ScreenCaptureKit only sends a frame when pixels change. rec-rs
  writes on an exact 1/fps grid and repeats the newest frame when needed. Measured frame spacing
  is exactly 16.667 ms.
- **Single clock.** Screen, system audio, mic and webcam are all stamped on the host clock, and
  audio is never stretched. The flash/beep sync test measures 0.0 ms between video and audio in
  the file, including across pauses.
- **System audio latency.** ScreenCaptureKit captures system audio when it is mixed, which is
  one output latency before you hear it. rec-rs shifts it later by the output device's latency,
  read from CoreAudio (about 27 ms on built-in speakers, more on Bluetooth), so the audio lines
  up with the picture you saw. It is re-read every second while capturing, so switching to
  AirPods mid-recording keeps the sync. `rec-rs devices` prints the latency.
- **Webcam sync.** Emission runs a little behind real time, so the webcam frame captured at each
  instant is the one used. Webcams deliver frames 70–120 ms late, and the lag adapts to the
  slowest source.
- **Crash-safe.** Movie fragments are written every 2 s, so a crash or power loss leaves a
  playable file up to the last fragment. A normal stop finalizes a regular movie
  (`ftyp wide mdat moov`).
- **Edge to edge.** The frame has the source's aspect ratio, so the picture fills it with no
  letterbox or pillarbox bars. A single window's transparent rounded corners are filled with
  the window's own edge color instead of encoding as black.
- **Camera.** In the app the running camera passes from the preview to the recording and
  back, so a recording starts with the exposure already settled. A camera started cold (CLI)
  is given 2 s to settle before the file starts, so it never opens on black or dark frames.
- **Audio quality.** System audio and the mic are captured at 48 kHz; a device at another rate
  (44.1 kHz USB, 16/24 kHz Bluetooth) is resampled with a windowed-sinc filter that is
  continuous across buffers. The mix runs through a limiter that keeps it under −1 dBFS
  instead of clipping. Measured with a sweep, tones and noise played as system audio: flat
  within 0.1 dB from 50 Hz to 20 kHz, no clicks.
- **Pause.** Pauses cut whole frame slots out of the timeline. Audio is cut at the exact sample
  and shifted with the video, so there is no gap and no drift.

## Feature comparison with OBS (macOS)

| Feature | OBS 32 | rec-rs |
|---|---|---|
| Display capture | ✅ | ✅ ScreenCaptureKit, zero-copy |
| Window capture | ✅ | ✅ |
| Region / crop | ✅ crop filter | ✅ drag over the preview, or `--region` |
| Application capture (all windows of an app) | ✅ | ✅ |
| Scenes | ✅ unlimited, free layout, transitions | ✅ nine templates plus custom scenes (free layout, image, text and color layers), live switching with Move, Fade or Cut transitions |
| Output frame shape | ✅ any canvas | ✅ the source's shape, 16:9, 9:16, 1:1, 4:5 |
| Multiple displays/sources at once | ✅ | ✅ other displays as layers in custom scenes, plus the webcam |
| System audio | ✅ | ✅ |
| Microphone | ✅ | ✅ |
| Webcam overlay | ✅ any layout | ✅ drag anywhere with corner snapping; circle, rounded, square, pill or squircle; roundness, border, shadow, mirror; live while recording |
| Separate audio tracks | ✅ up to 6 | ✅ mix plus system and mic stems |
| Per-source gain | ✅ | ✅ |
| Audio filters (noise suppression, compressor) | ✅ | ✅ mic: RNNoise noise suppression, gate, compressor, low cut (live) |
| Hardware H.264 / HEVC | ✅ | ✅ |
| ProRes | ✅ | ✅ 422 |
| 10-bit / HDR | ✅ | ⚠️ experimental: HEVC Main10 or ProRes, BT.2020 PQ, recording only |
| Constant frame rate | ✅ | ✅ exact grid |
| Crash-safe files | ✅ hybrid/fragmented MP4 | ✅ fragments every 2 s |
| Pause / resume | ✅ | ✅ sample-accurate, app and CLI; the pause is cut from the file, audio stays in sync |
| Replay buffer | ✅ | ✅ 15–300 s in memory, ⇧⌘S or `save-replay` writes an MP4 without stopping, memory use shown |
| Streaming | ✅ RTMP, WHIP, SRT, multitrack | ✅ RTMP/RTMPS (YouTube, Twitch, custom), SRT (with encryption) and WHIP, several at once from one encode (app and CLI), H.264 or HEVC Main10 (enhanced RTMP), platform bitrate presets up to 4K60, auto-reconnect, live status; key in the Keychain |
| Screen scaling | GPU bilinear/bicubic/Lanczos | ✅ native capture, Metal Lanczos per plane (5K→4K VMAF 100 vs 90 for ScreenCaptureKit) |
| Live preview | ✅ | ✅ |
| Level meters | ✅ | ✅ peak with hold |
| Global hotkey | ✅ configurable | ✅ configurable: record, pause, save replay, go live, next/previous scene |
| Separate camera file | ✅ (source record plugin) | ✅ `…-camera.mov` on the recording's timeline |
| Menu bar control | ❌ | ✅ menu bar item plus Dock app |
| Headless CLI | ❌ (obs-cli via websocket) | ✅ `rec-rs record …`, `stop`, `pause`, `resume` |
| Footprint | about 300 MB app | about 22 MB app; about 6% of one core at 4K60 screen-only, about 34% with the webcam overlay (see [PERFORMANCE.md](PERFORMANCE.md)) |
| Encoder input | frames copied back to the CPU for encoding | IOSurface passed straight to VideoToolbox |

## Known gaps

- **Scenes.** Layers are the screen, the camera, other displays, images, text and colors; there
  are no browser, media-file or game sources, no per-layer filters (chroma key, color
  correction), and only one camera. A recording can switch live only to scenes whose sources
  it started with (turn the camera on to switch into camera scenes).
- **Region capture** applies to one display; other displays in a scene are always whole.
- **Streaming.** No multitrack (simulcast renditions) and no SRT listener or rendezvous mode
  (caller only). Changing the live encode settings needs the stream stopped. HEVC to YouTube
  was tested only against a local server, not YouTube itself.
- **HDR is experimental.** It records HDR10 (BT.2020 PQ) from ScreenCaptureKit's HDR capture,
  but the replay buffer and streams stay SDR and are off meanwhile, a window's transparent
  corners aren't filled, and the preview isn't tone-mapped.
- **Noise suppression** needs a 48 kHz mic (ScreenCaptureKit captures at 48 kHz; other rates
  get the other filters only). It delays the mic by 20 ms, which the timestamps make up for.
- **Re-confirmation prompt.** macOS periodically asks you to re-confirm screen recording for
  apps that bypass the system picker. This happens to every recorder.

## Layout

| Path | Contents |
|---|---|
| `src/main.rs` | CLI |
| `src/app/` | App: windows and UI (`ui.rs`), scene gallery and layer editor (`scenes.rs`), menu bar item and app menu, Dock quit/reopen hooks, preview scaler, theme, macOS chrome |
| `src/settings.rs` | Persisted settings → record arguments |
| `src/hotkey.rs` | Global shortcuts (Carbon `RegisterEventHotKey`; needs no Accessibility permission) and their key names |
| `src/record.rs` | Sessions, pacing thread, signals, `stop`/`pause`/`resume` |
| `src/writer.rs` | AVAssetWriter sink, constant-frame-rate grid, pause timeline, track layout |
| `src/screen.rs` | ScreenCaptureKit capture, displays and windows |
| `src/camera.rs` | Webcam and AVCapture mic, device lists |
| `src/layout.rs` | Scene templates, custom layouts, frame shapes and transitions (plain geometry) |
| `src/compose.rs` | CoreImage compositor: layers, outlines, gradients, images, text, transitions |
| `src/filters.rs` | Mic filters: low cut, RNNoise noise suppression, gate, compressor |
| `src/mixer.rs` | Real-time audio mixer |
| `src/live/` | Shared live encode (VideoToolbox H.264, AAC/Opus), replay buffer, RTMP/RTMPS, SRT (with its MPEG-TS muxer) and WHIP outputs, Keychain |
| `src/tcc.rs` | Permissions and responsibility disclaim |
| `src/probe.rs` | AVFoundation verification |
| `src/synth.rs` | Test pattern with flash + beep |
| `src/scale.rs` | Native-resolution screen scaling (Metal Performance Shaders Lanczos, VideoToolbox fallback) |
| `portable/` | The platform-independent modules, built from `src/` for tests on any OS, and the SRT end-to-end tools |
| `scripts/` | Build, self-test, SRT end to end (`srt-e2e.sh`), soak, sync checks (`synccheck.py` with ffmpeg, `avsync.swift` with AVFoundation) and quality metrics (`quality.sh`, `encquality.sh`, `testcard.swift`) |
| `analysis/OBS_ARCHITECTURE.md` | OBS study |
