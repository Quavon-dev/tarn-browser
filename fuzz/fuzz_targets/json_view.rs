#![no_main]

use devkit::json_view::{MAX_CHILDREN, MAX_NODES, MAX_TEXT, Node, parse, parse_with_pretty};
use libfuzzer_sys::fuzz_target;

const MAX_PRETTY: usize = 4096;

fn count(node: &Node) -> usize {
    assert!(node.children.len() <= MAX_CHILDREN && node.children.len() <= node.child_count);
    assert!(node.text.chars().count() <= MAX_TEXT + 1);
    1 + node.children.iter().map(count).sum::<usize>()
}

fuzz_target!(|data: &[u8]| {
    if let Ok(root) = parse(data) {
        assert!(count(&root) <= MAX_NODES + 1);
    }
    if let Ok((root, pretty)) = parse_with_pretty(data, MAX_PRETTY) {
        assert!(count(&root) <= MAX_NODES + 1);
        assert!(pretty.len() <= MAX_PRETTY + "\n…".len() + 3);
    }
});
