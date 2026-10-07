#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlToken {
    Doctype(String),
    StartTag { name: String, self_closing: bool },
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

            let Some(offset) = input[cursor..].find('>') else {
                tokens.push(HtmlToken::Text(input[cursor..].to_owned()));
                break;
            };

            let end = cursor + offset;
            let inside = input[cursor + 1..end].trim();

            if let Some(doctype) = inside.strip_prefix("!DOCTYPE").or_else(|| inside.strip_prefix("!doctype")) {
                tokens.push(HtmlToken::Doctype(doctype.trim().to_owned()));
            } else if let Some(name) = inside.strip_prefix('/') {
                tokens.push(HtmlToken::EndTag(name.trim().to_ascii_lowercase()));
            } else {
                let self_closing = inside.ends_with('/');
                let name = inside.trim_end_matches('/').split_whitespace().next().unwrap_or("");
                if !name.is_empty() {
                    tokens.push(HtmlToken::StartTag {
                        name: name.to_ascii_lowercase(),
                        self_closing,
                    });
                }
            }
            cursor = end + 1;
        }

        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_basic_document() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        assert_eq!(tokens, vec![
            HtmlToken::StartTag { name: "html".into(), self_closing: false },
            HtmlToken::StartTag { name: "body".into(), self_closing: false },
            HtmlToken::Text("Hello".into()),
            HtmlToken::EndTag("body".into()),
            HtmlToken::EndTag("html".into()),
        ]);
    }

    #[test]
    fn tokenizes_comments_and_doctype() {
        let tokens = HtmlTokenizer::tokenize("<!DOCTYPE html><!--x-->");
        assert_eq!(tokens[0], HtmlToken::Doctype("html".into()));
        assert_eq!(tokens[1], HtmlToken::Comment("x".into()));
    }
}
