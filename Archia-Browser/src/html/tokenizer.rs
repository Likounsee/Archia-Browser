use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlToken {
    Doctype(String),
    StartTag {
        name: String,
        attributes: BTreeMap<String, String>,
        self_closing: bool,
    },
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
                let end = input[cursor..]
                    .find('<')
                    .map_or(input.len(), |offset| cursor + offset);
                let text = &input[cursor..end];
                if !text.is_empty() {
                    tokens.push(HtmlToken::Text(decode_character_references(text)));
                }
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

                tokens.push(HtmlToken::Comment(input[cursor + 4..].to_owned()));
                break;
            }

            let Some(offset) = find_tag_end(&input[cursor..]) else {
                tokens.push(HtmlToken::Text(input[cursor..].to_owned()));
                break;
            };
            let end = cursor + offset;
            let inside = input[cursor + 1..end].trim();

            if inside.len() >= 9 && inside[..9].eq_ignore_ascii_case("!doctype") {
                tokens.push(HtmlToken::Doctype(inside[9..].trim().to_owned()));
            } else if let Some(name) = inside.strip_prefix('/') {
                tokens.push(HtmlToken::EndTag(name.trim().to_ascii_lowercase()));
            } else if let Some((name, attributes, self_closing)) = parse_start_tag(inside) {
                let is_raw_text = matches!(name.as_str(), "script" | "style");
                tokens.push(HtmlToken::StartTag {
                    name: name.clone(),
                    attributes,
                    self_closing,
                });

                if is_raw_text && !self_closing {
                    if let Some((text_end, close_end)) =
                        find_raw_text_end(input, end + 1, &name)
                    {
                        if text_end > end + 1 {
                            tokens.push(HtmlToken::Text(
                                input[end + 1..text_end].to_owned(),
                            ));
                        }
                        cursor = close_end;
                        continue;
                    }
                }
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
        if ch.is_whitespace() || ch == '/' {
            break;
        }
        name.push(ch.to_ascii_lowercase());
        chars.next();
    }
    if name.is_empty() {
        return None;
    }

    let mut attributes = BTreeMap::new();
    let mut self_closing = false;
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        if chars.peek() == Some(&'/') {
            chars.next();
            self_closing = true;
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            break;
        }

        let mut attr_name = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() || matches!(ch, '=' | '/') {
                break;
            }
            attr_name.push(ch.to_ascii_lowercase());
            chars.next();
        }
        if attr_name.is_empty() {
            chars.next();
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }

        let mut value = String::new();
        if chars.peek() == Some(&'=') {
            chars.next();
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            let quote = chars.peek().copied().filter(|c| *c == '\'' || *c == '"');
            if let Some(q) = quote {
                chars.next();
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch == q {
                        break;
                    }
                    value.push(ch);
                }
            } else {
                while let Some(&ch) = chars.peek() {
                    if ch.is_whitespace() || ch == '/' {
                        break;
                    }
                    value.push(ch);
                    chars.next();
                }
            }
        }
        attributes.insert(attr_name, decode_character_references(&value));
    }
    Some((name, attributes, self_closing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_basic_document() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        assert_eq!(
            tokens,
            vec![
                HtmlToken::StartTag {
                    name: "html".into(),
                    attributes: BTreeMap::new(),
                    self_closing: false
                },
                HtmlToken::StartTag {
                    name: "body".into(),
                    attributes: BTreeMap::new(),
                    self_closing: false
                },
                HtmlToken::Text("Hello".into()),
                HtmlToken::EndTag("body".into()),
                HtmlToken::EndTag("html".into()),
            ]
        );
    }

    #[test]
    fn tokenizes_attributes_and_quoted_gt() {
        let tokens =
            HtmlTokenizer::tokenize(r#"<div id="main" class='card active' title="a > b">x</div>"#);
        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected start tag");
        };
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

fn find_raw_text_end(input: &str, start: usize, tag_name: &str) -> Option<(usize, usize)> {
    let lower = input[start..].to_ascii_lowercase();
    let marker = format!("</{tag_name}");
    let relative = lower.find(&marker)?;
    let text_end = start + relative;
    let after_name = text_end + marker.len();
    let close_end = input[after_name..]
        .find('>')
        .map(|offset| after_name + offset + 1)?;
    Some((text_end, close_end))
}

fn decode_character_references(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(relative) = input[cursor..].find('&') {
        let start = cursor + relative;
        output.push_str(&input[cursor..start]);

        let Some(end_relative) = input[start + 1..].find(';') else {
            output.push_str(&input[start..]);
            return output;
        };
        let end = start + 1 + end_relative;
        let reference = &input[start + 1..end];

        let decoded = match reference {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if reference.starts_with("#x") || reference.starts_with("#X") => {
                u32::from_str_radix(&reference[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if reference.starts_with('#') => reference[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32),
            _ => None,
        };

        if let Some(character) = decoded {
            output.push(character);
            cursor = end + 1;
        } else {
            output.push_str(&input[start..=end]);
            cursor = end + 1;
        }
    }

    output.push_str(&input[cursor..]);
    output
}

#[cfg(test)]
mod character_reference_tests {
    use super::*;

    #[test]
    fn decodes_basic_character_references() {
        assert_eq!(
            decode_character_references("&lt;div&gt; &amp; &#65; &#x1f600;"),
            "<div> & A 😀"
        );
    }

    #[test]
    fn leaves_unknown_references_untouched() {
        assert_eq!(decode_character_references("&unknown;"), "&unknown;");
    }

    #[test]
    fn preserves_raw_script_text() {
        let tokens = HtmlTokenizer::tokenize("<script>if (a < b) { c++; }</script>");
        assert!(matches!(
            tokens.get(1),
            Some(HtmlToken::Text(value)) if value.contains("a < b")
        ));
    }
}
