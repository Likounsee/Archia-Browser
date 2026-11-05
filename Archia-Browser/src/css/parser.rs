use super::style::Property;
use super::tokenizer::CssToken;

const MAX_CSS_DECLARATIONS: usize = 4_096;
const MAX_CSS_DECLARATION_VALUE_BYTES: usize = 64 * 1024;

fn append_bounded(value: &mut String, part: &str) -> bool {
    let Some(next_len) = value.len().checked_add(part.len()) else {
        return false;
    };
    if next_len > MAX_CSS_DECLARATION_VALUE_BYTES || value.try_reserve(part.len()).is_err() {
        return false;
    }
    value.push_str(part);
    true
}

fn append_char_bounded(value: &mut String, character: char) -> bool {
    let mut encoded = [0; 4];
    append_bounded(value, character.encode_utf8(&mut encoded))
}

/// Copy attacker-controlled token text only after a fallible capacity reservation.
fn try_copy_string(value: &str) -> Option<String> {
    let mut copy = String::new();
    copy.try_reserve(value.len()).ok()?;
    copy.push_str(value);
    Some(copy)
}

/// Trim in place so a parsed declaration does not temporarily retain two
/// full-size copies of its value.
fn trim_in_place(value: &mut String) {
    let leading_bytes = value.len() - value.trim_start().len();
    let trimmed_bytes = value.trim().len();
    drop(value.drain(..leading_bytes));
    value.truncate(trimmed_bytes);
}

pub fn parse_declarations(tokens: &[CssToken]) -> Vec<Property> {
    let mut properties = Vec::new();
    let mut index = 0;

    while index < tokens.len() && properties.len() < MAX_CSS_DECLARATIONS {
        while matches!(
            tokens.get(index),
            Some(CssToken::Whitespace | CssToken::Semicolon)
        ) {
            index += 1;
        }

        let Some(CssToken::Ident(name)) = tokens.get(index) else {
            index += 1;
            continue;
        };
        let Some(name) = try_copy_string(name) else {
            return properties;
        };
        index += 1;

        while matches!(tokens.get(index), Some(CssToken::Whitespace)) {
            index += 1;
        }
        if !matches!(tokens.get(index), Some(CssToken::Colon)) {
            continue;
        }
        index += 1;

        let value_start = index;
        let declaration_end = find_declaration_end(tokens, value_start);
        let important_start = find_important_suffix(tokens, value_start, declaration_end);
        let value_end = important_start.unwrap_or(declaration_end);
        let important = important_start.is_some();
        let mut value = String::new();

        while index < value_end {
            let appended = match &tokens[index] {
                CssToken::Whitespace => {
                    value.ends_with(' ') || append_char_bounded(&mut value, ' ')
                }
                CssToken::Hash(v) => {
                    append_char_bounded(&mut value, '#') && append_bounded(&mut value, v)
                }
                CssToken::Ident(v) | CssToken::Number(v) | CssToken::String(v) => {
                    append_bounded(&mut value, v)
                }
                CssToken::Delim(c) => append_char_bounded(&mut value, *c),
                CssToken::Colon => append_char_bounded(&mut value, ':'),
                CssToken::LeftParen => append_char_bounded(&mut value, '('),
                CssToken::RightParen => append_char_bounded(&mut value, ')'),
                CssToken::LeftBrace => append_char_bounded(&mut value, '{'),
                CssToken::RightBrace => append_char_bounded(&mut value, '}'),
                CssToken::Semicolon => append_char_bounded(&mut value, ';'),
            };
            if !appended {
                return properties;
            }
            index += 1;
        }

        if properties.try_reserve(1).is_err() {
            return properties;
        }
        trim_in_place(&mut value);
        properties.push(Property {
            name,
            value,
            important,
        });
        index = declaration_end;
        if matches!(tokens.get(index), Some(CssToken::Semicolon)) {
            index += 1;
        }
    }

    properties
}

