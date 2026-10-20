# Cap Studio Mode: what Rec has and what it would take

Source: <https://cap.so/features/studio-mode> (read 2026-10-20). Cap lists eight Studio Mode
features. This note compares each one with Rec as it is today and estimates the work for the
missing ones. Nothing here is implemented.

## Summary

| Cap feature | Rec today | Gap | Effort |
|---|---|---|---|
| 4K at 60 fps | Yes: 4K60 (and 120 fps), native 5K capture, Lanczos scaling, H.264/HEVC/ProRes | None | — |
| Local recording and processing | Yes: everything runs on the Mac; nothing is uploaded unless you stream | None | — |
| Precision timeline editor | No: there is no editor | Trim, cut and export of a take | Medium (trim/cut), large (full editor) |
| Multiple camera angles, layout changed after recording | Partly: scenes and layouts switch live, and `--camera-file` saves the camera alone on the same timeline | The screen is saved already composited, so the layout can't be changed after recording | Large |
| Custom backgrounds and branding | Mostly: gradient backgrounds (8 presets plus custom colors), **image or wallpaper backgrounds with blur (done)**, screen inset with rounded corners and shadow, image, text and color layers, 5 frame shapes | Saved brand presets (logo, colors, name tag) | Small |
| Separate audio levels, noise reduction | Yes, and more: per-source gain, RNNoise, gate, compressor, low cut, stems on separate tracks, per-app audio | None | — |
| Smart auto-zoom | No | Zoom in on the cursor or clicks, live or afterwards | Medium (live), large (in an editor) |
| Export to MP4, GIF, shareable link | MP4, MOV and **GIF (done)** | Shareable links | Links need a hosting service |

## Feature by feature

### 1. Precision timeline editor (missing)

The biggest gap. Two steps make sense:

- **Trim and cut (medium).** Open a finished take in a window with a filmstrip and waveform,
  mark in/out points and cuts, and export with `AVMutableComposition` plus
  `AVAssetExportSession` in passthrough mode. Nothing is re-encoded, so export is instant and
  lossless, and the files are already `AVFoundation`-friendly (`rec-rs probe` checks this). The
  separate audio tracks are kept. Cuts land on keyframes in passthrough (every 2 s), so
  frame-exact cuts need a re-encode of the GOPs around them.
- **Full editor (large).** Several tracks, zoom segments, layout changes and captions. This
  needs the separate-sources recording in point 2, a timeline UI in egui, and a renderer that
  runs `compose.rs` offline through VideoToolbox.

### 2. Change the layout after recording (partly there)

Today the compositor bakes the scene into the file. To change it later, Rec would also have
to save each source raw: the screen at native size, each camera (already possible for one
camera), and cursor and click positions with timestamps. It would also save the scene timeline:
which scene and layer boxes were used, and when. The editor then re-renders through the same
`compose.rs`, so what you see live and what you export look the same. Costs: about twice the
disk space and encoder load while recording, so it would be an option, not the default.

### 3. Backgrounds and branding (mostly there)

Missing pieces, each small:

- An image or wallpaper as the background (the compositor already draws image layers, so
  this is a `Background` variant).
- Brand presets: a saved set (logo layer, colors, name tag, frame shape) applied to any
  scene. Custom scenes and JSON export already cover much of this.

### 4. Smart auto-zoom (missing)

- **Live (medium).** ScreenCaptureKit draws the cursor into the frames, so Rec doesn't know
  where it is. It would read the cursor position (`NSEvent.mouseLocation`) and clicks (an
  event tap, which needs the Accessibility permission, or a passive `NSEvent` global monitor
  for clicks), then animate the screen layer's crop in `compose.rs` toward the click with
  easing, and ease back out after a pause. The screen is captured at native resolution, so a
  2x zoom stays sharp at 1080p. It would work in recordings and streams alike.
- **Afterwards (large).** Needs the editor and the raw screen recording from point 2. Zoom
  segments would be suggested from the saved clicks and adjusted on the timeline.
- Related polish from the same data: a smoothed or enlarged cursor drawn by Rec instead of
  ScreenCaptureKit, and hiding the cursor when it is idle.

### 5. Export formats

- **GIF (small).** Decode the take with `AVAssetReader`, downscale, and write with ImageIO's
  `CGImageDestination` (frame delays, loop count). For short clips; long ones are huge.
- **Shareable links (out of scope for now).** These need a server to upload to. A middle
  ground is "Upload to…" a service the user picks (YouTube upload API, an S3 bucket), which
  keeps Rec local-first.

## Not compared

Cap's Instant Mode (cloud links, transcription, comments, team sharing) is a cloud product and
not a Studio Mode feature. Rec has things Cap's page doesn't list: live streaming to several
destinations, a replay buffer, studio mode with transitions, chroma key, web page and video
layers, and global shortcuts.

## Suggested order

1. ~~GIF export and image backgrounds~~ (done: Export GIF…, `rec-rs gif`, image and
   desktop-picture backgrounds with blur).
2. Live auto-zoom on clicks (the most "Cap-like" feature, and it also helps streams).
3. Trim and cut editor with passthrough export.
4. Raw source recording, then the full editor with layout changes and zoom afterwards.
