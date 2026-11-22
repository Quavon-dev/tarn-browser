use std::collections::HashSet;

use serde::Deserialize;

pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_GROUPS: usize = 50;
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub tab_ids: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    groups: Vec<Group>,
}

/// Validates a model's grouping proposal against the tab ids that were sent.
/// The caller still asks the user before applying it.
pub fn parse(output: &str, sent: &HashSet<i64>) -> Result<Vec<Group>, String> {
    if output.len() > MAX_OUTPUT_BYTES {
        return Err("proposal too large".into());
    }
    let json = match (output.find('{'), output.rfind('}')) {
        (Some(start), Some(end)) if start < end => &output[start..=end],
        _ => return Err("no JSON object".into()),
    };
    let mut groups = serde_json::from_str::<Proposal>(json).map_err(|e| e.to_string())?.groups;
    if groups.is_empty() || groups.len() > MAX_GROUPS {
        return Err("wrong number of groups".into());
    }
    let mut seen = HashSet::new();
    for g in &mut groups {
        g.name = g.name.chars().filter(|c| !c.is_control() && !crate::page_context::is_bidi(*c)).collect::<String>().trim().to_string();
        if g.name.is_empty() || g.name.chars().count() > MAX_NAME_CHARS {
            return Err("invalid group name".into());
        }
        if g.tab_ids.is_empty() {
            return Err("empty group".into());
        }
        for id in &g.tab_ids {
            if !sent.contains(id) {
                return Err(format!("unknown tab id {id}"));
            }
            if !seen.insert(*id) {
                return Err(format!("duplicate tab id {id}"));
            }
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent() -> HashSet<i64> {
        [1, 2, 3].into()
    }

    #[test]
    fn accepts_valid_proposal_in_fence() {
        let out = "```json\n{\"groups\":[{\"name\":\" News \",\"tab_ids\":[1,3]},{\"name\":\"Dev\",\"tab_ids\":[2]}]}\n```";
        let groups = parse(out, &sent()).unwrap();
        assert_eq!(groups[0], Group { name: "News".into(), tab_ids: vec![1, 3] });
    }

    #[test]
    fn names_lose_bidi_and_zero_width_characters() {
        let named = |name: &str| format!(r#"{{"groups":[{{"name":"{name}","tab_ids":[1]}}]}}"#);
        assert_eq!(parse(&named("\u{202e}Ne\u{200b}ws\u{2066}"), &sent()).unwrap()[0].name, "News");
        assert!(parse(&named("\u{200b}\u{202e}"), &sent()).is_err(), "a name of nothing visible is refused");
    }

    #[test]
    fn rejects_bad_proposals() {
        for bad in [
            r#"{"groups":[{"name":"A","tab_ids":[1,99]}]}"#,
            r#"{"groups":[{"name":"A","tab_ids":[1]},{"name":"B","tab_ids":[1]}]}"#,
            r#"{"groups":[{"name":"  ","tab_ids":[1]}]}"#,
            r#"{"groups":[{"name":"A","tab_ids":[]}]}"#,
            r#"{"groups":[]}"#,
            r#"{"groups":[{"name":"A","tab_ids":[1],"open":"https://evil"}]}"#,
            "no json",
        ] {
            assert!(parse(bad, &sent()).is_err(), "{bad}");
        }
        let huge = format!("{{\"groups\":[{{\"name\":\"{}\",\"tab_ids\":[1]}}]}}", "a".repeat(MAX_OUTPUT_BYTES));
        assert!(parse(&huge, &sent()).is_err());
    }
}
