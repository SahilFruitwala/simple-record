//! Which apps' sound "system audio" records: every app (the default), only
//! some, or all but some. Apps are named by bundle ID; an app's helper
//! processes (`com.google.Chrome.helper` for `com.google.Chrome`), which
//! often play a browser's sound, count as the app.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "mode", content = "apps")]
pub enum AppAudio {
    #[default]
    All,
    Only(Vec<String>),
    Except(Vec<String>),
}

/// The running apps a capture filter names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// No filter: every app but Rec.
    Everything,
    /// Only these (indices into the running apps).
    Include(Vec<usize>),
    /// All but these (Rec included).
    Exclude(Vec<usize>),
}

/// `id` is `app` or one of its helpers.
fn belongs(id: &str, app: &str) -> bool {
    id == app || id.strip_prefix(app).is_some_and(|rest| rest.starts_with('.'))
}

impl AppAudio {
    pub fn is_all(&self) -> bool {
        match self {
            AppAudio::All => true,
            // Excluding nothing is everything.
            AppAudio::Except(v) => v.is_empty(),
            AppAudio::Only(_) => false,
        }
    }

    /// The apps listed (none for `All`).
    pub fn apps(&self) -> &[String] {
        match self {
            AppAudio::All => &[],
            AppAudio::Only(v) | AppAudio::Except(v) => v,
        }
    }

    pub fn apps_mut(&mut self) -> Option<&mut Vec<String>> {
        match self {
            AppAudio::All => None,
            AppAudio::Only(v) | AppAudio::Except(v) => Some(v),
        }
    }

    /// The filter for these running apps (bundle IDs); `own` is Rec's.
    pub fn pick(&self, running: &[&str], own: Option<usize>) -> Pick {
        let named = |list: &[String]| -> Vec<usize> { (0..running.len()).filter(|&i| Some(i) != own && list.iter().any(|a| belongs(running[i], a))).collect() };
        match self {
            _ if self.is_all() => Pick::Everything,
            AppAudio::Only(list) => Pick::Include(named(list)),
            AppAudio::Except(list) => {
                let mut v = named(list);
                v.extend(own);
                v.sort();
                Pick::Exclude(v)
            }
            AppAudio::All => unreachable!(),
        }
    }

    /// For the command line: `Only` from `--audio-apps`, `Except` from
    /// `--audio-except` (comma-separated bundle IDs).
    pub fn from_args(only: Option<&str>, except: Option<&str>) -> AppAudio {
        let list = |s: &str| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        match (only, except) {
            (Some(o), _) => AppAudio::Only(list(o)),
            (None, Some(e)) => AppAudio::Except(list(e)),
            (None, None) => AppAudio::All,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: [&str; 5] = ["com.apple.Music", "com.google.Chrome", "com.google.Chrome.helper", "com.google.ChromeBeta", "rs.rec"];

    #[test]
    fn picks() {
        let own = Some(4);
        assert_eq!(AppAudio::All.pick(&RUNNING, own), Pick::Everything);
        assert_eq!(AppAudio::Except(vec![]).pick(&RUNNING, own), Pick::Everything);
        // Chrome and its helper, not another app sharing the prefix.
        assert_eq!(AppAudio::Only(vec!["com.google.Chrome".into()]).pick(&RUNNING, own), Pick::Include(vec![1, 2]));
        assert_eq!(AppAudio::Except(vec!["com.apple.Music".into()]).pick(&RUNNING, own), Pick::Exclude(vec![0, 4]));
        // Rec itself is never included.
        assert_eq!(AppAudio::Only(vec!["rs.rec".into()]).pick(&RUNNING, own), Pick::Include(vec![]));
        assert_eq!(AppAudio::Only(vec!["com.not.running".into()]).pick(&RUNNING, own), Pick::Include(vec![]));
    }

    #[test]
    fn args_and_settings() {
        assert_eq!(AppAudio::from_args(Some("a.b, c.d,"), None), AppAudio::Only(vec!["a.b".into(), "c.d".into()]));
        assert_eq!(AppAudio::from_args(None, Some("x.y")), AppAudio::Except(vec!["x.y".into()]));
        assert_eq!(AppAudio::from_args(None, None), AppAudio::All);
        let json = serde_json::to_string(&AppAudio::Only(vec!["a.b".into()])).unwrap();
        assert_eq!(json, r#"{"mode":"only","apps":["a.b"]}"#);
        assert_eq!(serde_json::from_str::<AppAudio>(r#"{"mode":"all"}"#).unwrap(), AppAudio::All);
    }
}
