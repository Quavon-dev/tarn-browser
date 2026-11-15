//! The YouTube page-world script: removes ad fields from YouTube's player data
//! before the player reads them. The logic is compiled in; the signed bundle may only change which
//! keys it removes, within the limits `parse` enforces.

use serde::{Deserialize, Serialize};

const SOURCE: &str = include_str!("youtube.js");
const MAX_KEYS: usize = 32;
/// Fields playback depends on; a config naming one is refused, so a bad bundle cannot break video.
const KEEP: &[&str] = &[
    "streamingData",
    "playabilityStatus",
    "playbackTracking",
    "videoDetails",
    "microformat",
    "captions",
    "storyboards",
    "responseContext",
    "playerConfig",
    "heartbeatParams",
    "contents",
    "entries",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub keys: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { keys: ["adPlacements", "adSlots", "playerAds", "adBreakHeartbeatParams"].map(String::from).to_vec() }
    }
}

pub fn parse(bytes: &[u8]) -> Option<Config> {
    let config: Config = serde_json::from_slice(bytes).ok()?;
    let valid = |k: &String| (1..=40).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphabetic()) && !KEEP.contains(&k.as_str());
    (!config.keys.is_empty() && config.keys.len() <= MAX_KEYS && config.keys.iter().all(valid)).then_some(config)
}

/// Keys are ASCII letters only (`parse`), so their JSON form is a safe JavaScript literal.
pub fn script(config: &Config) -> String {
    SOURCE.replace("__TARN_KEYS__", &serde_json::to_string(&config.keys).unwrap_or_else(|_| "[]".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_safe_configs() {
        assert_eq!(parse(br#"{"keys":["adPlacements","adSlots"]}"#).unwrap().keys, ["adPlacements", "adSlots"]);
        for bad in [
            &br#"{"keys":[]}"#[..],
            br#"{"keys":["streamingData"]}"#,
            br#"{"keys":["ad-slots"]}"#,
            br#"{"keys":["a\"];alert(1);//"]}"#,
            br#"{"keys":["adSlots"],"paths":["/x"]}"#,
            b"not json",
        ] {
            assert_eq!(parse(bad), None, "{}", String::from_utf8_lossy(bad));
        }
        let many = format!(r#"{{"keys":[{}]}}"#, vec!["\"adSlots\""; MAX_KEYS + 1].join(","));
        assert_eq!(parse(many.as_bytes()), None);
    }

    #[test]
    fn script_embeds_keys_and_nothing_else() {
        let script = script(&Config::default());
        assert!(script.contains(r#"const KEYS = ["adPlacements","adSlots","playerAds","adBreakHeartbeatParams"];"#));
        assert!(!script.contains("__TARN_"));
        for banned in ["fetch(", "XMLHttpRequest(", "localStorage", "document.cookie", "eval(", "Function(", "postMessage", "webkit.messageHandlers"] {
            assert!(!script.contains(banned), "{banned}");
        }
    }
}
