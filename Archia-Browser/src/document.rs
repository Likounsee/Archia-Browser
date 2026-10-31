use crate::core::memory::{MemoryBudget, MemoryReservation};
use crate::css::StyleSheet;
use crate::html::{parse, HtmlTokenizer, Node};
use crate::layout::{LayoutEngine, LayoutNode, LayoutViewport};
use crate::net::Url;
use crate::render::{DisplayList, SoftwareRenderer};
use crate::style_tree::{StyleEngine, StyledNode};
use std::sync::Arc;

const USER_AGENT_STYLESHEET: &str = r#"
html, body { display: block; }
body { margin: 0; }
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
img { display: inline-block; }
button, input, select, textarea { display: inline-block; }
input, select { min-height: 20px; }
button { padding: 2px 6px; }
hr { display: block; border-top: 1px; margin-top: 8px; margin-bottom: 8px; }
"#;

#[derive(Debug)]
pub struct Page {
    url: Option<Url>,
    _memory_reservation: Option<Arc<MemoryReservation>>,
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
            _memory_reservation: None,
            document,
            styled,
            layout,
            display_list,
        }
    }

    /// Construct a page under a shared memory budget.
    ///
    /// The input-based estimate is reserved before tokenization, DOM construction,
    /// style calculation, layout, and display-list creation. After construction the
    /// reservation is reconciled with the retained-structure estimate. The estimate
    /// is deliberately conservative but remains an estimate, not allocator telemetry.
    pub fn try_from_html_at_with_budget(
        url: Option<Url>,
        html: &str,
        css: &str,
        viewport: LayoutViewport,
        budget: Arc<MemoryBudget>,
    ) -> Option<Self> {
        const HTML_EXPANSION_FACTOR: usize = 12;
        const CSS_EXPANSION_FACTOR: usize = 6;

        let html_charge = html.len().checked_mul(HTML_EXPANSION_FACTOR)?;
        let css_charge = css.len().checked_mul(CSS_EXPANSION_FACTOR)?;
        let initial_charge = html_charge.checked_add(css_charge)?.max(1);
        let reservation = MemoryReservation::try_new(Arc::clone(&budget), initial_charge)?;
        Self::try_from_html_at_with_reservation(url, html, css, viewport, reservation)
    }

    /// Build a page using a reservation already held by the caller.
    ///
    /// The caller can keep the reservation alive while decoding input and loading
    /// linked stylesheets, then transfer it here so the retained page structures
    /// continue to be charged without a gap or a duplicate reservation.
    pub(crate) fn try_from_html_at_with_reservation(
        url: Option<Url>,
        html: &str,
        css: &str,
        viewport: LayoutViewport,
        reservation: MemoryReservation,
    ) -> Option<Self> {
        let mut page = Self::from_html_at(url, html, css, viewport);
        let retained = page.retained_structure_estimate();
        if retained > reservation.bytes() {
            if !reservation.try_grow_by(retained - reservation.bytes()) {
                return None;
            }
        } else {
            reservation.shrink_to(retained);
        }
        page.attach_memory_reservation(Arc::new(reservation));
        Some(page)
    }

    /// Recompute layout and paint commands for a resized viewport without reparsing HTML/CSS.
    pub fn relayout(&mut self, viewport: LayoutViewport) {
        self.layout = LayoutEngine::layout_styled(&self.styled, viewport);
        self.display_list = SoftwareRenderer::build_display_list_styled(&self.styled, &self.layout);
    }

    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    /// Retain the memory reservation for this page's document representation.
    pub(crate) fn attach_memory_reservation(&mut self, reservation: Arc<MemoryReservation>) {
        self._memory_reservation = Some(reservation);
    }

    /// Conservative lower bound for the heap retained by the document's structural trees.
    /// Estimates retained DOM, computed-style, layout, and display-list storage.
    /// This is a conservative accounting estimate, not an allocator-exact measurement.
    pub(crate) fn retained_structure_estimate(&self) -> usize {
        // Count heap-owned buffers recursively, but count each tree node's inline
        // storage only through its containing Vec capacity. Adding size_of::<Node>()
        // at every recursion level would count child elements twice.
        fn node_owned_bytes(node: &Node) -> usize {
            let mut bytes = node
                .children
                .capacity()
                .saturating_mul(std::mem::size_of::<Node>());
            match &node.kind {
                crate::html::NodeKind::Document => {}
                crate::html::NodeKind::Element { name, attributes } => {
                    bytes = bytes.saturating_add(name.capacity());
                    for (key, value) in attributes {
                        // BTreeMap/attribute entry metadata is an estimate; allocator
                        // overhead and fragmentation are deliberately not claimed exact.
                        bytes = bytes
                            .saturating_add(std::mem::size_of::<(String, String)>())
                            .saturating_add(key.capacity())
                            .saturating_add(value.capacity());
                    }
                }
                crate::html::NodeKind::Text(value) | crate::html::NodeKind::Comment(value) => {
                    bytes = bytes.saturating_add(value.capacity());
                }
            }
            for child in &node.children {
                bytes = bytes.saturating_add(node_owned_bytes(child));
            }
            bytes
        }

        fn styled_owned_bytes(node: &StyledNode) -> usize {
            let mut bytes = node
                .children
                .capacity()
                .saturating_mul(std::mem::size_of::<StyledNode>());
            // Styled nodes own shallow DOM-node copies plus two BTreeMaps of
            // computed property values/importance flags. Include string lengths
            // and approximate per-entry metadata for both maps.
            bytes = bytes.saturating_add(node_owned_bytes(&node.node));
            for (name, value) in node.style.iter() {
                // Computed style entries borrow property names and values; account for
                // string contents and map-entry metadata without claiming ownership.
                bytes = bytes
                    .saturating_add(std::mem::size_of::<(&str, &str)>())
                    .saturating_add(name.len())
                    .saturating_add(value.len());
            }
            let importance_name_bytes = node
                .style
                .iter()
                .fold(0usize, |total, (name, _)| total.saturating_add(name.len()));
            bytes = bytes.saturating_add(
                node.style
                    .len()
                    .saturating_mul(std::mem::size_of::<(&str, bool)>())
                    .saturating_add(importance_name_bytes),
            );
            for child in &node.children {
                bytes = bytes.saturating_add(styled_owned_bytes(child));
            }
            bytes
        }

        fn layout_owned_bytes(node: &LayoutNode) -> usize {
            let mut bytes = node
                .children
                .capacity()
                .saturating_mul(std::mem::size_of::<LayoutNode>());
            for child in &node.children {
                bytes = bytes.saturating_add(layout_owned_bytes(child));
            }
            bytes
        }

        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(std::mem::size_of::<Node>())
            .saturating_add(node_owned_bytes(&self.document))
            .saturating_add(std::mem::size_of::<StyledNode>())
            .saturating_add(styled_owned_bytes(&self.styled))
            .saturating_add(std::mem::size_of::<LayoutNode>())
            .saturating_add(layout_owned_bytes(&self.layout))
            .saturating_add(
                self.display_list
                    .capacity()
                    .saturating_mul(std::mem::size_of::<crate::render::PaintCommand>()),
            );

        for command in self.display_list.commands() {
            if let crate::render::PaintCommand::DrawText { text, .. } = command {
                bytes = bytes.saturating_add(text.capacity());
            }
        }
        bytes.max(1)
    }

    /// Deep-copy a page while charging the retained copy to a specific budget.
    ///
    /// Unlike `try_clone`, this also accounts for pages created without a loader
    /// reservation and for pages originating from a different browser budget.
    pub(crate) fn try_clone_with_budget(
        &self,
        budget: Arc<crate::core::memory::MemoryBudget>,
    ) -> Option<Self> {
        let reservation = MemoryReservation::try_new(budget, self.retained_structure_estimate())?;
        let mut page = Self {
            url: self.url.clone(),
            _memory_reservation: None,
            document: self.document.clone(),
            styled: self.styled.clone(),
            layout: self.layout.clone(),
            display_list: self.display_list.clone(),
        };
        page.attach_memory_reservation(Arc::new(reservation));
        Some(page)
    }

    /// Deep-copy a page only after reserving a separate charge for its retained
    /// document, style tree, layout tree, and display list. Returns None when
    /// the shared budget cannot accommodate another copy.
    pub fn try_clone(&self) -> Option<Self> {
        let reservation = match &self._memory_reservation {
            Some(reservation) => Some(Arc::new(reservation.try_clone_reservation()?)),
            None => None,
        };

        Some(Self {
            url: self.url.clone(),
            _memory_reservation: reservation,
            document: self.document.clone(),
            styled: self.styled.clone(),
            layout: self.layout.clone(),
            display_list: self.display_list.clone(),
        })
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
    collect_form_entries_with_disabled_fieldset(node, entries, false);
}

