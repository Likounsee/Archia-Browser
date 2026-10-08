use crate::css::StyleSheet;
use crate::html::{parse, HtmlTokenizer, Node};
use crate::layout::{LayoutEngine, LayoutNode, LayoutViewport};
use crate::net::Url;
use crate::render::{DisplayList, SoftwareRenderer};
use crate::style_tree::{StyleEngine, StyledNode};

#[derive(Debug, Clone)]
pub struct Page {
    url: Option<Url>,
    pub document: Node,
    pub styled: StyledNode,
    pub layout: LayoutNode,
    pub display_list: DisplayList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSubmission {
    pub method: FormMethod,
    pub url: Url,
    pub body: Vec<u8>,
    pub content_type: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormSubmissionError {
    NotForm,
    MissingDocumentUrl,
    UnsupportedMethod,
    UnsupportedEncoding,
    Url(crate::net::UrlError),
}

impl From<crate::net::UrlError> for FormSubmissionError {
    fn from(error: crate::net::UrlError) -> Self {
        Self::Url(error)
    }
}

impl Page {
    pub fn from_html(html: &str, css: &str, viewport: LayoutViewport) -> Self {
        Self::from_html_at(None, html, css, viewport)
    }

    pub fn from_html_at(url: Option<Url>, html: &str, css: &str, viewport: LayoutViewport) -> Self {
        let tokens = HtmlTokenizer::tokenize(html);
        let document = parse(&tokens);
        let stylesheet = StyleSheet::parse(css);
        let styled = StyleEngine::style(&document, &stylesheet);
        let layout = LayoutEngine::layout_styled(&styled, viewport);
        let display_list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        Self {
            url,
            document,
            styled,
            layout,
            display_list,
        }
    }

    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    pub fn base_url(&self) -> Option<Url> {
        let document_url = self.url.clone()?;
        let href = self
            .document
            .find_first_element("base")?
            .attribute("href")?
            .trim();
        if href.is_empty() {
            return None;
        }

        document_url.resolve(href).ok()
    }

    pub fn resolve_reference(&self, reference: &str) -> Result<Url, crate::net::UrlError> {
        let base = self
            .base_url()
            .or_else(|| self.url.clone())
            .ok_or(crate::net::UrlError::MissingAuthority)?;
        base.resolve(reference)
    }

    pub fn form_submission(&self, form: &Node) -> Result<FormSubmission, FormSubmissionError> {
        if form.tag_name() != Some("form") {
            return Err(FormSubmissionError::NotForm);
        }

        let action = form.attribute("action").unwrap_or("").trim();
        let url = self.resolve_reference(action)?;
        let method = match form
            .attribute("method")
            .unwrap_or("get")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "get" => FormMethod::Get,
            "post" => FormMethod::Post,
            _ => return Err(FormSubmissionError::UnsupportedMethod),
        };

        let encoding = form
            .attribute("enctype")
            .unwrap_or("application/x-www-form-urlencoded")
            .trim()
            .to_ascii_lowercase();
        if encoding != "application/x-www-form-urlencoded" {
            return Err(FormSubmissionError::UnsupportedEncoding);
        }

        let mut entries = Vec::new();
        collect_form_entries(form, &mut entries);
        let encoded = encode_form_entries(&entries);

        match method {
            FormMethod::Get => Ok(FormSubmission {
                method,
                url: append_query(&url, &encoded)?,
                body: Vec::new(),
                content_type: None,
            }),
            FormMethod::Post => Ok(FormSubmission {
                method,
                url,
                body: encoded.into_bytes(),
                content_type: Some("application/x-www-form-urlencoded"),
            }),
        }
    }

    pub fn title(&self) -> Option<String> {
        let title = self.document.find_first_element("title")?.text_content();
        let title = title.trim();
        (!title.is_empty()).then(|| title.to_owned())
    }

    pub fn render_into(&self, surface: &mut crate::surface::SoftwareSurface) {
        SoftwareRenderer::rasterize(&self.display_list, surface);
    }
}

fn collect_form_entries(node: &Node, entries: &mut Vec<(String, String)>) {
    if let Some(tag) = node.tag_name() {
        match tag {
            "input" => {
                let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) else {
                    return;
                };
                if node.has_attribute("disabled") {
                    return;
                }

                let input_type = node
                    .attribute("type")
                    .unwrap_or("text")
                    .trim()
                    .to_ascii_lowercase();
                if matches!(
                    input_type.as_str(),
                    "button" | "reset" | "submit" | "image" | "file"
                ) {
                    return;
                }
                if matches!(input_type.as_str(), "checkbox" | "radio")
                    && !node.has_attribute("checked")
                {
                    return;
                }

                let value = node
                    .attribute("value")
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        if matches!(input_type.as_str(), "checkbox" | "radio") {
                            "on".to_owned()
                        } else {
                            String::new()
                        }
                    });
                entries.push((name.to_owned(), value));
            }
            "textarea" => {
                if node.has_attribute("disabled") {
                    return;
                }
                if let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) {
                    entries.push((name.to_owned(), node.text_content()));
                }
            }
            _ => {}
        }
    }

    for child in node.children() {
        collect_form_entries(child, entries);
    }
}

