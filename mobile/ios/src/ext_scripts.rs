//! The pieces of Tarn extensions only WKWebView on iOS needs: its message handler names and the
//! setup for `engine::tarn_ext::injections`. WebKit gives iOS no frame id, so each document of a
//! world draws a random token, says it first, and starts every message with it.

use engine::handover::js_string;

pub const HANDLER: &str = "tarnExt";
pub const MESSAGE: &str = "extensionMessage";
pub const POST: &str = "extensionPost";
pub const FRAME: &str = "extensionFrame";
/// Hex digits of the per-document token a frame's messages start with.
pub const TOKEN: usize = engine::tarn_ext::DOCUMENT_LEN;
pub const SETUP: &str = "const h = globalThis.webkit && webkit.messageHandlers && webkit.messageHandlers.tarnExt; if (!h) return; \
    const b = new Uint8Array(16); crypto.getRandomValues(b); const token = Array.from(b, x => x.toString(16).padStart(2, '0')).join(''); \
    globalThis.__tarnExtDoc = token; h.postMessage(token); globalThis.__tarnExtPost = s => h.postMessage(token + String(s));";

/// The transport and bootstrap of an install's own page, in its main world and main frame only.
pub fn page_script(origin_id: &str, bootstrap: &str) -> String {
    format!(
        "(() => {{ if (window.top !== window || location.protocol !== 'tarn-extension:' || location.host !== {}) return; \
        const h = webkit.messageHandlers.{HANDLER}; globalThis.__tarnExtPost = s => h.postMessage(String(s));\n{bootstrap}\n}})();",
        js_string(origin_id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::tarn_ext::{Decisions, FrameNav, RunAt, Script, World, WorldScripts, injections};
    use url::Url;

    #[test]
    fn the_shared_gate_carries_the_token_transport() {
        let world = WorldScripts {
            world: World::for_install("ab12").unwrap(),
            bootstrap: "BOOT();".into(),
            scripts: vec![Script { run_at: RunAt::DocumentStart, all_frames: false, js: vec!["'use strict';\nA();".into()], css: vec![] }],
        };
        let url = Url::parse("https://shop.test/p#x").unwrap();
        let mut decisions = Decisions::default();
        decisions.record(&FrameNav { url: &url, top: &url, main_frame: true }, vec![0]);
        let scripts = injections(&world, &decisions, SETUP);
        assert!(scripts[0].source.contains(r#"["m https://shop.test/p",[0]]"#) && scripts[0].source.contains("h.postMessage(token)"));
        assert!(scripts[0].source.find("if (!run) return;") < scripts[0].source.find("const h = globalThis.webkit"));
        assert!(scripts[1].source.starts_with("'use strict';if (!(globalThis.__tarnExtRun || []).includes(0)) throw"), "{}", scripts[1].source);
    }

    #[test]
    fn a_subframe_document_keeps_the_token_its_replies_must_match() {
        assert!(SETUP.contains(&format!("globalThis.{} = token; h.postMessage(token);", engine::tarn_ext::DOCUMENT)));
        assert!(SETUP.find("crypto.getRandomValues(b)") < SETUP.find("globalThis.__tarnExtDoc = token"));
    }

    #[test]
    fn pages_get_the_transport_only_on_their_own_origin() {
        let script = page_script("abc", "BOOT();");
        assert!(script.contains("location.host !== \"abc\"") && script.contains("window.top !== window") && script.ends_with("BOOT();\n})();"));
        assert_eq!(
            engine::tarn_ext::reply(None, "{\"a\":\"</script>\"}"),
            format!("if (typeof globalThis.__tarnExtReceive === 'function') globalThis.__tarnExtReceive({});", js_string("{\"a\":\"</script>\"}"))
        );
    }
}