fn collect_form_entries_with_disabled_fieldset(
    node: &Node,
    entries: &mut Vec<(String, String)>,
    disabled_by_fieldset: bool,
) {
    if !disabled_by_fieldset {
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
    }

    let disabled_fieldset = node.tag_name() == Some("fieldset") && node.has_attribute("disabled");
    let mut first_legend_seen = false;
    for child in node.children() {
        // HTML exempts descendants of a disabled fieldset's first direct legend.
        // Other descendants are disabled, even if they don't carry a disabled
        // attribute themselves.
        let is_first_legend =
            disabled_fieldset && child.tag_name() == Some("legend") && !first_legend_seen;
        if child.tag_name() == Some("legend") {
            first_legend_seen = true;
        }
        let child_disabled = disabled_by_fieldset || (disabled_fieldset && !is_first_legend);
        collect_form_entries_with_disabled_fieldset(child, entries, child_disabled);
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
    // Disabled options are not successful controls and must not prevent
    // the browser from choosing the first enabled option in a single-select.
    let selected = options
        .iter()
        .filter(|option| option.selected && !option.disabled)
        .count();
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
    collect_options_with_disabled_ancestor(node, false, options);
}

fn collect_options_with_disabled_ancestor(
    node: &Node,
    disabled_ancestor: bool,
    options: &mut Vec<FormOption>,
) {
    for child in node.children() {
        if child.tag_name() == Some("option") {
            options.push(FormOption {
                value: child
                    .attribute("value")
                    .map(str::to_owned)
                    .unwrap_or_else(|| child.text_content()),
                selected: child.has_attribute("selected"),
                disabled: disabled_ancestor || child.has_attribute("disabled"),
            });
        } else {
            // A disabled optgroup disables all of its option descendants.
            // Propagate this only through optgroups, not arbitrary disabled
            // containers, so unrelated markup cannot suppress form controls.
            let child_disabled = disabled_ancestor
                || (child.tag_name() == Some("optgroup") && child.has_attribute("disabled"));
            collect_options_with_disabled_ancestor(child, child_disabled, options);
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
    // A GET form updates the query component, not the fragment. Preserve the
    // fragment in the resulting navigation URL while keeping it after the query.
    if let Some(fragment) = url.fragment() {
        target.push('#');
        target.push_str(fragment);
    }
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
    fn relayout_updates_viewport_without_reparsing_document() {
        let mut page = Page::from_html(
            r#"<body><main style="background-color: #123456">Responsive content</main></body>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let original_document_text = page.document.text_content();
        assert_eq!(page.layout.rect.width, 320);

        page.relayout(LayoutViewport::new(960, 540));

        assert_eq!(page.layout.rect.width, 960);
        assert_eq!(page.document.text_content(), original_document_text);
        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::FillRect {
                    color: 0x123456ff,
                    ..
                }
            )
        }));
    }

    #[test]
    fn applies_basic_user_agent_styles_to_local_document_markup() {
        let page = Page::from_html(
            "<html><body><h1>Title</h1><p>Text</p></body></html>",
            "",
            LayoutViewport::new(320, 200),
        );

        let body = &page.styled.children[0].children[1];
        assert_eq!(body.style.get("margin-top"), Some("0"));
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
    fn disabled_fieldset_excludes_controls_except_first_legend() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/").unwrap()),
            r#"<form><fieldset disabled><input name="outside" value="no"><legend><input name="legend" value="yes"></legend><input name="after" value="no"></fieldset></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let submission = page.form_submission(form).unwrap();
        assert_eq!(submission.url.query(), Some("legend=yes"));
    }

    #[test]
    fn select_ignores_options_inside_disabled_optgroups() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/").unwrap()),
            r#"<form><select name="color"><optgroup disabled><option selected value="red">Red</option></optgroup><option value="blue">Blue</option></select></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let submission = page.form_submission(form).unwrap();
        assert_eq!(submission.url.query(), Some("color=blue"));
    }

    #[test]
    fn get_form_submission_preserves_existing_query_and_fragment() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/search").unwrap()),
            r#"<form action="/find?existing=1#results" method="get"><input name="q" value="rust"></form>"#,
            "",
            LayoutViewport::new(320, 200),
        );
        let form = page.document.find_first_element("form").unwrap();

        let submission = page.form_submission(form).unwrap();
        assert_eq!(
            submission.url.to_string(),
            "https://example.org/find?existing=1&q=rust#results"
        );
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

    #[test]
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
        assert_eq!(surface.pixel(0, 0), Some(crate::surface::Color::RED));
    }

    #[test]
    fn deep_page_copy_gets_an_independent_memory_reservation() {
        let budget = Arc::new(crate::core::memory::MemoryBudget::new(4096));
        let mut page = Page::from_html("<body>Hello</body>", "", LayoutViewport::new(20, 20));
        page.attach_memory_reservation(Arc::new(
            MemoryReservation::try_new(Arc::clone(&budget), 1024).unwrap(),
        ));

        let copy = page.try_clone().expect("second page should fit");
        assert_eq!(budget.used(), 2048);
        drop(copy);
        assert_eq!(budget.used(), 1024);
        drop(page);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn deep_page_copy_is_rejected_when_shared_budget_is_full() {
        let budget = Arc::new(crate::core::memory::MemoryBudget::new(1024));
        let mut page = Page::from_html("<body>Hello</body>", "", LayoutViewport::new(20, 20));
        page.attach_memory_reservation(Arc::new(
            MemoryReservation::try_new(Arc::clone(&budget), 1024).unwrap(),
        ));

        assert!(page.try_clone().is_none());
        assert_eq!(
            budget.used(),
            1024,
            "failed clone must not leak a reservation"
        );
    }

    #[test]
    fn retained_structure_estimate_tracks_dom_and_layout_growth() {
        let small = Page::from_html("<body><p>x</p></body>", "", LayoutViewport::new(320, 200));
        let large_html = format!(
            "<body>{}</body>",
            "<section class=\"card\"><p>content</p></section>".repeat(80)
        );
        let large = Page::from_html(
            &large_html,
            ".card { padding: 4px; }",
            LayoutViewport::new(320, 200),
        );

        assert!(
            large.retained_structure_estimate() > small.retained_structure_estimate(),
            "the structural memory estimate must reflect additional DOM and layout nodes"
        );
    }

    #[test]
    fn retained_structure_estimate_accounts_for_display_list_text_storage() {
        let short = Page::from_html("<body>x</body>", "", LayoutViewport::new(320, 200));
        let long_text = "long retained text ".repeat(256);
        let long = Page::from_html(
            &format!("<body>{long_text}</body>"),
            "",
            LayoutViewport::new(320, 200),
        );

        assert!(
            long.retained_structure_estimate() > short.retained_structure_estimate(),
            "display-list text buffers must contribute to retained memory estimates"
        );
    }

    #[test]
    fn retained_structure_estimate_accounts_for_computed_style_properties() {
        let html = "<body><div class=\"target\">content</div></body>";
        let plain = Page::from_html(html, "", LayoutViewport::new(320, 200));
        let styled = Page::from_html(
            html,
            ".target { color: red; background-color: blue; padding: 4px; margin: 2px; border-width: 1px; font-size: 18px; }",
            LayoutViewport::new(320, 200),
        );

        assert!(
            styled.retained_structure_estimate() > plain.retained_structure_estimate(),
            "computed property maps and their strings must contribute to the retained estimate"
        );
    }

    #[test]
    fn budgeted_page_construction_fails_before_build_when_budget_is_insufficient() {
        let budget = Arc::new(crate::core::memory::MemoryBudget::new(1));
        let page = Page::try_from_html_at_with_budget(
            None,
            "<body><p>bounded</p></body>",
            "",
            LayoutViewport::new(320, 200),
            Arc::clone(&budget),
        );

        assert!(page.is_none());
        assert_eq!(budget.used(), 0, "failed preflight must not leak budget");
    }

    #[test]
    fn budgeted_page_construction_releases_memory_after_repeated_lifecycles() {
        let budget = Arc::new(crate::core::memory::MemoryBudget::new(16 * 1024 * 1024));

        for _ in 0..1_000 {
            let page = Page::try_from_html_at_with_budget(
                None,
                "<body><p>bounded lifecycle</p></body>",
                "p { color: red; margin: 2px; }",
                LayoutViewport::new(320, 200),
                Arc::clone(&budget),
            )
            .expect("small page should fit in the shared budget");
            assert!(budget.used() > 0, "retained page must be charged");
            assert!(budget.used() <= budget.limit());
            drop(page);
            assert_eq!(budget.used(), 0, "page drop must release its reservation");
        }
    }

    #[test]
    fn budgeted_page_clone_keeps_original_and_copy_charges_independent() {
        let budget = Arc::new(crate::core::memory::MemoryBudget::new(2 * 1024 * 1024));
        let page = Page::try_from_html_at_with_budget(
            None,
            "<body><p>independent</p></body>",
            "",
            LayoutViewport::new(320, 200),
            Arc::clone(&budget),
        )
        .expect("page should fit");
        let original_charge = budget.used();
        let copy = page
            .try_clone()
            .expect("clone should reserve its own charge");

        assert!(budget.used() >= original_charge.saturating_mul(2));
        drop(copy);
        assert_eq!(budget.used(), original_charge);
        drop(page);
        assert_eq!(budget.used(), 0);
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
