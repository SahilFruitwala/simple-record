//! ZeroRec: Zero-copy, high-performance screen recording and live streaming engine for macOS.
//!
//! Features:
//! - Zero-copy ScreenCaptureKit screen capture
//! - Metal Performance Shaders Lanczos scaling
//! - CoreImage on Metal scene compositing
//! - Direct VideoToolbox hardware encoding (H.264, HEVC, ProRes)
//! - Live streaming over RTMP, SRT, and WHIP (WebRTC)
//! - Sample-accurate 48 kHz audio mixer with RNNoise DSP

pub mod appaudio;
pub mod announce;
pub mod brand;
pub mod browser;
pub mod camera;
pub mod cli;
pub mod compose;
pub mod disk;
pub mod doctor;
pub mod effects;
pub mod filters;
pub mod gif;
pub mod hotkey;
pub mod layout;
pub mod live;
pub mod logging;
pub mod media;
pub mod mixer;
pub mod monitor;
pub mod outlatency;
pub mod play;
pub mod probe;
pub mod record;
pub mod scale;
pub mod screen;
pub mod settings;
pub mod shapes;
pub mod shortcut;
pub mod synth;
pub mod tcc;
pub mod util;
pub mod writer;
pub mod zoom;

pub use appaudio::AppAudio;
pub use compose::{Corner, PipSpec, Scene, Shape};
pub use live::{Live, LiveConfig};
pub use record::{Options, Session, check_permissions, drop_unallowed, run, stop_running};
pub use screen::{AppInfo, DisplayInfo, Target, WindowInfo, applications, displays, windows};
pub use writer::{Codec, Quality, PreviewFn, Stats};
pub use cli::{Cli, Cmd, RecordArgs, parse_record_args, prepare, prepare_lenient};
