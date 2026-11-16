#![no_main]

use engine::reader::{Article, Block, MAX_BLOCK, MAX_BYTES};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(json) = std::str::from_utf8(data) else { return };
    let Some(article) = Article::parse(json) else { return };
    assert!(!article.blocks.is_empty());
    assert!(article.title.chars().count() <= 1024 && article.byline.chars().count() <= 256);
    let mut total = 0;
    for block in &article.blocks {
        let (Block::Heading(text) | Block::Paragraph(text)) = block;
        assert!(!text.is_empty() && text.chars().count() <= MAX_BLOCK);
        assert!(!text.chars().any(char::is_control));
        total += text.len();
    }
    assert!(total <= MAX_BYTES);
});
