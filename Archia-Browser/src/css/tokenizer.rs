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
const MAX_CSS_TOKEN_TEXT_BYTES: usize = 64 * 1024;

fn is_css_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\u{000C}' | '\r' | ' ')
}

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
        macro_rules! push_token {
            ($token:expr) => {{
                if !try_push_css_token(&mut tokens, $token) {
                    return tokens;
                }
            }};
        }

        while tokens.len() < MAX_CSS_TOKENS {
            let Some(ch) = chars.next() else {
                break;
            };

            // CSS comments are lexical trivia, not declaration values. Consume
            // them before tokenization, without interpreting comment markers
            // inside quoted strings (which are handled in the string branch).
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
                continue;
            }

            match ch {
                c if is_css_whitespace(c) => {
                    if !matches!(tokens.last(), Some(CssToken::Whitespace)) {
                        push_token!(CssToken::Whitespace);
                    }
                }
                ':' => push_token!(CssToken::Colon),
                ';' => push_token!(CssToken::Semicolon),
                '{' => push_token!(CssToken::LeftBrace),
                '}' => push_token!(CssToken::RightBrace),
                '(' => push_token!(CssToken::LeftParen),
                ')' => push_token!(CssToken::RightParen),
                '#' => {
                    let starts_hash = chars.peek().copied().is_some_and(|next| {
                        is_name_code_point(next) || next == '\\' && starts_valid_escape(&chars)
                    });
                    if starts_hash {
                        let value = consume_ident(&mut chars);
                        if value.len() > MAX_CSS_TOKEN_TEXT_BYTES {
                            return tokens;
                        }
                        push_token!(CssToken::Hash(value));
                    } else {
                        push_token!(CssToken::Delim('#'));
                    }
                }
                '"' | '\'' => {
                    let quote = ch;
                    let mut value = String::new();
                    while let Some(next) = chars.next() {
                        if next == quote {
                            break;
                        }
                        if next == '\\' {
                            match chars.peek().copied() {
                                Some(newline) if is_css_newline(newline) => {
                                    chars.next();
                                    if newline == '\r' && chars.peek() == Some(&'\n') {
                                        chars.next();
                                    }
                                }
                                Some(_) => {
                                    if let Some(escaped) = consume_escape(&mut chars) {
                                        value.push(escaped);
                                    }
                                }
                                None => break,
                            }
                        } else {
                            value.push(next);
                        }
                    }
                    if value.len() > MAX_CSS_TOKEN_TEXT_BYTES {
                        return tokens;
                    }
                    push_token!(CssToken::String(value));
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
                    if value.len() > MAX_CSS_TOKEN_TEXT_BYTES {
                        return tokens;
                    }
                    push_token!(CssToken::Number(value));
                }
                c if is_ident_start(c) => {
                    let mut value = String::from(c);
                    value.push_str(&consume_ident(&mut chars));
                    if value.len() > MAX_CSS_TOKEN_TEXT_BYTES {
                        return tokens;
                    }
                    push_token!(CssToken::Ident(value));
                }
                '\\' if chars.peek().is_some_and(|next| !is_css_newline(*next)) => {
                    if let Some(escaped) = consume_escape(&mut chars) {
                        let mut value = String::new();
                        value.push(escaped);
                        value.push_str(&consume_ident(&mut chars));
                        if value.len() > MAX_CSS_TOKEN_TEXT_BYTES {
                            return tokens;
                        }
                        push_token!(CssToken::Ident(value));
                    } else {
                        push_token!(CssToken::Delim('\\'));
                    }
                }
                c => push_token!(CssToken::Delim(c)),
            }
        }

        tokens
    }
}

/// Append only after the token vector has obtained capacity; on failure the caller
/// can retain the already-tokenized prefix.
fn try_push_css_token(tokens: &mut Vec<CssToken>, token: CssToken) -> bool {
    if tokens.len() >= MAX_CSS_TOKENS || tokens.try_reserve(1).is_err() {
        return false;
    }
    tokens.push(token);
    true
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_alphabetic() || !c.is_ascii()
}

fn consume_ident(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut value = String::new();
    while let Some(next) = chars.peek().copied() {
        if is_name_code_point(next) {
            value.push(next);
            chars.next();
        } else if next == '\\' && starts_valid_escape(chars) {
            chars.next();
            if let Some(escaped) = consume_escape(chars) {
                value.push(escaped);
            }
        } else {
            break;
        }
    }
    value
}

fn is_name_code_point(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_alphanumeric() || !c.is_ascii()
}

fn is_css_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{000C}')
}

fn starts_valid_escape(chars: &std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let mut lookahead = chars.clone();
    lookahead.next() == Some('\\') && lookahead.peek().is_some_and(|next| !is_css_newline(*next))
}

