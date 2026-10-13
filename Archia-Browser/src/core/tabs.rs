use super::navigation::{NavigationEntry, NavigationHistory};
use crate::document::Page;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationAction {
    Navigate,
    Reload,
    Back,
    Forward,
}

/// Identifies one in-flight navigation. A result may only commit while its
/// token is still current for the tab that started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationToken {
    tab_id: TabId,
    generation: u64,
    requested_url: crate::net::Url,
    action: NavigationAction,
    history_index: Option<usize>,
}

impl NavigationToken {
    pub const fn tab_id(&self) -> TabId {
        self.tab_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn requested_url(&self) -> &crate::net::Url {
        &self.requested_url
    }

    pub const fn action(&self) -> NavigationAction {
        self.action
    }
}

#[derive(Debug)]
pub struct Tab {
    id: TabId,
    history: NavigationHistory,
    page: Option<Page>,
    navigation_generation: u64,
    pending_navigation: Option<NavigationToken>,
}

impl Tab {
    fn new(id: TabId) -> Self {
        Self {
            id,
            history: NavigationHistory::new(),
            page: None,
            navigation_generation: 0,
            pending_navigation: None,
        }
    }

    pub const fn id(&self) -> TabId {
        self.id
    }

    pub fn history(&self) -> &NavigationHistory {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut NavigationHistory {
        &mut self.history
    }

    pub fn page(&self) -> Option<&Page> {
        self.page.as_ref()
    }

    pub fn set_page(&mut self, page: Page) {
        self.page = Some(page);
    }

    pub fn clear_page(&mut self) {
        self.page = None;
    }

    /// Start a navigation transaction. Starting another navigation supersedes
    /// the previous one, so a late response cannot replace the newer document.
    pub fn begin_navigation(&mut self, requested_url: crate::net::Url) -> Option<NavigationToken> {
        self.begin_navigation_action(requested_url, NavigationAction::Navigate, None)
    }

    pub fn begin_reload(&mut self) -> Option<NavigationToken> {
        let url = self.history.current()?.url().clone();
        self.begin_navigation_action(url, NavigationAction::Reload, self.history.current_index())
    }

    pub fn begin_back_navigation(&mut self) -> Option<NavigationToken> {
        let (index, entry) = self.history.back_target()?;
        self.begin_navigation_action(entry.url().clone(), NavigationAction::Back, Some(index))
    }

    pub fn begin_forward_navigation(&mut self) -> Option<NavigationToken> {
        let (index, entry) = self.history.forward_target()?;
        self.begin_navigation_action(entry.url().clone(), NavigationAction::Forward, Some(index))
    }

    fn begin_navigation_action(
        &mut self,
        requested_url: crate::net::Url,
        action: NavigationAction,
        history_index: Option<usize>,
    ) -> Option<NavigationToken> {
        let generation = self.navigation_generation.checked_add(1)?;
        let token = NavigationToken {
            tab_id: self.id,
            generation,
            requested_url,
            action,
            history_index,
        };
        self.navigation_generation = generation;
        self.pending_navigation = Some(token.clone());
        Some(token)
    }

    pub fn is_navigation_current(&self, token: &NavigationToken) -> bool {
        self.pending_navigation.as_ref() == Some(token) && token.tab_id == self.id
    }

    /// Cancel only the transaction identified by this token. A stale caller
    /// cannot accidentally cancel a newer navigation.
    pub fn cancel_navigation(&mut self, token: &NavigationToken) -> bool {
        if !self.is_navigation_current(token) {
            return false;
        }
        self.pending_navigation = None;
        true
    }

    /// Commit a loaded page only if its navigation transaction is still current.
    ///
    /// On a stale response, returns the page to the caller and leaves the
    /// currently displayed page and history untouched. Redirected final URLs
    /// are taken from the committed page when available.
    pub fn commit_navigation(
        &mut self,
        token: &NavigationToken,
        page: Page,
        title: Option<String>,
    ) -> Result<(), Page> {
        if !self.is_navigation_current(token) {
            return Err(page);
        }

        let final_url = page
            .url()
            .cloned()
            .unwrap_or_else(|| token.requested_url.clone());
        let entry = NavigationEntry::new(final_url, title);
        let history_updated = match token.action {
            NavigationAction::Navigate => {
                self.history.push(entry);
                true
            }
            NavigationAction::Reload => {
                if self.history.replace_current(entry.clone()) {
                    true
                } else {
                    self.history.push(entry);
                    true
                }
            }
            NavigationAction::Back | NavigationAction::Forward => token
                .history_index
                .is_some_and(|index| self.history.replace_at_and_select(index, entry)),
        };
        if !history_updated {
            return Err(page);
        }
        self.page = Some(page);
        self.pending_navigation = None;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct TabManager {
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_id: u64,
}

impl TabManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.active_tab().map(Tab::id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.active.and_then(|index| self.tabs.get_mut(index))
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// Open a new tab, returning `None` if the unique ID space is exhausted.
    pub fn try_open(&mut self) -> Option<TabId> {
        let id = TabId(self.next_id);
        let next_id = self.next_id.checked_add(1)?;
        self.tabs.push(Tab::new(id));
        self.active = Some(self.tabs.len() - 1);
        self.next_id = next_id;
        Some(id)
    }

    /// Open a new tab.
    ///
    /// Panics only if all 64-bit tab IDs have been exhausted. Use
    /// `try_open` when exhaustion must be handled without panicking.
    pub fn open(&mut self) -> TabId {
        self.try_open().expect("tab ID space exhausted")
    }

    pub fn select(&mut self, id: TabId) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.active = Some(index);
        true
    }

    pub fn close(&mut self, id: TabId) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };

        self.tabs.remove(index);
        self.active = match self.active {
            None => None,
            Some(_) if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(active.min(self.tabs.len() - 1)),
            Some(active) => Some(active),
        };
        true
    }

