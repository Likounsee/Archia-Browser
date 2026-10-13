use crate::net::Url;
use std::collections::VecDeque;

/// Bound per-tab navigation metadata so long-running sessions cannot grow history forever.
pub const MAX_HISTORY_ENTRIES: usize = 4096;

/// Limit untrusted page titles retained by each history entry.
pub const MAX_HISTORY_TITLE_BYTES: usize = 4096;

fn bounded_title(title: Option<String>) -> Option<String> {
    title.map(|mut title| {
        if title.len() > MAX_HISTORY_TITLE_BYTES {
            let mut boundary = MAX_HISTORY_TITLE_BYTES;
            while !title.is_char_boundary(boundary) {
                boundary -= 1;
            }
            title.truncate(boundary);
        }
        title
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationEntry {
    url: Url,
    title: Option<String>,
}

impl NavigationEntry {
    pub fn new(url: Url, title: Option<String>) -> Self {
        Self {
            url,
            title: bounded_title(title),
        }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn set_title(&mut self, title: Option<String>) {
        self.title = bounded_title(title);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavigationHistory {
    entries: VecDeque<NavigationEntry>,
    current: Option<usize>,
}

impl NavigationHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn current(&self) -> Option<&NavigationEntry> {
        self.current.and_then(|index| self.entries.get(index))
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn back_target(&self) -> Option<(usize, &NavigationEntry)> {
        let target = self.current?.checked_sub(1)?;
        self.entries.get(target).map(|entry| (target, entry))
    }

    pub fn forward_target(&self) -> Option<(usize, &NavigationEntry)> {
        let target = self.current?.checked_add(1)?;
        self.entries.get(target).map(|entry| (target, entry))
    }

    /// Atomically select and update an existing history slot after a successful
    /// back/forward load. Returns false without mutation for an invalid index.
    pub fn replace_at_and_select(&mut self, index: usize, entry: NavigationEntry) -> bool {
        let Some(current) = self.entries.get_mut(index) else {
            return false;
        };
        *current = entry;
        self.current = Some(index);
        true
    }

    pub fn can_go_back(&self) -> bool {
        self.current.is_some_and(|index| index > 0)
    }

    pub fn can_go_forward(&self) -> bool {
        self.current
            .is_some_and(|index| index + 1 < self.entries.len())
    }

    pub fn push(&mut self, entry: NavigationEntry) {
        if let Some(current) = self.current {
            self.entries.truncate(current + 1);
        }

        self.entries.push_back(entry);
        if self.entries.len() > MAX_HISTORY_ENTRIES {
            // push() always moves to the newest entry, so dropping the oldest
            // entry keeps the active index valid and preserves back/forward order.
            self.entries.pop_front();
        }
        self.current = Some(self.entries.len() - 1);
    }

    pub fn back(&mut self) -> Option<&NavigationEntry> {
        let index = self.current?;
        if index == 0 {
            return self.current();
        }

        self.current = Some(index - 1);
        self.current()
    }

    pub fn forward(&mut self) -> Option<&NavigationEntry> {
        let index = self.current?;
        if index + 1 >= self.entries.len() {
            return self.current();
        }

        self.current = Some(index + 1);
        self.current()
    }

    pub fn replace_current(&mut self, entry: NavigationEntry) -> bool {
        let Some(index) = self.current else {
            return false;
        };
        let Some(current) = self.entries.get_mut(index) else {
            return false;
        };

        *current = entry;
        true
    }

    pub fn set_current_title(&mut self, title: Option<String>) -> bool {
        let Some(index) = self.current else {
            return false;
        };
        let Some(entry) = self.entries.get_mut(index) else {
            return false;
        };

        entry.set_title(title);
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str) -> NavigationEntry {
        NavigationEntry::new(Url::parse(url).unwrap(), None)
    }

    #[test]
    fn starts_empty() {
        let history = NavigationHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.current(), None);
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn push_back_and_forward_follow_history() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/one"));
        history.push(entry("https://example.org/two"));
        history.push(entry("https://example.org/three"));

        assert_eq!(
            history.current().map(|entry| entry.url().to_string()),
            Some("https://example.org/three".to_string())
        );
        assert!(history.can_go_back());
        assert!(!history.can_go_forward());

        assert_eq!(
            history.back().map(|entry| entry.url().to_string()),
            Some("https://example.org/two".to_string())
        );
        assert!(history.can_go_forward());

        assert_eq!(
            history.forward().map(|entry| entry.url().to_string()),
            Some("https://example.org/three".to_string())
        );
    }

    #[test]
    fn new_navigation_discards_forward_entries() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/one"));
        history.push(entry("https://example.org/two"));
        history.back();
        history.push(entry("https://example.org/branch"));

        assert_eq!(history.len(), 2);
        assert_eq!(
            history.current().map(|entry| entry.url().to_string()),
            Some("https://example.org/branch".to_string())
        );
        assert!(!history.can_go_forward());
    }

    #[test]
    fn boundary_navigation_keeps_current_entry() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/one"));

        assert_eq!(
            history.back().map(|entry| entry.url().to_string()),
            Some("https://example.org/one".to_string())
        );
        assert_eq!(
            history.forward().map(|entry| entry.url().to_string()),
            Some("https://example.org/one".to_string())
        );
    }

    #[test]
    fn titles_are_retained_and_can_be_updated() {
        let mut history = NavigationHistory::new();
        history.push(NavigationEntry::new(
            Url::parse("https://example.org/").unwrap(),
            Some("Example".to_owned()),
        ));

        assert_eq!(
            history.current().and_then(NavigationEntry::title),
            Some("Example")
        );
        assert!(history.set_current_title(Some("Updated".to_owned())));
        assert_eq!(
            history.current().and_then(NavigationEntry::title),
            Some("Updated")
        );
        assert!(history.set_current_title(None));
        assert_eq!(history.current().and_then(NavigationEntry::title), None);
    }

    #[test]
    fn replace_current_preserves_history_position() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/one"));
        history.push(entry("https://example.org/two"));
        history.back();

        assert!(history.replace_current(entry("https://example.org/reloaded")));
        assert_eq!(history.len(), 2);
        assert_eq!(
            history.current().map(|entry| entry.url().to_string()),
            Some("https://example.org/reloaded".to_owned())
        );
        assert!(history.can_go_forward());
    }

    #[test]
    fn page_titles_are_bounded_without_splitting_utf8() {
        let oversized = format!("{}{}", "a".repeat(MAX_HISTORY_TITLE_BYTES - 2), "éé");
        let mut history = NavigationHistory::new();
        history.push(NavigationEntry::new(
            Url::parse("https://example.org/").unwrap(),
            Some(oversized),
        ));

        let title = history.current().and_then(NavigationEntry::title).unwrap();
        assert!(title.len() <= MAX_HISTORY_TITLE_BYTES);
        assert!(title.is_char_boundary(title.len()));

        assert!(history.set_current_title(Some("é".repeat(MAX_HISTORY_TITLE_BYTES))));
        let updated = history.current().and_then(NavigationEntry::title).unwrap();
        assert!(updated.len() <= MAX_HISTORY_TITLE_BYTES);
        assert!(updated.is_char_boundary(updated.len()));
    }

    #[test]
    fn title_update_does_not_create_history_entry() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/"));
        assert!(history.set_current_title(Some("Example".to_owned())));

        assert_eq!(history.len(), 1);
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn title_update_on_empty_history_fails() {
        let mut history = NavigationHistory::new();
        assert!(!history.set_current_title(Some("Example".to_owned())));
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/"));
        history.clear();

        assert!(history.is_empty());
        assert_eq!(history.current(), None);
    }
    #[test]
    fn history_is_bounded_and_keeps_the_newest_entries() {
        let mut history = NavigationHistory::new();
        for index in 0..=MAX_HISTORY_ENTRIES {
            history.push(entry(&format!("https://example.org/{index}")));
        }

        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(
            history.current().map(|entry| entry.url().to_string()),
            Some(format!("https://example.org/{MAX_HISTORY_ENTRIES}"))
        );
        for _ in 0..MAX_HISTORY_ENTRIES - 1 {
            assert!(history.back().is_some());
        }
        assert_eq!(
            history.current().map(|entry| entry.url().to_string()),
            Some("https://example.org/1".to_owned()),
            "the oldest retained entry should be the first URL after eviction"
        );
        assert!(!history.can_go_back());
    }

    #[test]
    fn bounded_history_branching_discards_forward_entries_and_stays_bounded() {
        let mut history = NavigationHistory::new();
        for index in 0..MAX_HISTORY_ENTRIES {
            history.push(entry(&format!("https://example.org/{index}")));
        }
        history.back();
        history.push(entry("https://example.org/branch"));

        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(
            history.current().map(|entry| entry.url().to_string()),
            Some("https://example.org/branch".to_owned())
        );
        assert!(!history.can_go_forward());
    }
}
