# Platform support

**Today Rec runs on macOS 15+ on Apple Silicon only.** Windows and Linux builds are not
possible yet. This is an assessment of why, and what a port would take.

## Why it doesn't build elsewhere

29 of the 45 source files use Apple frameworks directly. The platform-specific layers are the
core of the app, not a thin shell:

| Layer | macOS today | Windows | Linux |
|---|---|---|---|
| Screen and window capture | ScreenCaptureKit | Windows.Graphics.Capture | PipeWire via xdg-desktop-portal (Wayland), X11 fallback |
| System audio | ScreenCaptureKit | WASAPI loopback | PipeWire monitor sources |
| Microphone, webcam | AVCaptureSession | WASAPI, Media Foundation | PipeWire/ALSA, V4L2 |
| Compositing scenes | CoreImage on Metal | wgpu compute or render passes | the same wgpu code |
| Video encode | VideoToolbox, AVAssetWriter | Media Foundation, NVENC/AMF/QSV | VA-API, NVENC, or x264 |
| Audio encode, MP4/MOV mux | AudioToolbox, AVAssetWriter | Media Foundation | a Rust muxer (e.g. `mp4`) or FFmpeg |
| Web page layers | WKWebView | WebView2 | WebKitGTK |
| Credential storage | Keychain | Credential Manager | Secret Service |
| Global hotkey, menu bar | Carbon/AppKit | `RegisterHotKey`, tray icon | portal GlobalShortcuts, tray |
| WHIP crypto | Apple CryptoKit | `str0m` with `openssl` or `rust-crypto` | same |

What is already portable (and tested on Linux in CI, see `portable/`): scene layouts and
shapes, mic filters (including RNNoise), shortcuts, the MPEG-TS muxer and SRT. The UI is egui
on wgpu, which runs on all three platforms, apart from `src/app/macos.rs` (titlebar, Dock).

## What a port takes

1. **Split `src/` into a platform-neutral core and backends** behind traits (`ScreenSource`,
   `CameraSource`, `AudioSource`, `Compositor`, `Encoder`, `Sink`). Today those are concrete
   Objective-C types passed through the whole pipeline.
2. **Rewrite the compositor on wgpu.** It is the largest piece: `compose.rs` (870 lines) is
   written against CoreImage and would be reimplemented as shaders for layers, shapes,
   borders, gradients, text and transitions. This also gives macOS the same code path.
3. **One backend per OS** for capture, audio and encode, from the table above.
4. **Replace the AVAssetWriter sink** with a muxer that works without Apple frameworks.
5. Platform packaging: `.msi`/`.exe` (WiX or Inno Setup, signed), `.AppImage`/`.deb`/Flatpak.
   Icons for these are ready: `support/icon.ico`, `support/icon.png`.

This is a multi-month effort and a rewrite of the capture and encode paths, not a
configuration change; it should be planned as its own project. A reasonable order is
Windows first (largest user base, the capture APIs are mature), then Linux.

## CI probe

`.github/workflows/ci.yml` builds and tests the portable crate on Windows as well, as an
informational (non-blocking) job, so any drift that would make the port harder shows up early.
