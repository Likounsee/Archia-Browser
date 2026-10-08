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
            .expect("browser always has an active tab unless all tabs were closed")
            .history()
    }

    pub fn navigate(&mut self, url: net::Url, title: Option<String>) {
        if let Some(tab) = self.tabs.active_tab_mut() {
            tab.history_mut()
                .push(core::navigation::NavigationEntry::new(url, title));
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

    pub fn current_url(&self) -> Option<&net::Url> {
        self.tabs
            .active_tab()
            .and_then(|tab| tab.history().current())
            .map(|entry| entry.url())
    }

    pub fn back(&mut self) -> Option<&net::Url> {
        self.tabs
            .active_tab_mut()
            .and_then(|tab| tab.history_mut().back())
            .map(|entry| entry.url())
    }

    pub fn forward(&mut self) -> Option<&net::Url> {
        self.tabs
            .active_tab_mut()
            .and_then(|tab| tab.history_mut().forward())
            .map(|entry| entry.url())
    }

    pub fn new_tab(&mut self) -> core::tabs::TabId {
        self.tabs.open()
    }

    pub fn select_tab(&mut self, id: core::tabs::TabId) -> bool {
        self.tabs.select(id)
    }

    pub fn close_tab(&mut self, id: core::tabs::TabId) -> bool {
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
    fn browser_starts_with_one_active_tab() {
        let browser = Browser::new();
        assert_eq!(browser.tabs().len(), 1);
        assert!(browser.tabs().active_id().is_some());
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
