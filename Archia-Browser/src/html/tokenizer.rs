use std::collections::BTreeMap;

// Bound scanning and retained token data for untrusted documents.
const MAX_HTML_INPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_HTML_TOKENS: usize = 100_000;

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
        // Truncate oversized documents only at a UTF-8 character boundary.
        let input = if input.len() > MAX_HTML_INPUT_BYTES {
            let mut end = MAX_HTML_INPUT_BYTES;
            while !input.is_char_boundary(end) {
                end -= 1;
            }
            &input[..end]
        } else {
            input
        };
        let mut tokens = Vec::new();
        let mut cursor = 0;
        let bytes = input.as_bytes();

        while cursor < bytes.len() && tokens.len() < MAX_HTML_TOKENS {
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

            // HTML is untrusted input. Do not slice at an arbitrary byte
            // offset before checking the prefix: a malformed tag can start
            // with multi-byte UTF-8 and otherwise panic the tokenizer.
            if inside
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("!doctype"))
            {
                tokens.push(HtmlToken::Doctype(inside[8..].trim().to_owned()));
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
                    if let Some((text_end, close_end)) = find_raw_text_end(input, end + 1, &name) {
                        if text_end > end + 1 && tokens.len() < MAX_HTML_TOKENS {
                            tokens.push(HtmlToken::Text(input[end + 1..text_end].to_owned()));
                        }
                        cursor = close_end;
                        continue;
                    }

                    // Script/style contents remain raw text through EOF when
                    // the closing tag is missing. Re-tokenizing the remainder
                    // as markup can create a parser differential on malformed
                    // or attacker-controlled documents.
                    if end + 1 < input.len() && tokens.len() < MAX_HTML_TOKENS {
                        tokens.push(HtmlToken::Text(input[end + 1..].to_owned()));
                    }
                    break;
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
                // Slashes are valid inside unquoted attribute values
                // (for example, href=https://example.test/path). A slash
                // only marks a self-closing tag when encountered between
                // attributes, not while consuming a value.
                while let Some(&ch) = chars.peek() {
                    if ch.is_whitespace() {
                        break;
                    }
                    value.push(ch);
                    chars.next();
                }
            }
        }
        // HTML ignores duplicate attributes after the first occurrence.
        // Keeping the first value avoids parser differentials where policy
        // checks and later DOM consumers could interpret different values.
        attributes
            .entry(attr_name)
            .or_insert_with(|| decode_character_references(&value));
    }
    Some((name, attributes, self_closing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_unicode_tag_does_not_panic() {
        let tokens = HtmlTokenizer::tokenize("<ééééé>");
        assert!(matches!(tokens.as_slice(), [HtmlToken::StartTag { name, .. }] if name == "ééééé"));
    }

    #[test]
    fn oversized_input_is_truncated_at_a_utf8_boundary() {
        let mut input = "a".repeat(MAX_HTML_INPUT_BYTES);
        input.push('é');
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), 1);
        let HtmlToken::Text(text) = &tokens[0] else {
            panic!("expected text token");
        };
        assert_eq!(text.len(), MAX_HTML_INPUT_BYTES);
    }

    #[test]
    fn token_count_is_bounded_for_many_small_tags() {
        let input = "<b></b>".repeat(MAX_HTML_TOKENS);
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), MAX_HTML_TOKENS);
    }

    #[test]
    fn raw_text_cannot_push_token_count_over_limit() {
        let input = format!("{}x<script>payload</script>", "<b></b>".repeat((MAX_HTML_TOKENS - 1) / 2));
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), MAX_HTML_TOKENS);
    }

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
    fn keeps_first_duplicate_attribute_value() {
        let tokens = HtmlTokenizer::tokenize(
            r#"<a href="https://safe.example/" href="https://other.example/">link</a>"#,
        );
        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected anchor start tag");
        };
        assert_eq!(
            attributes.get("href").map(String::as_str),
            Some("https://safe.example/")
        );
    }

    #[test]
    fn preserves_slashes_in_unquoted_attribute_values() {
        let tokens = HtmlTokenizer::tokenize(
            "<a href=https://example.test/path>link</a><img src=/assets/icon.svg>",
        );
        let HtmlToken::StartTag {
            attributes: link, ..
        } = &tokens[0]
        else {
            panic!("expected anchor start tag");
        };
        assert_eq!(
            link.get("href").map(String::as_str),
            Some("https://example.test/path")
        );
        let HtmlToken::StartTag {
            attributes: image, ..
        } = &tokens[3]
        else {
            panic!("expected image start tag");
        };
        assert_eq!(
            image.get("src").map(String::as_str),
            Some("/assets/icon.svg")
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
    let mut search_from = 0;

    while let Some(relative) = lower[search_from..].find(&marker) {
        let relative = search_from + relative;
        let text_end = start + relative;
        let after_name = text_end + marker.len();
        let next = input.as_bytes().get(after_name).copied();

        // A raw-text end tag must end its name here. Without this boundary
        // check, </scripture> would incorrectly terminate a <script> block.
        if next.is_some_and(|byte| byte == b'>' || byte == b'/' || byte.is_ascii_whitespace()) {
            if let Some(offset) = input[after_name..].find('>') {
                return Some((text_end, after_name + offset + 1));
            }
            return None;
        }

        search_from = relative + marker.len();
        if search_from >= lower.len() {
            return None;
        }
    }
    None
}

fn decode_numeric_character_reference(codepoint: u32) -> char {
    // HTML replaces null and invalid Unicode scalar values in numeric
    // character references rather than preserving the source spelling.
    if codepoint == 0 || char::from_u32(codepoint).is_none() {
        return '\u{FFFD}';
    }

    // HTML's numeric-reference algorithm remaps these legacy Windows-1252
    // control values to their printable characters.
    const WINDOWS_1252_REPLACEMENTS: &[(u32, char)] = &[
        (0x80, '€'),
        (0x82, '‚'),
        (0x83, 'ƒ'),
        (0x84, '„'),
        (0x85, '…'),
        (0x86, '†'),
        (0x87, '‡'),
        (0x88, 'ˆ'),
        (0x89, '‰'),
        (0x8A, 'Š'),
        (0x8B, '‹'),
        (0x8C, 'Œ'),
        (0x8E, 'Ž'),
        (0x91, '‘'),
        (0x92, '’'),
        (0x93, '“'),
        (0x94, '”'),
        (0x95, '•'),
        (0x96, '–'),
        (0x97, '—'),
        (0x98, '˜'),
        (0x99, '™'),
        (0x9A, 'š'),
        (0x9B, '›'),
        (0x9C, 'œ'),
        (0x9E, 'ž'),
        (0x9F, 'Ÿ'),
    ];
    if let Some((_, replacement)) = WINDOWS_1252_REPLACEMENTS
        .iter()
        .find(|(value, _)| *value == codepoint)
    {
        return *replacement;
    }

    // The remaining C1 controls are retained by the HTML algorithm.
    char::from_u32(codepoint).unwrap_or('\u{FFFD}')
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
                    .map(decode_numeric_character_reference)
            }
            _ if reference.starts_with('#') => reference[1..]
                .parse::<u32>()
                .ok()
                .map(decode_numeric_character_reference),
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
    fn replaces_invalid_numeric_character_references() {
        assert_eq!(
            decode_character_references("&#0; &#xD800; &#x110000;"),
            "\u{FFFD} \u{FFFD} \u{FFFD}"
        );
    }

    #[test]
    fn maps_legacy_numeric_control_references() {
        assert_eq!(decode_character_references("&#x80; &#x91;"), "€ ‘");
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

    #[test]
    fn unterminated_raw_text_element_consumes_remainder_as_text() {
        let tokens = HtmlTokenizer::tokenize("<script>if (a < b) { const x = '<p>not markup</p>';");

        assert!(matches!(
            tokens.as_slice(),
            [
                HtmlToken::StartTag { name, .. },
                HtmlToken::Text(value)
            ] if name == "script" && value.contains("<p>not markup</p>")
        ));
    }

    #[test]
    fn raw_script_does_not_close_on_longer_tag_name_prefix() {
        let tokens = HtmlTokenizer::tokenize(
            "<script>const x = '</scripture>'; run();</script><p>after</p>",
        );
        assert!(matches!(
            tokens.get(1),
            Some(HtmlToken::Text(value)) if value.contains("</scripture>") && value.contains("run();")
        ));
        assert!(matches!(
            tokens.get(2),
            Some(HtmlToken::StartTag { name, .. }) if name == "p"
        ));
    }
}