    /// Begin a navigation for a specific tab, independent of which tab is active.
    pub fn begin_navigation(
        &mut self,
        id: TabId,
        requested_url: crate::net::Url,
    ) -> Option<NavigationToken> {
        self.tab_mut(id)?.begin_navigation(requested_url)
    }

    /// Commit a response to the tab that started it. Closed tabs and superseded
    /// requests are rejected, and the page is returned to the caller on rejection.
    pub fn commit_navigation(
        &mut self,
        id: TabId,
        token: &NavigationToken,
        page: Page,
        title: Option<String>,
    ) -> Result<(), Page> {
        let Some(tab) = self.tab_mut(id) else {
            return Err(page);
        };
        if token.tab_id != id {
            return Err(page);
        }
        tab.commit_navigation(token, page, title)
    }

    pub fn cancel_navigation(&mut self, id: TabId, token: &NavigationToken) -> bool {
        self.tab_mut(id)
            .is_some_and(|tab| tab.cancel_navigation(token))
    }

    pub fn ids(&self) -> impl Iterator<Item = TabId> + '_ {
        self.tabs.iter().map(Tab::id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Url;

    #[test]
    fn new_tabs_start_without_a_document() {
        let mut manager = TabManager::new();
        manager.open();
        assert!(manager.active_tab().unwrap().page().is_none());
    }

    #[test]
    fn starts_without_tabs() {
        let manager = TabManager::new();
        assert!(manager.is_empty());
        assert_eq!(manager.active_id(), None);
    }

    #[test]
    fn opening_tab_activates_it() {
        let mut manager = TabManager::new();
        let first = manager.open();
        let second = manager.open();

        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active_id(), Some(second));
        assert!(manager.tab(first).is_some());
    }

    #[test]
    fn selecting_unknown_tab_fails_without_changing_active_tab() {
        let mut manager = TabManager::new();
        let first = manager.open();
        let unknown = TabId(999);

        assert!(!manager.select(unknown));
        assert_eq!(manager.active_id(), Some(first));
    }

    #[test]
    fn each_tab_owns_independent_history() {
        let mut manager = TabManager::new();
        let first = manager.open();
        let second = manager.open();

        manager.tab_mut(first).unwrap().history_mut().push(
            super::super::navigation::NavigationEntry::new(
                Url::parse("https://example.org/one").unwrap(),
                None,
            ),
        );
        manager.tab_mut(second).unwrap().history_mut().push(
            super::super::navigation::NavigationEntry::new(
                Url::parse("https://example.org/two").unwrap(),
                None,
            ),
        );

        assert_eq!(
            manager
                .tab(first)
                .unwrap()
                .history()
                .current()
                .unwrap()
                .url()
                .to_string(),
            "https://example.org/one"
        );
        assert_eq!(
            manager
                .tab(second)
                .unwrap()
                .history()
                .current()
                .unwrap()
                .url()
                .to_string(),
            "https://example.org/two"
        );
    }

