use std::collections::VecDeque;

#[derive(Debug)]
pub struct CacheEntry<T> {
    pub key: String,
    pub value: T,
    pub bytes: usize,
}

#[derive(Debug)]
pub struct ResourceCache<T> {
    capacity: usize,
    used: usize,
    entries: VecDeque<CacheEntry<T>>,
}

impl<T> ResourceCache<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            used: 0,
            entries: VecDeque::new(),
        }
    }

    pub fn insert(&mut self, key: impl Into<String>, value: T, bytes: usize) {
        let key = key.into();

        if let Some(position) = self.entries.iter().position(|entry| entry.key == key) {
            if let Some(previous) = self.entries.remove(position) {
                self.used = self.used.saturating_sub(previous.bytes);
            }
        }

        while self.used.saturating_add(bytes) > self.capacity {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.used = self.used.saturating_sub(oldest.bytes);
        }

        if bytes > self.capacity {
            return;
        }

        self.used += bytes;
        self.entries.push_back(CacheEntry { key, value, bytes });
    }

    pub fn get(&mut self, key: &str) -> Option<&T> {
        let position = self.entries.iter().position(|entry| entry.key == key)?;
        if let Some(entry) = self.entries.remove(position) {
            self.entries.push_back(entry);
        }
        self.entries.back().map(|entry| &entry.value)
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn remove(&mut self, key: &str) -> Option<T> {
        let position = self.entries.iter().position(|entry| entry.key == key)?;
        let entry = self.entries.remove(position)?;
        self.used = self.used.saturating_sub(entry.bytes);
        Some(entry.value)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_get_promotes_recent_entries() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 4);
        cache.insert("b", 2, 4);
        assert_eq!(cache.get("a"), Some(&1));
        cache.insert("c", 3, 4);

        assert_eq!(cache.get("a"), Some(&1));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("c"), Some(&3));
    }

    #[test]
    fn replacing_a_key_reclaims_previous_bytes() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 6);
        cache.insert("a", 2, 4);

        assert_eq!(cache.get("a"), Some(&2));
        assert_eq!(cache.used(), 4);
    }

    #[test]
    fn removing_a_key_releases_its_bytes() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 6);
        assert_eq!(cache.remove("a"), Some(1));
        assert_eq!(cache.used(), 0);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cache_reclaims_old_entries() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 6);
        cache.insert("b", 2, 6);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.used(), 6);
    }
}
