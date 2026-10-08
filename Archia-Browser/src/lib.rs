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
    history: core::navigation::NavigationHistory,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            memory: core::memory::MemoryBudget::new(512 * 1024 * 1024),
            history: core::navigation::NavigationHistory::new(),
        }
    }

    pub const fn version(&self) -> &'static str {
        self.version
    }

    pub fn memory(&self) -> &core::memory::MemoryBudget {
        &self.memory
    }

    pub fn history(&self) -> &core::navigation::NavigationHistory {
        &self.history
    }

    pub fn navigate(&mut self, url: net::Url, title: Option<String>) {
        self.history
            .push(core::navigation::NavigationEntry::new(url, title));
    }

    pub fn current_url(&self) -> Option<&net::Url> {
        self.history.current().map(|entry| entry.url())
    }

    pub fn back(&mut self) -> Option<&net::Url> {
        self.history.back().map(|entry| entry.url())
    }

    pub fn forward(&mut self) -> Option<&net::Url> {
        self.history.forward().map(|entry| entry.url())
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
}
