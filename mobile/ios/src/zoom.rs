pub use engine::view_policy::zoom_factor as clamp;

pub fn is_actual_size(factor: f64) -> bool {
    percent(factor) == 100
}

fn percent(factor: f64) -> u32 {
    (clamp(factor) * 100.0).round() as u32
}

pub fn script(factor: f64) -> String {
    let percent = percent(factor);
    format!(
        "(() => {{ const later = () => setTimeout(zoom, 300); const zoom = () => {{ const root = document.documentElement; if (!root) return; \
         if (!root.clientWidth) {{ addEventListener('resize', later, {{ once: true }}); return; }} \
         const style = root.style; \
         const MAX_BOOST = 4; if (globalThis.tarnBoost === undefined) {{ \
           const sample = [...document.querySelectorAll('p')].sort((a, b) => b.textContent.length - a.textContent.length)[0] || document.body; \
           if (!sample) return; \
           const size = () => parseFloat(getComputedStyle(sample).fontSize) || 1; \
           const autosized = size(); \
           style.setProperty('-webkit-text-size-adjust', 'none', 'important'); \
           globalThis.tarnBoost = Math.min(MAX_BOOST, Math.max(1, autosized / size())); \
         }} \
         style.removeProperty('-webkit-text-size-adjust'); \
         if ({percent} === 100) return; \
         style.setProperty('-webkit-text-size-adjust', Math.round({percent} * globalThis.tarnBoost) + '%', 'important'); }}; \
         if (globalThis.tarnZoom) removeEventListener('resize', globalThis.tarnZoom); \
         globalThis.tarnZoom = later; zoom(); }})()"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_scales_text_on_top_of_webkit_autosizing() {
        let script = script(1.5);
        assert!(script.contains("if (globalThis.tarnBoost === undefined)"));
        assert!(script.contains("globalThis.tarnBoost = Math.min(MAX_BOOST, Math.max(1, autosized / size()));"));
        assert!(script.contains("const MAX_BOOST = 4;"), "a page cannot inflate the boost without limit");
        assert!(script.contains("removeProperty('-webkit-text-size-adjust'); if (150 === 100) return;"));
        assert!(script.contains("Math.round(150 * globalThis.tarnBoost) + '%'"));
        assert!(script.contains("if (!root.clientWidth) { addEventListener('resize', later, { once: true }); return; }"));
    }

    #[test]
    fn steps_round_to_whole_percent() {
        assert_eq!(script(1.1 + 0.1), script(1.2));
        assert!(script(0.9).contains("Math.round(90 * globalThis.tarnBoost)"));
    }

    #[test]
    fn actual_size_restores_the_page_default() {
        assert!(is_actual_size(1.0));
        assert!(is_actual_size(0.999));
        assert!(script(1.0).contains("if (100 === 100) return;"));
    }

    #[test]
    fn factors_are_clamped() {
        assert!(script(10.0).contains("Math.round(300 * globalThis.tarnBoost)"));
        assert!(script(0.1).contains("Math.round(50 * globalThis.tarnBoost)"));
        assert!(is_actual_size(f64::NAN));
    }
}
