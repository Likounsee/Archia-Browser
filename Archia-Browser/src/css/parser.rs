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
        let name = name.clone();
        index += 1;

        while matches!(tokens.get(index), Some(CssToken::Whitespace)) {
            index += 1;
        }
        if !matches!(tokens.get(index), Some(CssToken::Colon)) {
            continue;
        }
        index += 1;

        let mut value = String::new();
        while index < tokens.len() && !matches!(tokens.get(index), Some(CssToken::Semicolon)) {
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
                CssToken::LeftBrace | CssToken::RightBrace | CssToken::Semicolon => true,
            };
            if !appended {
                // Stop before copying attacker-controlled token text beyond the declaration budget.
                return properties;
            }
            index += 1;
        }

        if value.len() > MAX_CSS_DECLARATION_VALUE_BYTES || properties.try_reserve(1).is_err() {
            return properties;
        }
        properties.push(Property {
            name,
            value: value.trim().to_owned(),
        });
        if matches!(tokens.get(index), Some(CssToken::Semicolon)) {
            index += 1;
        }
    }

    properties
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
    fn parses_simple_declarations() {
        let tokens = CssTokenizer::tokenize("color: red; margin: 0;");
        let properties = parse_declarations(&tokens);
        assert_eq!(properties[0].name, "color");
        assert_eq!(properties[0].value, "red");
        assert_eq!(properties[1].name, "margin");
        assert_eq!(properties[1].value, "0");
    }
}
