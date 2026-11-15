use serde_json::Value;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DEPTH: usize = 64;
pub const MAX_TEXT: usize = 1024;
pub const MAX_CHILDREN: usize = 1000;
pub const MAX_NODES: usize = 100_000;

#[derive(Debug)]
pub enum Error {
    TooLarge,
    TooDeep,
    Invalid(serde_json::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooLarge => write!(f, "JSON larger than {MAX_BYTES} bytes"),
            Error::TooDeep => write!(f, "JSON nested deeper than {MAX_DEPTH} levels"),
            Error::Invalid(e) => write!(f, "invalid JSON: {e}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Root,
    Name(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: Key,
    pub kind: Kind,
    pub text: String,
    /// At most `MAX_CHILDREN` per container and `MAX_NODES` per document.
    pub children: Vec<Node>,
    /// The real number of children, which may exceed `children.len()`.
    pub child_count: usize,
}

pub fn parse(bytes: &[u8]) -> Result<Node, Error> {
    let value = parse_value(bytes)?;
    let mut budget = MAX_NODES;
    Ok(build(Key::Root, &value, &mut budget))
}

/// The tree and the pretty-printed text, cut at `max_pretty` bytes: indentation can make the
/// pretty form many times larger than the input.
pub fn parse_with_pretty(bytes: &[u8], max_pretty: usize) -> Result<(Node, String), Error> {
    let value = parse_value(bytes)?;
    let mut out = Capped { buf: Vec::new(), max: max_pretty };
    let complete = serde_json::to_writer_pretty(&mut out, &value).is_ok();
    let mut pretty = String::from_utf8_lossy(&out.buf).into_owned();
    if !complete {
        pretty.push_str("\n…");
    }
    let mut budget = MAX_NODES;
    Ok((build(Key::Root, &value, &mut budget), pretty))
}

struct Capped {
    buf: Vec<u8>,
    max: usize,
}

impl std::io::Write for Capped {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = self.max - self.buf.len();
        if room == 0 {
            return Err(std::io::Error::other("pretty text limit"));
        }
        let n = data.len().min(room);
        self.buf.extend_from_slice(&data[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn parse_value(bytes: &[u8]) -> Result<Value, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(Error::Invalid)?;
    if depth(&value) > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    Ok(value)
}

// serde_json's own recursion limit (128) keeps this walk shallow enough for the stack.
fn depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Value::Object(map) => 1 + map.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

fn build(key: Key, value: &Value, budget: &mut usize) -> Node {
    *budget = budget.saturating_sub(1);
    let (kind, text) = match value {
        Value::Null => (Kind::Null, "null".to_owned()),
        Value::Bool(b) => (Kind::Bool, b.to_string()),
        Value::Number(n) => (Kind::Number, bounded(&n.to_string())),
        Value::String(s) => (Kind::String, bounded(s)),
        Value::Array(_) => (Kind::Array, String::new()),
        Value::Object(_) => (Kind::Object, String::new()),
    };
    let mut children = Vec::new();
    let child_count = match value {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate().take(MAX_CHILDREN) {
                if *budget == 0 {
                    break;
                }
                children.push(build(Key::Index(i), item, budget));
            }
            items.len()
        }
        Value::Object(map) => {
            for (name, item) in map.iter().take(MAX_CHILDREN) {
                if *budget == 0 {
                    break;
                }
                children.push(build(Key::Name(bounded(name)), item, budget));
            }
            map.len()
        }
        _ => 0,
    };
    Node { key, kind, text, children, child_count }
}

fn bounded(s: &str) -> String {
    match s.char_indices().nth(MAX_TEXT) {
        Some((end, _)) => format!("{}…", &s[..end]),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tree() {
        let root = parse(br#"{"a":[1,true,null],"b":"x"}"#).unwrap();
        assert_eq!(root.kind, Kind::Object);
        assert_eq!(root.child_count, 2);
        let a = &root.children[0];
        assert_eq!(a.key, Key::Name("a".into()));
        assert_eq!(a.kind, Kind::Array);
        assert_eq!(a.children[1].key, Key::Index(1));
        assert_eq!(a.children[1].text, "true");
        assert_eq!(a.children[2].kind, Kind::Null);
        assert_eq!(root.children[1].text, "x");
    }

    #[test]
    fn rejects_invalid() {
        assert!(matches!(parse(b"{\"a\":"), Err(Error::Invalid(_))));
        assert!(matches!(parse(b""), Err(Error::Invalid(_))));
    }

    #[test]
    fn rejects_oversized() {
        let big = vec![b' '; MAX_BYTES + 1];
        assert!(matches!(parse(&big), Err(Error::TooLarge)));
    }

    #[test]
    fn rejects_too_deep() {
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(ok.as_bytes()).is_ok());
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(matches!(parse(deep.as_bytes()), Err(Error::TooDeep)));
        let very_deep = "[".repeat(100_000);
        assert!(parse(very_deep.as_bytes()).is_err());
    }

    #[test]
    fn bounds_text_and_children() {
        let long = format!("\"{}\"", "é".repeat(MAX_TEXT + 10));
        let node = parse(long.as_bytes()).unwrap();
        assert_eq!(node.text.chars().count(), MAX_TEXT + 1);
        assert!(node.text.ends_with('…'));

        let items = vec!["0"; MAX_CHILDREN + 5].join(",");
        let node = parse(format!("[{items}]").as_bytes()).unwrap();
        assert_eq!(node.children.len(), MAX_CHILDREN);
        assert_eq!(node.child_count, MAX_CHILDREN + 5);
    }

    #[test]
    fn bounds_total_nodes() {
        let row = format!("[{}]", vec!["0"; MAX_CHILDREN].join(","));
        let doc = format!("[{}]", vec![row; 200].join(","));
        let root = parse(doc.as_bytes()).unwrap();
        fn count(n: &Node) -> usize {
            1 + n.children.iter().map(count).sum::<usize>()
        }
        assert!(count(&root) <= MAX_NODES);
        assert_eq!(root.child_count, 200);
    }

    #[test]
    fn pretty_prints_within_limit() {
        let (root, pretty) = parse_with_pretty(br#"{"a":[1]}"#, 1024).unwrap();
        assert_eq!(root.child_count, 1);
        assert_eq!(pretty, "{\n  \"a\": [\n    1\n  ]\n}");
        let (_, cut) = parse_with_pretty(br#"{"a":[1,2,3,4,5]}"#, 10).unwrap();
        assert!(cut.len() <= 10 + "\n…".len() && cut.ends_with('…'));
        assert!(parse_with_pretty(b"nope", 1024).is_err());
    }
}