/// Find a declaration terminator without splitting semicolons nested in
/// functions or component-value blocks.
fn find_declaration_end(tokens: &[CssToken], start: usize) -> usize {
    let mut parentheses = 0usize;
    let mut braces = 0usize;
    let mut index = start;
    while index < tokens.len() {
        match tokens[index] {
            CssToken::LeftParen => parentheses = parentheses.saturating_add(1),
            CssToken::RightParen => parentheses = parentheses.saturating_sub(1),
            CssToken::LeftBrace => braces = braces.saturating_add(1),
            CssToken::RightBrace => braces = braces.saturating_sub(1),
            CssToken::Semicolon if parentheses == 0 && braces == 0 => break,
            _ => {}
        }
        index += 1;
    }
    index
}

/// Detect !important from token boundaries, never from text inside strings
/// or nested function arguments.
fn find_important_suffix(tokens: &[CssToken], start: usize, end: usize) -> Option<usize> {
    let mut cursor = end;
    while cursor > start && matches!(tokens.get(cursor - 1), Some(CssToken::Whitespace)) {
        cursor -= 1;
    }
    if !matches!(tokens.get(cursor.checked_sub(1)?), Some(CssToken::Ident(value)) if value.eq_ignore_ascii_case("important")) {
        return None;
    }
    cursor -= 1;
    while cursor > start && matches!(tokens.get(cursor - 1), Some(CssToken::Whitespace)) {
        cursor -= 1;
    }
    if matches!(tokens.get(cursor.checked_sub(1)?), Some(CssToken::Delim('!'))) {
        Some(cursor - 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::CssTokenizer;

    #[test]
    fn preserves_hash_prefix_in_declaration_values() {
        let tokens = CssTokenizer::tokenize("background-color: #102030;");
        let properties = parse_declarations(&tokens);
        assert_eq!(properties[0].value, "#102030");
        assert!(!properties[0].important);
    }

    #[test]
    fn oversized_declaration_value_stops_before_unbounded_concatenation() {
        let tokens = vec![
            CssToken::Ident("color".into()),
            CssToken::Colon,
            CssToken::Ident("a".repeat(MAX_CSS_DECLARATION_VALUE_BYTES / 2)),
            CssToken::Ident("b".repeat(MAX_CSS_DECLARATION_VALUE_BYTES / 2 + 1)),
            CssToken::Semicolon,
            CssToken::Ident("margin".into()),
            CssToken::Colon,
            CssToken::Number("0".into()),
            CssToken::Semicolon,
        ];
        assert!(parse_declarations(&tokens).is_empty());
    }

    #[test]
    fn important_is_recognized_from_tokens_not_string_contents() {
        let tokens = CssTokenizer::tokenize(
            r#"content: "hello!important"; color: red ! important;"#,
        );
        let properties = parse_declarations(&tokens);

        assert_eq!(properties[0].value, "hello!important");
        assert!(!properties[0].important);
        assert_eq!(properties[1].value, "red");
        assert!(properties[1].important);
    }

    #[test]
    fn semicolons_inside_functions_do_not_end_declarations() {
        let tokens = CssTokenizer::tokenize(
            "background-image: url(data:image/svg+xml;a=b); color: red;",
        );
        let properties = parse_declarations(&tokens);

        assert_eq!(properties.len(), 2);
        assert_eq!(properties[0].value, "url(data:image/svg+xml;a=b)");
        assert_eq!(properties[1].name, "color");
        assert_eq!(properties[1].value, "red");
    }

    #[test]
    fn trims_declaration_values_in_place_without_corrupting_unicode() {
        let mut value = "  café  ".to_owned();
        trim_in_place(&mut value);
        assert_eq!(value, "café");
    }

    #[test]
    fn declaration_count_is_bounded_at_the_configured_limit() {
        let mut tokens = Vec::new();
        for index in 0..=MAX_CSS_DECLARATIONS {
            tokens.extend([
                CssToken::Ident(format!("property-{index}")),
                CssToken::Colon,
                CssToken::Number("1".into()),
                CssToken::Semicolon,
            ]);
        }

        assert_eq!(parse_declarations(&tokens).len(), MAX_CSS_DECLARATIONS);
    }

    #[test]
    fn parses_simple_declarations() {
        let tokens = CssTokenizer::tokenize("color: red; margin: 0;");
        let properties = parse_declarations(&tokens);
        assert_eq!(properties[0].name, "color");
        assert_eq!(properties[0].value, "red");
        assert!(!properties[0].important);
        assert_eq!(properties[1].name, "margin");
        assert_eq!(properties[1].value, "0");
    }
}
