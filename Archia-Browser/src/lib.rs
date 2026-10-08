pub mod core;
pub mod css;
pub mod document;
pub mod html;
pub mod layout;
pub mod net;
pub mod platform;
pub mod render;
pub mod security;
pub mod style_tree;
pub mod surface;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkActivationError {
    NotAnchor,
    MissingHref,
    Url(net::UrlError),
}

impl From<net::UrlError> for LinkActivationError {
    fn from(error: net::UrlError) -> Self {
        Self::Url(error)
    }
}

#[derive(Debug, Default)]
struct LocalFilePolicy;

impl net::pipeline::RequestPolicyEngine for LocalFilePolicy {
    fn decide(&self, request: &net::Request) -> net::pipeline::PolicyDecision {
        if request.url.scheme() == "file" {
            net::pipeline::PolicyDecision::Allow
        } else {
            net::pipeline::PolicyDecision::Block
        }
    }
}

pub struct Browser {
    version: &'static str,
    memory: core::memory::MemoryBudget,
    tabs: core::tabs::TabManager,
}

impl Browser {
    pub fn new() -> Self {
        let mut tabs = core::tabs::TabManager::new();
        tabs.open();

        Self {
            version: env!("CARGO_PKG_VERSION"),
            memory: core::memory::MemoryBudget::new(512 * 1024 * 1024),
            tabs,
        }
    }

