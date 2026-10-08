use super::navigation::NavigationHistory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Default)]
pub struct Tab {
    id: TabId,
    history: NavigationHistory,
}

impl Tab {
    fn new(id: TabId) -> Self {
        Self {
            id,
            history: NavigationHistory::new(),
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

    pub fn open(&mut self) -> TabId {
        let id = TabId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.tabs.push(Tab::new(id));
        self.active = Some(self.tabs.len() - 1);
        id
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
            Some(active) if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(active.min(self.tabs.len() - 1)),
            Some(active) => Some(active),
        };
        true
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
    fn ids_are_monotonic_even_after_close() {
        let mut manager = TabManager::new();
        let first = manager.open();
        assert!(manager.close(first));

        let second = manager.open();
        assert!(second.value() > first.value());
    }
}
