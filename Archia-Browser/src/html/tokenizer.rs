use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlToken {
    Doctype(String),
    StartTag { name: String, attributes: BTreeMap<String, String>, self_closing: bool },
    EndTag(String),
    Text(String),
    Comment(String),
}

#[derive(Debug, Default)]
pub struct HtmlTokenizer;

impl HtmlTokenizer {
    pub fn tokenize(input: &str) -> Vec<HtmlToken> {
        let mut tokens = Vec::new();
        let mut cursor = 0;
        let bytes = input.as_bytes();

        while cursor < bytes.len() {
            if bytes[cursor] != b'<' {
                let end = input[cursor..].find('<').map_or(input.len(), |offset| cursor + offset);
                let text = &input[cursor..end];
                if !text.is_empty() { tokens.push(HtmlToken::Text(text.to_owned())); }
                cursor = end;
                continue;
            }

            if input[cursor..].starts_with("<!--") {
                if let Some(offset) = input[cursor + 4..].find("-->") {
                    let end = cursor + 4 + offset;
                    tokens.push(HtmlToken::Comment(input[cursor + 4..end].to_owned()));
                    cursor = end + 3;
                    continue;
                }
            }

            let Some(offset) = find_tag_end(&input[cursor..]) else {
                tokens.push(HtmlToken::Text(input[cursor..].to_owned()));
                break;
            };
            let end = cursor + offset;
            let inside = input[cursor + 1..end].trim();

            if let Some(doctype) = inside.strip_prefix("!DOCTYPE").or_else(|| inside.strip_prefix("!doctype")) {
                tokens.push(HtmlToken::Doctype(doctype.trim().to_owned()));
            } else if let Some(name) = inside.strip_prefix('/') {
                tokens.push(HtmlToken::EndTag(name.trim().to_ascii_lowercase()));
            } else if let Some((name, attributes, self_closing)) = parse_start_tag(inside) {
                tokens.push(HtmlToken::StartTag { name, attributes, self_closing });
            }
            cursor = end + 1;
        }
        tokens
    }
}

fn find_tag_end(input: &str) -> Option<usize> {
    let mut quote = None;
    for (index, ch) in input.char_indices() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn parse_start_tag(input: &str) -> Option<(String, BTreeMap<String, String>, bool)> {
    let mut chars = input.chars().peekable();
    let mut name = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() || ch == '/' { break; }
        name.push(ch.to_ascii_lowercase());
        chars.next();
    }
    if name.is_empty() { return None; }

    let mut attributes = BTreeMap::new();
    let mut self_closing = false;
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) { chars.next(); }
        if chars.peek().is_none() { break; }
        if chars.peek() == Some(&'/') {
            chars.next();
            self_closing = true;
            while chars.peek().is_some_and(|c| c.is_whitespace()) { chars.next(); }
            break;
        }

        let mut attr_name = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() || matches!(ch, '=' | '/') { break; }
            attr_name.push(ch.to_ascii_lowercase());
            chars.next();
        }
        if attr_name.is_empty() {
            chars.next();
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) { chars.next(); }

        let mut value = String::new();
        if chars.peek() == Some(&'=') {
            chars.next();
            while chars.peek().is_some_and(|c| c.is_whitespace()) { chars.next(); }
            let quote = chars.peek().copied().filter(|c| *c == '\'' || *c == '"');
            if let Some(q) = quote {
                chars.next();
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch == q { break; }
                    value.push(ch);
                }
            } else {
                while let Some(&ch) = chars.peek() {
                    if ch.is_whitespace() || ch == '/' { break; }
                    value.push(ch);
                    chars.next();
                }
            }
        }
        attributes.insert(attr_name, value);
    }
    Some((name, attributes, self_closing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_basic_document() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        assert_eq!(tokens, vec![
            HtmlToken::StartTag { name: "html".into(), attributes: BTreeMap::new(), self_closing: false },
            HtmlToken::StartTag { name: "body".into(), attributes: BTreeMap::new(), self_closing: false },
            HtmlToken::Text("Hello".into()),
            HtmlToken::EndTag("body".into()),
            HtmlToken::EndTag("html".into()),
        ]);
    }

    #[test]
    fn tokenizes_attributes_and_quoted_gt() {
        let tokens = HtmlTokenizer::tokenize(r#"<div id="main" class='card active' title="a > b">x</div>"#);
        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else { panic!("expected start tag"); };
        assert_eq!(attributes.get("id"), Some(&"main".to_string()));
        assert_eq!(attributes.get("class"), Some(&"card active".to_string()));
        assert_eq!(attributes.get("title"), Some(&"a > b".to_string()));
    }

    #[test]
    fn tokenizes_comments_and_doctype() {
        let tokens = HtmlTokenizer::tokenize("<!DOCTYPE html><!--x-->");
        assert_eq!(tokens[0], HtmlToken::Doctype("html".into()));
        assert_eq!(tokens[1], HtmlToken::Comment("x".into()));
    }
}