fn encode_form_entries(entries: &[(String, String)]) -> String {
    entries
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                encode_form_component(name),
                encode_form_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_form_component(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'*' => {
                output.push(*byte as char)
            }
            b' ' => output.push('+'),
            byte => {
                output.push('%');
                output.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
                output.push(char::from(b"0123456789ABCDEF"[(byte & 0x0f) as usize]));
            }
        }
    }
    output
}

fn append_query(url: &Url, encoded: &str) -> Result<Url, crate::net::UrlError> {
    let mut target = format!("{}://{}{}", url.scheme(), url.authority(), url.path());
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
        if !query.is_empty() && !encoded.is_empty() {
            target.push('&');
        }
    } else if !encoded.is_empty() {
        target.push('?');
    }
    target.push_str(encoded);
    Url::parse(&target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_static_html_css_pipeline() {
        let page = Page::from_html(
            r#"<body><div style="background-color: #102030">Hello</div></body>"#,
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(page.document.text_content(), "Hello");
        assert!(!page.display_list.commands().is_empty());
        assert_eq!(page.layout.rect.width, 320);
    }

    #[test]
    fn css_styles_reach_render_tree() {
        let page = Page::from_html(
            r#"<body><div class="hero">Hello</div></body>"#,
            ".hero { background-color: red; }",
            LayoutViewport::new(100, 100),
        );

        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::FillRect {
                    color: 0xff0000ff,
                    ..
                }
            )
        }));
    }

    #[test]
    #[test]
    fn builds_get_form_submission_from_successful_controls() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/search").unwrap()),
            r#"<form action="/find" method="get"><input name="q" value="hello world"><input name="skip" disabled value="x"><input name="flag" type="checkbox" value="yes" checked><textarea name="note">line one</textarea><input type="submit" value="Go"></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let submission = page.form_submission(form).unwrap();
        assert_eq!(submission.method, FormMethod::Get);
        assert_eq!(
            submission.url.to_string(),
            "https://example.org/find?q=hello+world&flag=yes&note=line+one"
        );
        assert!(submission.body.is_empty());
        assert_eq!(submission.content_type, None);
    }

    #[test]
    fn builds_post_form_submission() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/form").unwrap()),
            r#"<form method="post"><input name="q" value="rust"></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let submission = page.form_submission(form).unwrap();
        assert_eq!(submission.method, FormMethod::Post);
        assert_eq!(submission.url.to_string(), "https://example.org/form");
        assert_eq!(submission.body, b"q=rust");
        assert_eq!(
            submission.content_type,
            Some("application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn resolves_links_against_document_base_element() {
        let url = Url::parse("https://example.org/docs/index.html").unwrap();
        let page = Page::from_html_at(
            Some(url),
            r#"<head><base href="/guide/"></head><body><a href="chapter.html">Next</a></body>"#,
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(
            page.base_url().map(|value| value.to_string()),
            Some("https://example.org/guide/".to_owned())
        );
        assert_eq!(
            page.resolve_reference("chapter.html").unwrap().to_string(),
            "https://example.org/guide/chapter.html"
        );
    }

    #[test]
    fn invalid_base_href_falls_back_to_document_url() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/docs/index.html").unwrap()),
            r#"<head><base href="javascript:bad"></head><body>Hello</body>"#,
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(
            page.resolve_reference("next.html").unwrap().to_string(),
            "https://example.org/docs/next.html"
        );
    }

    #[test]
    fn retains_document_url_when_loaded_at_a_navigation_target() {
        let url = Url::parse("https://example.org/docs/index.html").unwrap();
        let page = Page::from_html_at(
            Some(url.clone()),
            "<title>Example</title><body>Hello</body>",
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(page.url(), Some(&url));
    }

    fn extracts_trimmed_document_title() {
        let page = Page::from_html(
            "<html><head><title>  Archia Browser  </title></head><body>Hello</body></html>",
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(page.title(), Some("Archia Browser".to_owned()));
    }

    #[test]
    fn empty_or_missing_title_is_none() {
        let missing = Page::from_html("<body>Hello</body>", "", LayoutViewport::new(320, 200));
        let empty = Page::from_html(
            "<head><title>   </title></head><body>Hello</body>",
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(missing.title(), None);
        assert_eq!(empty.title(), None);
    }
}
