use super::style::Property;
use super::tokenizer::CssToken;

pub fn parse_declarations(tokens: &[CssToken]) -> Vec<Property> {
    let mut properties = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
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
            match &tokens[index] {
                CssToken::Whitespace => {
                    if !value.ends_with(' ') {
                        value.push(' ');
                    }
                }
                CssToken::Hash(v) => {
                    value.push('#');
                    value.push_str(v);
                }
                CssToken::Ident(v) | CssToken::Number(v) | CssToken::String(v) => value.push_str(v),
                CssToken::Delim(c) => value.push(*c),
                CssToken::Colon => value.push(':'),
                CssToken::LeftParen => value.push('('),
                CssToken::RightParen => value.push(')'),
                CssToken::LeftBrace | CssToken::RightBrace | CssToken::Semicolon => {}
            }
            index += 1;
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
    fn parses_simple_declarations() {
        let tokens = CssTokenizer::tokenize("color: red; margin: 0;");
        let properties = parse_declarations(&tokens);
        assert_eq!(properties[0].name, "color");
        assert_eq!(properties[0].value, "red");
        assert_eq!(properties[1].name, "margin");
        assert_eq!(properties[1].value, "0");
    }
}
