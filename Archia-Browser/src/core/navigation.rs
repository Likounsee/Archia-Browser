use crate::net::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationEntry {
    url: Url,
    title: Option<String>,
}

impl NavigationEntry {
    pub fn new(url: Url, title: Option<String>) -> Self {
        Self { url, title }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavigationHistory {
    entries: Vec<NavigationEntry>,
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

        self.entries.push(entry);
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
    fn titles_are_retained() {
        let mut history = NavigationHistory::new();
        history.push(NavigationEntry::new(
            Url::parse("https://example.org/").unwrap(),
            Some("Example".to_owned()),
        ));

        assert_eq!(
            history.current().and_then(NavigationEntry::title),
            Some("Example")
        );
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut history = NavigationHistory::new();
        history.push(entry("https://example.org/"));
        history.clear();

        assert!(history.is_empty());
        assert_eq!(history.current(), None);
    }
}
