use qrcode::{Color, EcLevel, QrCode};

/// The QR code of `text` (error correction M) as rows of dark modules with a one-module light
/// border, the shape `share::qr_code` gives on macOS; `None` when the text is too long.
pub fn encode(text: &str) -> Option<Vec<Vec<bool>>> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    let light = vec![false; width + 2];
    let rows = colors.chunks(width).map(|row| std::iter::once(false).chain(row.iter().map(|c| *c == Color::Dark)).chain(std::iter::once(false)).collect());
    Some(std::iter::once(light.clone()).chain(rows).chain(std::iter::once(light)).collect())
}

#[cfg(test)]
mod tests {
    use super::encode;

    fn finder(code: &[Vec<bool>], top: usize, left: usize) -> bool {
        (0..7).all(|y| {
            (0..7).all(|x| {
                let ring = y.min(x).min(6 - y).min(6 - x);
                code[top + y][left + x] == (ring != 1)
            })
        })
    }

    #[test]
    fn codes_have_their_three_finder_patterns_inside_a_light_border() {
        let code = encode("https://example.com/a?b=c").unwrap();
        let n = code.len();
        assert!(code.iter().all(|row| row.len() == n));
        assert!(!code[0].contains(&true) && !code[n - 1].contains(&true) && code.iter().all(|r| !r[0] && !r[n - 1]));
        let size = n - 2;
        assert!(size >= 21 && (size - 21).is_multiple_of(4), "{size} modules");
        assert!(finder(&code, 1, 1) && finder(&code, 1, n - 8) && finder(&code, n - 8, 1));
        assert!(!finder(&code, n - 8, n - 8));
        assert!(encode(&"a".repeat(5000)).is_none());
    }
}
