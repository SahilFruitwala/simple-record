# Changelog

## 0.1.0 (unreleased)

First release: screen, system audio, microphone and webcam recording to `.mov`/`.mp4`; scenes
with live transitions; replay buffer; RTMP, SRT and WHIP streaming; new logo; macOS DMG
packaging and a tag-triggered release workflow. macOS 15+ on Apple Silicon only.

- Permissions: a camera or microphone that isn't allowed no longer stops the preview or a
  recording. SimpleRecord asks the first time it needs one, and otherwise records without it
  and links to that permission's page in System Settings from its card. A dismissed Screen
  Recording error stays dismissed, and the banner has a Restart button.
- Permissions left over from another build no longer get stuck: **Ask Again** forgets the earlier
  decision and shows the macOS prompt, the welcome screen returns at launch while Screen Recording
  is missing, a refusal from ScreenCaptureKit shows the permission banner, and local builds are
  **SimpleRecord Dev** (`com.heyfabrika.rec.dev`) so they stop colliding with the installed app.
- Recording quality, after comparing with Cap: recordings default to the screen's native
  size (new installs); audio clock drift is eased out instead of jumped over, which removes
  periodic clicks; new Auto level filter (`--level`) holds speech near -16 LUFS with a
  look-ahead limiter; noise suppression keeps 10% of the original voice; new installs start
  with low cut, noise suppression and Auto level on; Bluetooth mics show a warning; smart zoom
  scales up with Lanczos.
- More mic control: the low cut's frequency (40-300 Hz, `--low-cut-hz`), a voice EQ with
  warmth, presence and air bands (`--eq LOW,PRESENCE,AIR`) and a split-band de-esser
  (`--deess`). All live, in Settings > Audio and the Voice filters card.
- Recording quality preset: Standard, High or Max for the automatic bitrate (`--quality`).
- Floating recording controls while recording: stop and timer, mic mute, pause, restart and
  delete (to the Trash), open Rec. Never captured. Settings > Video turns them off.
- Image backgrounds: a picture or the desktop wallpaper fills the frame behind scenes, with an
  optional blur (`--background-image`, `--background-blur`).
- GIF export: Export GIF… in the app and `simple-record gif`, for the whole recording or a part.