    #[test]
    fn closing_active_tab_selects_neighbor() {
        let mut manager = TabManager::new();
        let first = manager.open();
        let second = manager.open();
        let third = manager.open();

        manager.select(second);
        assert!(manager.close(second));
        assert_eq!(manager.active_id(), Some(third));
        assert!(manager.tab(first).is_some());
        assert!(manager.tab(second).is_none());
    }

    #[test]
    fn closing_last_tab_leaves_no_active_tab() {
        let mut manager = TabManager::new();
        let only = manager.open();

        assert!(manager.close(only));
        assert!(manager.is_empty());
        assert_eq!(manager.active_id(), None);
    }

    #[test]
    fn tab_id_exhaustion_fails_without_creating_duplicate_ids() {
        let mut manager = TabManager::new();
        manager.next_id = u64::MAX;

        assert_eq!(manager.try_open(), None);
        assert!(manager.is_empty());
        assert_eq!(manager.active_id(), None);
        assert_eq!(manager.next_id, u64::MAX);
    }

    fn test_page(url: &str) -> Page {
        Page::from_html_at(
            Some(Url::parse(url).unwrap()),
            "<html><body><p>test</p></body></html>",
            "",
            crate::layout::LayoutViewport::new(800, 600),
        )
    }

    #[test]
    fn latest_navigation_wins_and_stale_response_cannot_replace_page() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        let first = manager
            .begin_navigation(tab, Url::parse("https://example.org/slow").unwrap())
            .unwrap();
        let second = manager
            .begin_navigation(tab, Url::parse("https://example.org/fast").unwrap())
            .unwrap();

        assert!(!manager.tab(tab).unwrap().is_navigation_current(&first));
        assert!(manager.tab(tab).unwrap().is_navigation_current(&second));
        assert!(manager
            .commit_navigation(tab, &first, test_page("https://example.org/slow"), None)
            .is_err());
        assert!(manager.tab(tab).unwrap().page().is_none());
        assert!(manager.tab(tab).unwrap().history().is_empty());

        assert!(manager
            .commit_navigation(
                tab,
                &second,
                test_page("https://example.org/fast"),
                Some("Fast".into())
            )
            .is_ok());
        let committed = manager.tab(tab).unwrap();
        assert_eq!(
            committed.page().and_then(Page::url).unwrap().to_string(),
            "https://example.org/fast"
        );
        assert_eq!(
            committed.history().current().unwrap().url().to_string(),
            "https://example.org/fast"
        );
        assert_eq!(committed.history().current().unwrap().title(), Some("Fast"));
        assert!(committed.pending_navigation.is_none());
    }

    #[test]
    fn cancelled_navigation_does_not_clear_existing_document_or_history() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        let initial = manager
            .begin_navigation(tab, Url::parse("https://example.org/initial").unwrap())
            .unwrap();
        manager
            .commit_navigation(
                tab,
                &initial,
                test_page("https://example.org/initial"),
                None,
            )
            .unwrap();

        let pending = manager
            .begin_navigation(tab, Url::parse("https://example.org/cancelled").unwrap())
            .unwrap();
        assert!(manager.cancel_navigation(tab, &pending));
        assert!(!manager.cancel_navigation(tab, &pending));
        assert!(manager
            .commit_navigation(
                tab,
                &pending,
                test_page("https://example.org/cancelled"),
                None
            )
            .is_err());

