#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssToken {
    Ident(String),
    Hash(String),
    Number(String),
    Whitespace,
    Colon,
    Semicolon,
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    Delim(char),
    String(String),
}

const MAX_CSS_INPUT_BYTES: usize = 1024 * 1024;
const MAX_CSS_TOKENS: usize = 100_000;

#[derive(Debug, Default)]
pub struct CssTokenizer;

impl CssTokenizer {
    pub fn tokenize(input: &str) -> Vec<CssToken> {
        // This tokenizer is also a public entry point, so callers must not be
        // able to bypass the stylesheet parser's input-size budget. Truncate
        // only at a UTF-8 boundary and cap token growth independently.
        let input = if input.len() > MAX_CSS_INPUT_BYTES {
            let mut end = MAX_CSS_INPUT_BYTES;
            while !input.is_char_boundary(end) {
                end -= 1;
            }
            &input[..end]
        } else {
            input
        };
        let mut tokens = Vec::new();
        let mut chars = input.chars().peekable();

        while tokens.len() < MAX_CSS_TOKENS {
            let Some(ch) = chars.next() else {
                break;
            };
            match ch {
                c if c.is_whitespace() => {
                    if !matches!(tokens.last(), Some(CssToken::Whitespace)) {
                        tokens.push(CssToken::Whitespace);
                    }
                }
                ':' => tokens.push(CssToken::Colon),
                ';' => tokens.push(CssToken::Semicolon),
                '{' => tokens.push(CssToken::LeftBrace),
                '}' => tokens.push(CssToken::RightBrace),
                '(' => tokens.push(CssToken::LeftParen),
                ')' => tokens.push(CssToken::RightParen),
                '#' => {
                    let value = consume_ident(&mut chars);
                    tokens.push(CssToken::Hash(value));
                }
                '"' | '\'' => {
                    let quote = ch;
                    let mut value = String::new();
                    while let Some(next) = chars.next() {
                        if next == quote {
                            break;
                        }
                        value.push(next);
                    }
                    tokens.push(CssToken::String(value));
                }
                c if c.is_ascii_digit()
                    || c == '.' && chars.peek().is_some_and(|n| n.is_ascii_digit()) =>
                {
                    let mut value = String::from(c);
                    while let Some(next) = chars.peek().copied() {
                        if next.is_ascii_digit() || next == '.' || next == '-' {
                            value.push(next);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    tokens.push(CssToken::Number(value));
                }
                c if is_ident_start(c) => {
                    let mut value = String::from(c);
                    value.push_str(&consume_ident(&mut chars));
                    tokens.push(CssToken::Ident(value));
                }
                c => tokens.push(CssToken::Delim(c)),
            }
        }

        tokens
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_alphabetic() || !c.is_ascii()
}

fn consume_ident(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut value = String::new();
    while let Some(next) = chars.peek().copied() {
        if next == '_' || next == '-' || next.is_ascii_alphanumeric() || !next.is_ascii() {
            value.push(next);
            chars.next();
        } else {
            break;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_declaration() {
        let tokens = CssTokenizer::tokenize("color: #fff;");
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("color".into()),
                CssToken::Colon,
                CssToken::Whitespace,
                CssToken::Hash("fff".into()),
                CssToken::Semicolon,
            ]
        );
    }

    #[test]
    fn direct_tokenizer_input_is_bounded_on_utf8_boundaries() {
        let input = "é".repeat(MAX_CSS_INPUT_BYTES);
        let tokens = CssTokenizer::tokenize(&input);

        assert_eq!(tokens.len(), 1);
        let CssToken::Ident(value) = &tokens[0] else {
            panic!("expected one identifier token");
        };
        assert!(value.len() <= MAX_CSS_INPUT_BYTES);
        assert!(value.len() >= MAX_CSS_INPUT_BYTES - 1);
    }

    #[test]
    fn token_count_is_bounded_for_many_small_tokens() {
        let input = ":".repeat(MAX_CSS_TOKENS + 1);
        let tokens = CssTokenizer::tokenize(&input);

        assert_eq!(tokens.len(), MAX_CSS_TOKENS);
        assert!(tokens.iter().all(|token| matches!(token, CssToken::Colon)));
    }

    #[test]
    fn empty_input_produces_no_tokens() {
        assert!(CssTokenizer::tokenize("").is_empty());
    }
}
