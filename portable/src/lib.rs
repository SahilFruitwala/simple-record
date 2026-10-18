//! The app's platform-independent modules, as they are in `src/`.
#![allow(dead_code)]

#[path = "../../src/filters.rs"]
pub mod filters;
#[path = "../../src/layout.rs"]
pub mod layout;
#[path = "../../src/logging.rs"]
pub mod logging;
#[path = "../../src/shapes.rs"]
pub mod shapes;
#[path = "../../src/shortcut.rs"]
pub mod shortcut;
#[path = "../../src/live/srt.rs"]
pub mod srt;
#[path = "../../src/live/ts.rs"]
pub mod ts;