        let current = manager.tab(tab).unwrap();
        assert_eq!(current.history().len(), 1);
        assert_eq!(
            current.history().current().unwrap().url().to_string(),
            "https://example.org/initial"
        );
        assert_eq!(
            current.page().and_then(Page::url).unwrap().to_string(),
            "https://example.org/initial"
        );
    }

    #[test]
    fn redirected_final_url_is_the_history_url() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        let token = manager
            .begin_navigation(tab, Url::parse("https://example.org/start").unwrap())
            .unwrap();
        manager
            .commit_navigation(
                tab,
                &token,
                test_page("https://example.org/final"),
                None,
            )
            .unwrap();
        assert_eq!(
            manager
                .tab(tab)
                .unwrap()
                .history()
                .current()
                .unwrap()
                .url()
                .to_string(),
            "https://example.org/final"
        );
    }

    #[test]
    fn closing_tab_rejects_its_late_navigation_result() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        let token = manager
            .begin_navigation(tab, Url::parse("https://example.org/late").unwrap())
            .unwrap();
        assert!(manager.close(tab));
        assert!(manager
            .commit_navigation(tab, &token, test_page("https://example.org/late"), None)
            .is_err());
        assert!(manager.tab(tab).is_none());
    }

    #[test]
    fn reload_replaces_current_entry_without_growing_history() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        let first = manager
            .begin_navigation(tab, Url::parse("https://example.org/page").unwrap())
            .unwrap();
        manager
            .commit_navigation(tab, &first, test_page("https://example.org/page"), None)
            .unwrap();

        let reload = manager.tab_mut(tab).unwrap().begin_reload().unwrap();
        assert_eq!(reload.action(), NavigationAction::Reload);
        manager
            .commit_navigation(
                tab,
                &reload,
                test_page("https://example.org/page"),
                Some("Reloaded".into()),
            )
            .unwrap();

        let history = manager.tab(tab).unwrap().history();
        assert_eq!(history.len(), 1);
        assert_eq!(history.current().unwrap().title(), Some("Reloaded"));
    }

    #[test]
    fn back_and_forward_change_history_only_after_successful_commit() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        for url in ["https://example.org/one", "https://example.org/two"] {
            let token = manager
                .begin_navigation(tab, Url::parse(url).unwrap())
                .unwrap();
            manager
                .commit_navigation(tab, &token, test_page(url), None)
                .unwrap();
        }

        let back = manager
            .tab_mut(tab)
            .unwrap()
            .begin_back_navigation()
            .unwrap();
        assert_eq!(back.action(), NavigationAction::Back);
        assert_eq!(back.requested_url().to_string(), "https://example.org/one");
        assert_eq!(manager.tab(tab).unwrap().history().current().unwrap().url().to_string(), "https://example.org/two");
        assert!(manager.commit_navigation(tab, &back, test_page("https://example.org/one"), None).is_ok());
        assert_eq!(manager.tab(tab).unwrap().history().current().unwrap().url().to_string(), "https://example.org/one");

        let forward = manager.tab_mut(tab).unwrap().begin_forward_navigation().unwrap();
        assert_eq!(forward.action(), NavigationAction::Forward);
        assert_eq!(forward.requested_url().to_string(), "https://example.org/two");
        assert!(manager.commit_navigation(tab, &forward, test_page("https://example.org/two"), None).is_ok());
        assert_eq!(manager.tab(tab).unwrap().history().current().unwrap().url().to_string(), "https://example.org/two");
        assert_eq!(manager.tab(tab).unwrap().history().len(), 2);
    }

    #[test]
    fn cancelled_back_navigation_does_not_move_history_or_replace_page() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        for url in ["https://example.org/one", "https://example.org/two"] {
            let token = manager
                .begin_navigation(tab, Url::parse(url).unwrap())
                .unwrap();
            manager
                .commit_navigation(tab, &token, test_page(url), None)
                .unwrap();
        }
        let back = manager
            .tab_mut(tab)
            .unwrap()
            .begin_back_navigation()
            .unwrap();
        assert!(manager.cancel_navigation(tab, &back));

        let current = manager.tab(tab).unwrap();
        assert_eq!(
            current.history().current().unwrap().url().to_string(),
            "https://example.org/two"
        );
        assert_eq!(
            current.page().and_then(Page::url).unwrap().to_string(),
            "https://example.org/two"
        );
        assert_eq!(current.history().len(), 2);
    }

    #[test]
    fn stale_cancellation_cannot_cancel_newer_navigation() {
        let mut manager = TabManager::new();
        let tab = manager.open();
        let old = manager
            .begin_navigation(tab, Url::parse("https://example.org/old").unwrap())
            .unwrap();
        let current = manager
            .begin_navigation(tab, Url::parse("https://example.org/current").unwrap())
            .unwrap();
        assert!(!manager.cancel_navigation(tab, &old));
        assert!(manager.tab(tab).unwrap().is_navigation_current(&current));
    }

    #[test]
    fn ids_are_monotonic_even_after_close() {
        let mut manager = TabManager::new();
        let first = manager.open();
        assert!(manager.close(first));

        let second = manager.open();
        assert!(second.value() > first.value());
    }
}
