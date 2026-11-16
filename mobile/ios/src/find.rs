/// Clears the page's selection, from which WebKit's find continues.
pub const RESTART: &str = "getSelection().removeAllRanges()";

/// Counts the page's matches within the shared limits; a longer query counts none, and the
/// engine's own find still decides whether there is one.
pub fn count_script(text: &str) -> String {
    use engine::page_scripts::{FIND_COUNT_MAX, FIND_QUERY_MAX, FIND_TEXT_MAX};
    let text = if text.chars().count() > FIND_QUERY_MAX { "" } else { text };
    let query = serde_json::to_string(&text.to_lowercase()).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        "(() => {{ const query = {query}; if (!query) return 0; \
         const text = (document.body ? document.body.innerText : '').slice(0, {FIND_TEXT_MAX}).toLowerCase(); \
         let count = 0; for (let at = text.indexOf(query); at !== -1 && count < {FIND_COUNT_MAX}; at = text.indexOf(query, at + query.length)) count++; \
         return count; }})()"
    )
}

pub fn matched(found: bool, counted: Option<f64>) -> u64 {
    if !found {
        return 0;
    }
    counted.filter(|n| n.is_finite() && *n >= 1.0).map_or(1, |n| n.min(engine::page_scripts::FIND_COUNT_MAX as f64) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_is_a_json_string_literal() {
        let script = count_script("Fox'); alert(1); ('");
        assert!(script.contains(r#"const query = "fox'); alert(1); ('";"#), "{script}");
        assert!(count_script("a\"b\\c\n").contains(r#"const query = "a\"b\\c\n";"#));
    }

    #[test]
    fn webkit_decides_whether_anything_matched_and_the_page_how_often() {
        assert_eq!(matched(false, Some(3.0)), 0);
        assert_eq!(matched(true, Some(3.0)), 3);
        assert_eq!(matched(true, Some(0.0)), 1);
        assert_eq!(matched(true, None), 1);
        assert_eq!(matched(true, Some(f64::NAN)), 1);
        assert_eq!(matched(true, Some(1e9)), engine::page_scripts::FIND_COUNT_MAX as u64);
    }

    #[test]
    fn a_query_past_the_limit_counts_nothing() {
        let long = "a".repeat(engine::page_scripts::FIND_QUERY_MAX + 1);
        assert!(count_script(&long).contains(r#"const query = "";"#));
    }
}
