pub fn count_script(text: &str) -> String {
    let query = serde_json::to_string(&text.to_lowercase()).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        "(() => {{ const query = {query}; if (!query) return 0; \
         const text = (document.body ? document.body.innerText : '').toLowerCase(); \
         let count = 0; for (let at = text.indexOf(query); at !== -1; at = text.indexOf(query, at + query.length)) count++; \
         return count; }})()"
    )
}

pub fn matched(found: bool, counted: Option<f64>) -> u64 {
    if !found {
        return 0;
    }
    counted.filter(|n| n.is_finite() && *n >= 1.0).map_or(1, |n| n as u64)
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
    }
}
