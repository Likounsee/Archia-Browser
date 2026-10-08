use crate::css::StyleSheet;
use crate::html::{parse, HtmlTokenizer, Node};
use crate::layout::{LayoutEngine, LayoutNode, LayoutViewport};
use crate::net::Url;
use crate::render::{DisplayList, SoftwareRenderer};
use crate::style_tree::{StyleEngine, StyledNode};

const USER_AGENT_STYLESHEET: &str = r#"
html, body { display: block; }
body { margin: 8px; }
h1 { display: block; font-size: 32px; font-weight: bold; margin-top: 21px; margin-bottom: 21px; }
h2 { display: block; font-size: 24px; font-weight: bold; margin-top: 19px; margin-bottom: 19px; }
h3 { display: block; font-size: 19px; font-weight: bold; margin-top: 18px; margin-bottom: 18px; }
h4 { display: block; font-size: 16px; font-weight: bold; margin-top: 21px; margin-bottom: 21px; }
h5 { display: block; font-size: 13px; font-weight: bold; margin-top: 21px; margin-bottom: 21px; }
h6 { display: block; font-size: 11px; font-weight: bold; margin-top: 21px; margin-bottom: 21px; }
p { display: block; margin-top: 16px; margin-bottom: 16px; }
blockquote { display: block; margin-top: 16px; margin-bottom: 16px; margin-left: 40px; margin-right: 40px; }
ul, ol { display: block; margin-top: 16px; margin-bottom: 16px; padding-left: 40px; }
li { display: block; }
strong, b { font-weight: bold; }
em, i { font-style: italic; }
pre { display: block; white-space: pre; }
code { font-family: monospace; }
a { color: #0000ee; }
"#;

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

impl FormSubmission {
    pub fn into_request(self) -> crate::net::Request {
        let mut request = crate::net::Request::new(self.url);
        match self.method {
            FormMethod::Get => request,
            FormMethod::Post => {
                request.method = crate::net::HttpMethod::Post;
                if let Some(content_type) = self.content_type {
                    request
                        .headers
                        .insert("content-type".to_owned(), content_type.to_owned());
                }
                request.body = self.body;
                request
            }
        }
    }
}

impl Page {
    pub fn from_html(html: &str, css: &str, viewport: LayoutViewport) -> Self {
        Self::from_html_at(None, html, css, viewport)
    }

    pub fn from_html_at(url: Option<Url>, html: &str, css: &str, viewport: LayoutViewport) -> Self {
        let tokens = HtmlTokenizer::tokenize(html);
        let document = parse(&tokens);
        let mut stylesheet_input =
            String::with_capacity(USER_AGENT_STYLESHEET.len() + css.len() + 32);
        stylesheet_input.push_str(USER_AGENT_STYLESHEET);
        stylesheet_input.push('\n');
        stylesheet_input.push_str(css);
        stylesheet_input.push('\n');
        collect_inline_styles(&document, &mut stylesheet_input);
        let stylesheet = StyleSheet::parse(&stylesheet_input);
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
        surface.clear(crate::surface::Color::WHITE);
        SoftwareRenderer::rasterize(&self.display_list, surface);
    }
}

fn collect_inline_styles(node: &Node, css: &mut String) {
    if node.tag_name() == Some("style") {
        css.push('\n');
        css.push_str(&node.text_content());
        css.push('\n');
    }
    for child in node.children() {
        collect_inline_styles(child, css);
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
                entries.push((name.to_owned(), normalize_form_newlines(&value)));
            }
            "textarea" => {
                if node.has_attribute("disabled") {
                    return;
                }
                if let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) {
                    entries.push((
                        name.to_owned(),
                        normalize_form_newlines(&node.text_content()),
                    ));
                }
            }
            "select" => collect_select_entries(node, entries),
            _ => {}
        }
    }

    for child in node.children() {
        collect_form_entries(child, entries);
    }
}

fn collect_select_entries(node: &Node, entries: &mut Vec<(String, String)>) {
    if node.has_attribute("disabled") {
        return;
    }
    let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) else {
        return;
    };
    let multiple = node.has_attribute("multiple");
    let mut options = Vec::new();
    collect_options(node, &mut options);
    let selected = options.iter().filter(|option| option.selected).count();
    for option in options {
        if option.disabled || (!option.selected && (selected > 0 || multiple)) {
            continue;
        }
        entries.push((name.to_owned(), normalize_form_newlines(&option.value)));
        if !multiple {
            break;
        }
    }
}

