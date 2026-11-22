pub const MAX_INPUT: usize = 200;
const MAX_DEPTH: usize = 32;

/// The value of an arithmetic expression: `+ - * / % ^`, parentheses and decimals. Plain numbers,
/// text over `MAX_INPUT` bytes, nesting deeper than 32 and results that are not finite give `None`.
pub fn evaluate(text: &str) -> Option<f64> {
    let text = text.trim().strip_prefix('=').unwrap_or(text).trim();
    if text.len() > MAX_INPUT || !text.contains(|c: char| "+-*/%^(".contains(c)) {
        return None;
    }
    let mut parser = Parser { bytes: text.as_bytes(), at: 0, depth: 0, operators: 0 };
    let value = parser.sum()?;
    parser.skip_space();
    (parser.at == parser.bytes.len() && parser.operators > 0 && value.is_finite()).then_some(value)
}

/// A result as the answer row shows and copies it: whole numbers without a decimal point, others
/// rounded to ten significant digits.
pub fn format(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let digits = (9 - value.abs().log10().floor() as i32).clamp(0, 15) as usize;
    let text = format!("{value:.digits$}");
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
    operators: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, ops: &[u8]) -> Option<u8> {
        self.skip_space();
        let op = *self.bytes.get(self.at).filter(|b| ops.contains(b))?;
        self.at += 1;
        self.operators += 1;
        Some(op)
    }

    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        while let Some(op) = self.eat(b"+-") {
            let right = self.product()?;
            value = if op == b'+' { value + right } else { value - right };
        }
        Some(value)
    }

    fn product(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        while let Some(op) = self.eat(b"*/%") {
            let right = self.unary()?;
            value = match op {
                b'*' => value * right,
                b'/' => value / right,
                _ => value % right,
            };
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.eat(b"-+") {
            Some(op) => {
                self.depth += 1;
                let value = (self.depth <= MAX_DEPTH).then(|| self.unary()).flatten();
                self.depth -= 1;
                value.map(|v| if op == b'-' { -v } else { v })
            }
            None => self.power(),
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat(b"^").is_none() {
            return Some(base);
        }
        self.depth += 1;
        let exponent = (self.depth <= MAX_DEPTH).then(|| self.unary()).flatten();
        self.depth -= 1;
        Some(base.powf(exponent?))
    }

    fn atom(&mut self) -> Option<f64> {
        self.skip_space();
        if self.bytes.get(self.at) == Some(&b'(') {
            self.at += 1;
            self.depth += 1;
            let value = (self.depth <= MAX_DEPTH).then(|| self.sum()).flatten();
            self.depth -= 1;
            self.skip_space();
            return (self.bytes.get(self.at) == Some(&b')')).then(|| {
                self.at += 1;
                value
            })?;
        }
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
            self.at += 1;
        }
        let number = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        if number.is_empty() || number.matches('.').count() > 1 {
            return None;
        }
        number.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(text: &str) -> Option<String> {
        evaluate(text).map(format)
    }

    #[test]
    fn evaluates_with_precedence_and_parentheses() {
        assert_eq!(shown("1 + 2 * 3").as_deref(), Some("7"));
        assert_eq!(shown("(1 + 2) * 3").as_deref(), Some("9"));
        assert_eq!(shown("2 ^ 3 ^ 2").as_deref(), Some("512"), "powers group to the right");
        assert_eq!(shown("-2 ^ 2").as_deref(), Some("-4"));
        assert_eq!(shown("10 % 4").as_deref(), Some("2"));
        assert_eq!(shown("7 / 2").as_deref(), Some("3.5"));
        assert_eq!(shown("= 6 * 7").as_deref(), Some("42"));
        assert_eq!(shown("1 / 3").as_deref(), Some("0.3333333333"));
        assert_eq!(shown("0.1 + 0.2").as_deref(), Some("0.3"));
        assert_eq!(shown("-(3)").as_deref(), Some("-3"));
        assert_eq!(shown("-0.0000001 * 0").as_deref(), Some("0"));
    }

    #[test]
    fn refuses_plain_numbers_text_and_endless_results() {
        for text in ["42", "", "  ", "abc", "1 +", "(1", "1)", "1..2 + 1", "1.2.3.4", "1 / 0", "2 ^ 99999", "1 + x", "example.com", "3 -- -"] {
            assert_eq!(evaluate(text), None, "{text}");
        }
        assert!(evaluate(&format!("1{}", "+1".repeat(MAX_INPUT))).is_none(), "too long");
        let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(evaluate(&deep), None, "nesting is bounded");
        assert_eq!(evaluate(&format!("1+{}1", "-".repeat(150))), None);
        assert_eq!(evaluate("(((1)))+1"), Some(2.0));
    }
}