    pub const fn version(&self) -> &'static str {
        self.version
    }

    pub fn memory(&self) -> &core::memory::MemoryBudget {
        &self.memory
    }

    pub fn tabs(&self) -> &core::tabs::TabManager {
        &self.tabs
    }

    pub fn history(&self) -> &core::navigation::NavigationHistory {
        self.tabs
            .active_tab()
            .expect("browser always has an active tab")
            .history()
    }

    pub fn reload<P, T>(
        &mut self,
        loader: &net::DocumentLoader<P, T>,
        viewport: crate::layout::LayoutViewport,
    ) -> Result<crate::document::Page, net::DocumentLoadError>
    where
        P: net::pipeline::RequestPolicyEngine,
        T: net::Transport,
    {
        let url = self
            .current_url()
            .cloned()
            .ok_or(net::DocumentLoadError::NoCurrentDocument)?;
        let request = net::Request::new(url);
        let page = loader.load(&request, viewport)?;
        let Some(url) = page.url().cloned() else {
            return Err(net::DocumentLoadError::NoCurrentDocument);
        };

        let entry = core::navigation::NavigationEntry::new(url, page.title());
        if !self
            .tabs
            .active_tab_mut()
            .is_some_and(|tab| tab.history_mut().replace_current(entry))
        {
            self.commit_page(&page);
        } else if let Some(tab) = self.tabs.active_tab_mut() {
            tab.set_page(page.clone());
        }
        Ok(page)
    }

    pub fn load_local_file_url(
        &mut self,
        url: &str,
        viewport: crate::layout::LayoutViewport,
    ) -> Result<crate::document::Page, crate::net::DocumentLoadError> {
        let url = net::Url::parse(url)
            .map_err(net::TransportError::InvalidUrl)
            .map_err(net::DocumentLoadError::Network)?;
        let request = net::Request::new(url);
        let loader = net::DocumentLoader::new(
            net::pipeline::NetworkPipeline::new(LocalFilePolicy),
            net::LocalFileTransport::new(),
        );
        self.load_request(&loader, &request, viewport)
    }

    pub fn load_request<P, T>(
        &mut self,
        loader: &net::DocumentLoader<P, T>,
        request: &net::Request,
        viewport: crate::layout::LayoutViewport,
    ) -> Result<crate::document::Page, net::DocumentLoadError>
    where
        P: net::pipeline::RequestPolicyEngine,
        T: net::Transport,
    {
        let page = loader.load(request, viewport)?;
        self.commit_page(&page);
        Ok(page)
    }

    pub fn navigate(&mut self, url: net::Url, title: Option<String>) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            let same_document = tab
                .history()
                .current()
                .is_some_and(|entry| entry.url().same_document(&url));
            tab.history_mut()
                .push(core::navigation::NavigationEntry::new(url, title));
            if !same_document {
                tab.clear_page();
            }
        }
    }

    pub fn navigate_reference(
        &mut self,
        reference: &str,
        title: Option<String>,
    ) -> Result<&net::Url, net::UrlError> {
        let base = self.current_url().ok_or(net::UrlError::MissingAuthority)?;
        let url = base.resolve(reference)?;
        self.navigate(url, title);
        Ok(self.current_url().expect("navigation created an entry"))
    }

    pub fn commit_page(&mut self, page: &crate::document::Page) -> bool {
        let Some(url) = page.url().cloned() else {
            return false;
        };

        self.navigate(url, page.title());
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.set_page(page.clone());
        }
        true
    }

    pub fn activate_link(
        &mut self,
        link: &crate::html::Node,
        title: Option<String>,
    ) -> Result<&net::Url, LinkActivationError> {
        if link.tag_name() != Some("a") {
            return Err(LinkActivationError::NotAnchor);
        }

        let href = link
            .link_href()
            .map(str::trim)
            .filter(|href| !href.is_empty())
            .ok_or(LinkActivationError::MissingHref)?;

        let base = self
            .current_url()
            .cloned()
            .ok_or(LinkActivationError::Url(net::UrlError::MissingAuthority))?;
        let url = base.resolve(href).map_err(LinkActivationError::from)?;
        let opens_new_tab = link
            .attribute("target")
            .is_some_and(|target| target.trim().eq_ignore_ascii_case("_blank"));

        if opens_new_tab {
            self.new_tab();
        }
        self.navigate(url, title);
        Ok(self
            .current_url()
            .expect("link navigation created an entry"))
    }

    pub fn activate_link_from_page(
        &mut self,
        page: &crate::document::Page,
        link: &crate::html::Node,
        title: Option<String>,
    ) -> Result<&net::Url, LinkActivationError> {
        if link.tag_name() != Some("a") {
            return Err(LinkActivationError::NotAnchor);
        }

        let href = link
            .link_href()
            .map(str::trim)
            .filter(|href| !href.is_empty())
            .ok_or(LinkActivationError::MissingHref)?;
        let url = page
            .resolve_reference(href)
            .map_err(LinkActivationError::from)?;
        let opens_new_tab = link
            .attribute("target")
            .is_some_and(|target| target.trim().eq_ignore_ascii_case("_blank"));

        if opens_new_tab {
            self.new_tab();
        }
        self.navigate(url, title);
        Ok(self
            .current_url()
            .expect("link navigation created an entry"))
    }

    pub fn current_page(&self) -> Option<&crate::document::Page> {
        self.tabs.active_tab().and_then(|tab| tab.page())
    }

    pub fn render_current_page(&self, surface: &mut crate::surface::SoftwareSurface) -> bool {
        let Some(page) = self.current_page() else {
            return false;
        };
        page.render_into(surface);
        true
    }

    pub fn current_url(&self) -> Option<&net::Url> {
        self.tabs
            .active_tab()
            .and_then(|tab| tab.history().current())
            .map(|entry| entry.url())
    }

    pub fn back(&mut self) -> Option<&net::Url> {
        let tab = self.tabs.active_tab_mut()?;
        let moved = tab.history_mut().back().is_some();
        if moved {
            tab.clear_page();
        }
        moved
            .then(|| tab.history().current())
            .flatten()
            .map(|entry| entry.url())
    }

    pub fn forward(&mut self) -> Option<&net::Url> {
        let tab = self.tabs.active_tab_mut()?;
        let moved = tab.history_mut().forward().is_some();
        if moved {
            tab.clear_page();
        }
        moved
            .then(|| tab.history().current())
            .flatten()
            .map(|entry| entry.url())
    }

    pub fn set_current_title(&mut self, title: Option<String>) -> bool {
        self.tabs
            .active_tab_mut()
            .is_some_and(|tab| tab.history_mut().set_current_title(title))
    }

    pub fn new_tab(&mut self) -> core::tabs::TabId {
        self.tabs.open()
    }

    pub fn select_tab(&mut self, id: core::tabs::TabId) -> bool {
        self.tabs.select(id)
    }

    pub fn close_tab(&mut self, id: core::tabs::TabId) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.close(id)
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_retains_loaded_page_for_rendering() {
        #[derive(Debug, Default)]
        struct AllowAll;

        impl net::pipeline::RequestPolicyEngine for AllowAll {
            fn decide(&self, _: &net::Request) -> net::pipeline::PolicyDecision {
                net::pipeline::PolicyDecision::Allow
            }
        }

        #[derive(Debug)]
        struct MockTransport;

        impl net::Transport for MockTransport {
            fn send(&self, _: &net::Request) -> Result<net::Response, net::TransportError> {
                Ok(net::Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>Hello</body>".to_vec()))
            }
        }

        let loader =
            net::DocumentLoader::new(net::pipeline::NetworkPipeline::new(AllowAll), MockTransport);
        let request = net::Request::new(net::Url::parse("https://example.org/").unwrap());
        let mut browser = Browser::new();
        browser
            .load_request(
                &loader,
                &request,
                crate::layout::LayoutViewport::new(64, 32),
            )
            .unwrap();

        assert!(browser.current_page().is_some());
        let mut surface = crate::surface::SoftwareSurface::new(64, 32);
        assert!(browser.render_current_page(&mut surface));
        assert_eq!(surface.pixel(63, 31), Some(crate::surface::Color::WHITE));
    }

    #[test]
    fn browser_loads_local_file_url_into_active_tab() {
        let path = std::env::temp_dir().join(format!(
            "archia-browser-browser-{}.html",
            std::process::id()
        ));
        std::fs::write(&path, b"<title>Local</title><body>Hello</body>").unwrap();
        let url = if cfg!(windows) {
            format!("file:///{}", path.display())
        } else {
            format!("file://{}", path.display())
        };

        let mut browser = Browser::new();
        let page = browser
            .load_local_file_url(&url, crate::layout::LayoutViewport::new(64, 32))
            .unwrap();

        assert_eq!(page.title(), Some("Local".to_owned()));
        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some(url)
        );
        assert!(browser.current_page().is_some());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn browser_starts_with_one_active_tab() {
        let browser = Browser::new();
        assert_eq!(browser.tabs().len(), 1);
        assert!(browser.tabs().active_id().is_some());
    }

    #[test]
    fn browser_reload_replaces_current_entry() {
        #[derive(Debug, Default)]
        struct AllowAll;

        impl net::pipeline::RequestPolicyEngine for AllowAll {
            fn decide(&self, _: &net::Request) -> net::pipeline::PolicyDecision {
                net::pipeline::PolicyDecision::Allow
            }
        }

        #[derive(Debug)]
        struct MockTransport;

        impl net::Transport for MockTransport {
            fn send(&self, _: &net::Request) -> Result<net::Response, net::TransportError> {
                Ok(net::Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<title>Reloaded</title><body>Updated</body>".to_vec()))
            }
        }

        let loader =
            net::DocumentLoader::new(net::pipeline::NetworkPipeline::new(AllowAll), MockTransport);
        let mut browser = Browser::new();
        browser.navigate(
            net::Url::parse("https://example.org/").unwrap(),
            Some("Old".to_owned()),
        );

        browser
            .reload(&loader, crate::layout::LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(browser.history().len(), 1);
        assert_eq!(
            browser.history().current().and_then(|entry| entry.title()),
            Some("Reloaded")
        );
    }

    #[test]
    fn browser_load_request_commits_loaded_page() {
        #[derive(Debug, Default)]
        struct AllowAll;

        impl net::pipeline::RequestPolicyEngine for AllowAll {
            fn decide(&self, _: &net::Request) -> net::pipeline::PolicyDecision {
                net::pipeline::PolicyDecision::Allow
            }
        }

        #[derive(Debug)]
        struct MockTransport;

        impl net::Transport for MockTransport {
            fn send(&self, _: &net::Request) -> Result<net::Response, net::TransportError> {
                Ok(net::Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<title>Loaded</title><body>Hello</body>".to_vec()))
            }
        }

        let loader =
            net::DocumentLoader::new(net::pipeline::NetworkPipeline::new(AllowAll), MockTransport);
        let request = net::Request::new(net::Url::parse("https://example.org/").unwrap());
        let mut browser = Browser::new();

        let page = browser
            .load_request(
                &loader,
                &request,
                crate::layout::LayoutViewport::new(320, 200),
            )
            .unwrap();

        assert_eq!(page.title(), Some("Loaded".to_owned()));
        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/".to_owned())
        );
        assert_eq!(
            browser.history().current().and_then(|entry| entry.title()),
            Some("Loaded")
        );
    }

    #[test]
    fn browser_navigation_updates_current_url() {
        let mut browser = Browser::new();
        browser.navigate(net::Url::parse("https://example.org/one").unwrap(), None);
        browser.navigate(net::Url::parse("https://example.org/two").unwrap(), None);

        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/two".to_owned())
        );
        assert!(browser.history().can_go_back());
    }

    #[test]
    fn browser_back_and_forward_use_navigation_history() {
        let mut browser = Browser::new();
        browser.navigate(net::Url::parse("https://example.org/one").unwrap(), None);
        browser.navigate(net::Url::parse("https://example.org/two").unwrap(), None);

        assert_eq!(
            browser.back().map(ToString::to_string),
            Some("https://example.org/one".to_owned())
        );
        assert_eq!(
            browser.forward().map(ToString::to_string),
            Some("https://example.org/two".to_owned())
        );
    }

    #[test]
    fn browser_navigates_relative_references() {
        let mut browser = Browser::new();
        browser.navigate(
            net::Url::parse("https://example.org/docs/index.html").unwrap(),
            Some("Index".to_owned()),
        );

        let current = browser
            .navigate_reference("../guide.html", Some("Guide".to_owned()))
            .unwrap();
        assert_eq!(current.to_string(), "https://example.org/guide.html");
        assert_eq!(browser.history().len(), 2);
        assert_eq!(
            browser.history().current().and_then(|entry| entry.title()),
            Some("Guide")
        );
    }

    #[test]
    fn browser_rejects_reference_without_base_navigation() {
        let mut browser = Browser::new();
        assert_eq!(
            browser.navigate_reference("/home", None),
            Err(net::UrlError::MissingAuthority)
        );
    }

    #[test]
    fn browser_commits_loaded_page_metadata() {
        let mut browser = Browser::new();
        let page = crate::document::Page::from_html_at(
            Some(net::Url::parse("https://example.org/docs/index.html").unwrap()),
            "<title>  Example  </title><body>Hello</body>",
            "",
            crate::layout::LayoutViewport::new(320, 200),
        );

        assert!(browser.commit_page(&page));
        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/docs/index.html".to_owned())
        );
        assert_eq!(
            browser.history().current().and_then(|entry| entry.title()),
            Some("Example")
        );
    }

    #[test]
    fn browser_does_not_commit_page_without_url() {
        let mut browser = Browser::new();
        let page = crate::document::Page::from_html(
            "<title>Example</title><body>Hello</body>",
            "",
            crate::layout::LayoutViewport::new(320, 200),
        );

        assert!(!browser.commit_page(&page));
        assert!(browser.current_url().is_none());
    }

    #[test]
    fn browser_activates_relative_anchor_links() {
        let mut browser = Browser::new();
        browser.navigate(
            net::Url::parse("https://example.org/docs/index.html").unwrap(),
            None,
        );

        let mut link = crate::html::Node::element("a");
        link.set_attribute("href", "../guide.html");

        let current = browser
            .activate_link(&link, Some("Guide".to_owned()))
            .unwrap();
        assert_eq!(current.to_string(), "https://example.org/guide.html");
        assert_eq!(browser.history().len(), 2);
        assert_eq!(
            browser.history().current().and_then(|entry| entry.title()),
            Some("Guide")
        );
    }

    #[test]
    fn browser_activates_page_link_using_base_href() {
        let mut browser = Browser::new();
        let page = crate::document::Page::from_html_at(
            Some(net::Url::parse("https://example.org/docs/index.html").unwrap()),
            r#"<head><base href="/guide/"></head><body>Hello</body>"#,
            "",
            crate::layout::LayoutViewport::new(320, 200),
        );
        let mut link = crate::html::Node::element("a");
        link.set_attribute("href", "chapter.html");

        let current = browser.activate_link_from_page(&page, &link, None).unwrap();
        assert_eq!(
            current.to_string(),
            "https://example.org/guide/chapter.html"
        );
    }

    #[test]
    fn browser_opens_blank_target_in_a_new_tab() {
        let mut browser = Browser::new();
        browser.navigate(net::Url::parse("https://example.org/").unwrap(), None);

        let mut link = crate::html::Node::element("a");
        link.set_attribute("href", "/new-tab");
        link.set_attribute("target", "_BLANK");

        let current = browser.activate_link(&link, None).unwrap();
        assert_eq!(current.to_string(), "https://example.org/new-tab");
        assert_eq!(browser.tabs().len(), 2);
    }

    #[test]
    fn browser_rejects_non_anchor_and_unsafe_scheme_links() {
        let mut browser = Browser::new();
        browser.navigate(net::Url::parse("https://example.org/").unwrap(), None);

        let div = crate::html::Node::element("div");
        assert_eq!(
            browser.activate_link(&div, None),
            Err(LinkActivationError::NotAnchor)
        );

        let mut script = crate::html::Node::element("a");
        script.set_attribute("href", "javascript:alert(1)");
        assert_eq!(
            browser.activate_link(&script, None),
            Err(LinkActivationError::Url(
                net::UrlError::UnsupportedReferenceScheme
            ))
        );
    }

    #[test]
    fn browser_fragment_navigation_keeps_loaded_page() {
        let mut browser = Browser::new();
        let page = crate::document::Page::from_html_at(
            Some(net::Url::parse("https://example.org/docs/index.html").unwrap()),
            "<body>Hello</body>",
            "",
            crate::layout::LayoutViewport::new(64, 32),
        );
        browser.commit_page(&page);

        browser.navigate_reference("#features", None).unwrap();

        assert!(browser.current_page().is_some());
        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/docs/index.html#features".to_owned())
        );
    }

    #[test]
    fn browser_fragment_navigation_creates_history_entry() {
        let mut browser = Browser::new();
        browser.navigate(
            net::Url::parse("https://example.org/docs/index.html").unwrap(),
            None,
        );

        let current = browser.navigate_reference("#features", None).unwrap();
        assert_eq!(
            current.to_string(),
            "https://example.org/docs/index.html#features"
        );
        assert!(browser.history().can_go_back());
    }

    #[test]
    fn browser_tabs_keep_independent_histories() {
        let mut browser = Browser::new();
        let first = browser.tabs().active_id().unwrap();

        browser.navigate(net::Url::parse("https://example.org/one").unwrap(), None);
        let second = browser.new_tab();
        browser.navigate(net::Url::parse("https://example.org/two").unwrap(), None);

        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/two".to_owned())
        );

        assert!(browser.select_tab(first));
        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/one".to_owned())
        );
        assert!(browser.select_tab(second));
        assert_eq!(
            browser.current_url().map(ToString::to_string),
            Some("https://example.org/two".to_owned())
        );
    }

    #[test]
    fn browser_updates_active_tab_title_without_navigation() {
        let mut browser = Browser::new();
        browser.navigate(
            net::Url::parse("https://example.org/").unwrap(),
            Some("Initial".to_owned()),
        );

        assert!(browser.set_current_title(Some("Updated".to_owned())));
        assert_eq!(browser.history().len(), 1);
        assert_eq!(
            browser.history().current().and_then(|entry| entry.title()),
            Some("Updated")
        );
    }

    #[test]
    fn browser_refuses_to_close_last_tab() {
        let mut browser = Browser::new();
        let only = browser.tabs().active_id().unwrap();

        assert!(!browser.close_tab(only));
        assert_eq!(browser.tabs().len(), 1);
        assert!(browser.tabs().active_id().is_some());
        assert!(browser.history().is_empty());
    }

    #[test]
    fn closing_tab_removes_its_history() {
        let mut browser = Browser::new();
        let first = browser.tabs().active_id().unwrap();
        let second = browser.new_tab();

        browser.navigate(net::Url::parse("https://example.org/two").unwrap(), None);
        assert!(browser.close_tab(second));
        assert!(browser.select_tab(first));
        assert_eq!(browser.current_url(), None);
        assert_eq!(browser.tabs().len(), 1);
    }
}