/// Consume a CSS escape, decoding up to six hexadecimal digits and optional whitespace.
fn consume_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<char> {
    let first = chars.next()?;
    if is_css_newline(first) {
        return None;
    }
    if first.is_ascii_hexdigit() {
        let mut digits = String::from(first);
        while digits.len() < 6 && chars.peek().is_some_and(|next| next.is_ascii_hexdigit()) {
            digits.push(chars.next()?);
        }
        if let Some(whitespace) = chars.peek().copied().filter(|next| is_css_whitespace(*next)) {
            chars.next();
            if whitespace == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
        }
        let code_point = u32::from_str_radix(&digits, 16).ok()?;
        Some(
            char::from_u32(code_point)
                .filter(|ch| *ch != '\0')
                .unwrap_or('\u{FFFD}'),
        )
    } else {
        Some(first)
    }
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
    fn direct_tokenizer_rejects_identifier_text_over_the_token_limit() {
        let input = "é".repeat(MAX_CSS_INPUT_BYTES);
        let tokens = CssTokenizer::tokenize(&input);

        // Oversized identifier text must not be retained as a token.
        assert!(tokens.is_empty());
    }

    #[test]
    fn token_count_is_bounded_for_many_small_tokens() {
        let input = ":".repeat(MAX_CSS_TOKENS + 1);
        let tokens = CssTokenizer::tokenize(&input);

        assert_eq!(tokens.len(), MAX_CSS_TOKENS);
        assert!(tokens.iter().all(|token| matches!(token, CssToken::Colon)));
    }

    #[test]
    fn skips_comments_between_css_tokens() {
        let tokens = CssTokenizer::tokenize("color/**/: /* value */ red;");
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("color".into()),
                CssToken::Colon,
                CssToken::Whitespace,
                CssToken::Ident("red".into()),
                CssToken::Semicolon,
            ]
        );
    }

    #[test]
    fn decodes_css_escapes_in_identifiers_and_hashes() {
        assert_eq!(
            CssTokenizer::tokenize(r"\63 olor"),
            vec![CssToken::Ident("color".into())]
        );
        assert_eq!(
            CssTokenizer::tokenize(r"foo\;bar"),
            vec![CssToken::Ident("foo;bar".into())]
        );
        assert_eq!(
            CssTokenizer::tokenize(r"#\31 23"),
            vec![CssToken::Hash("123".into())]
        );
        assert_eq!(
            CssTokenizer::tokenize(r"\110000 "),
            vec![CssToken::Ident("\u{FFFD}".into())]
        );
    }

    #[test]
    fn invalid_newline_escape_does_not_consume_the_newline() {
        assert_eq!(
            CssTokenizer::tokenize("foo\\\nbar"),
            vec![
                CssToken::Ident("foo".into()),
                CssToken::Delim('\\'),
                CssToken::Whitespace,
                CssToken::Ident("bar".into()),
            ]
        );
    }

    #[test]
    fn decodes_escapes_in_strings_without_ending_on_escaped_quotes() {
        let tokens = CssTokenizer::tokenize(r#"content: "a\"b\26 c/*literal*/";"#);
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("content".into()),
                CssToken::Colon,
                CssToken::Whitespace,
                CssToken::String("a\"b&c/*literal*/".into()),
                CssToken::Semicolon,
            ]
        );
    }

    #[test]
    fn removes_css_string_line_continuations_and_crlf_escape_whitespace() {
        assert_eq!(
            CssTokenizer::tokenize("\"left\\\r\nright\""),
            vec![CssToken::String("leftright".into())]
        );
        assert_eq!(
            CssTokenizer::tokenize("foo\\000026\r\nbar"),
            vec![CssToken::Ident("foo&bar".into())]
        );
    }

    #[test]
    fn lone_hash_is_a_delimiter_not_an_empty_hash_token() {
        assert_eq!(
            CssTokenizer::tokenize("#;"),
            vec![CssToken::Delim('#'), CssToken::Semicolon]
        );
    }

    #[test]
    fn unterminated_comment_consumes_the_remaining_input() {
        let tokens = CssTokenizer::tokenize("color: red; /* unfinished");
        assert_eq!(
            tokens,
            vec![
                CssToken::Ident("color".into()),
                CssToken::Colon,
                CssToken::Whitespace,
                CssToken::Ident("red".into()),
                CssToken::Semicolon,
                CssToken::Whitespace,
            ]
        );
    }

    #[test]
    fn unicode_space_characters_are_not_css_whitespace_tokens() {
        assert_eq!(
            CssTokenizer::tokenize("\u{00A0}\u{2003}"),
            vec![CssToken::Ident("\u{00A0}\u{2003}".into())]
        );
    }

    #[test]
    fn unicode_space_does_not_terminate_a_css_hex_escape() {
        assert_eq!(
            CssTokenizer::tokenize("foo\\26\u{00A0}bar"),
            vec![CssToken::Ident("foo&\u{00A0}bar".into())]
        );
    }

    #[test]
    fn empty_input_produces_no_tokens() {
        assert!(CssTokenizer::tokenize("").is_empty());
    }
}