#[derive(Debug)]
struct FormOption {
    value: String,
    selected: bool,
    disabled: bool,
}

fn collect_options(node: &Node, options: &mut Vec<FormOption>) {
    for child in node.children() {
        if child.tag_name() == Some("option") {
            options.push(FormOption {
                value: child
                    .attribute("value")
                    .map(str::to_owned)
                    .unwrap_or_else(|| child.text_content()),
                selected: child.has_attribute("selected"),
                disabled: child.has_attribute("disabled"),
            });
        } else {
            collect_options(child, options);
        }
    }
}

fn normalize_form_newlines(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                output.push_str("\r\n");
            }
            '\n' => output.push_str("\r\n"),
            _ => output.push(character),
        }
    }
    output
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
    fn applies_basic_user_agent_styles_to_local_document_markup() {
        let page = Page::from_html(
            "<html><body><h1>Title</h1><p>Text</p></body></html>",
            "",
            LayoutViewport::new(320, 200),
        );

        let body = &page.styled.children[0].children[1];
        assert_eq!(body.style.get("margin-top"), Some("8px"));
        let heading = &body.children[0];
        assert_eq!(heading.style.get("font-size"), Some("32px"));
        assert_eq!(heading.style.get("font-weight"), Some("bold"));
        assert_eq!(heading.style.get("margin-top"), Some("21px"));
    }

    #[test]
    fn author_styles_override_basic_user_agent_styles() {
        let page = Page::from_html(
            "<html><body><h1>Title</h1></body></html>",
            "body { margin: 0; } h1 { font-size: 20px; margin: 0; }",
            LayoutViewport::new(320, 200),
        );

        let body = &page.styled.children[0].children[1];
        let heading = &body.children[0];
        assert_eq!(body.style.get("margin-top"), Some("0"));
        assert_eq!(heading.style.get("font-size"), Some("20px"));
        assert_eq!(heading.style.get("margin-top"), Some("0"));
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
    fn selects_successful_options_and_normalizes_textarea_newlines() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/form").unwrap()),
            r#"<form method="post"><select name="choice"><option value="a">A</option><option value="b" selected>B</option></select><select name="tags" multiple><option value="one" selected>One</option><option value="two" selected>Two</option></select><textarea name="note">a
b</textarea></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let submission = page.form_submission(form).unwrap();
        assert_eq!(submission.body, b"choice=b&tags=one&tags=two&note=a%0D%0Ab");
    }

    #[test]
    fn converts_post_form_submission_into_request() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/form").unwrap()),
            r#"<form method="post"><input name="q" value="rust"></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let request = page.form_submission(form).unwrap().into_request();
        assert_eq!(request.method, crate::net::HttpMethod::Post);
        assert_eq!(request.url.to_string(), "https://example.org/form");
        assert_eq!(
            request.header("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(request.body, b"q=rust");
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
    fn inline_style_elements_reach_the_css_engine() {
        let page = Page::from_html(
            r#"<style>.card { color: red; }</style><body><span class="card">Hello</span></body>"#,
            "",
            LayoutViewport::new(100, 50),
        );

        fn find_card(
            node: &crate::style_tree::StyledNode,
        ) -> Option<&crate::style_tree::StyledNode> {
            if node.node.tag_name() == Some("span") {
                return Some(node);
            }
            node.children.iter().find_map(find_card)
        }

        assert_eq!(
            find_card(&page.styled).and_then(|node| node.style.get("color")),
            Some("red")
        );
    }

    #[test]
    fn render_into_clears_previous_surface_contents() {
        let page = Page::from_html(
            "<body><div style=\"background-color: red; width: 4px; height: 4px;\"></div></body>",
            "",
            LayoutViewport::new(20, 20),
        );
        let mut surface = crate::surface::SoftwareSurface::new(20, 20);
        surface.clear(crate::surface::Color::RED);
        page.render_into(&mut surface);

        assert_eq!(surface.pixel(19, 19), Some(crate::surface::Color::WHITE));
        assert_eq!(surface.pixel(8, 8), Some(crate::surface::Color::RED));
        assert_eq!(surface.pixel(0, 0), Some(crate::surface::Color::WHITE));
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
