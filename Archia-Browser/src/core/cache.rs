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
        // Even an empty payload occupies memory through its key and entry
        // metadata. Account for the key too, and charge at least one byte,
        // so empty entries cannot bypass the cache capacity.
        let bytes = bytes.max(key.len()).max(1);

        if let Some(position) = self.entries.iter().position(|entry| entry.key == key) {
            if let Some(previous) = self.entries.remove(position) {
                self.used = self.used.saturating_sub(previous.bytes);
            }
        }

        // Reject oversized entries before evicting unrelated cache entries.
        if bytes > self.capacity {
            return;
        }

        while bytes > self.capacity.saturating_sub(self.used) {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.used = self.used.saturating_sub(oldest.bytes);
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

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remove(&mut self, key: &str) -> Option<T> {
        let position = self.entries.iter().position(|entry| entry.key == key)?;
        let entry = self.entries.remove(position)?;
        self.used = self.used.saturating_sub(entry.bytes);
        Some(entry.value)
    }

    /// Removes entries that do not satisfy `keep`, releasing their accounted bytes.
    pub fn retain(&mut self, mut keep: impl FnMut(&CacheEntry<T>) -> bool) {
        let mut used = self.used;
        self.entries.retain(|entry| {
            if keep(entry) {
                true
            } else {
                used = used.saturating_sub(entry.bytes);
                false
            }
        });
        self.used = used;
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
    fn zero_byte_entries_still_consume_cache_capacity() {
        let mut cache = ResourceCache::new(2);
        cache.insert("a", 1, 0);
        cache.insert("b", 2, 0);
        cache.insert("c", 3, 0);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.used(), 2);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.get("c"), Some(&3));

        let mut zero_capacity = ResourceCache::new(0);
        zero_capacity.insert("empty", (), 0);
        assert_eq!(zero_capacity.len(), 0);
        assert_eq!(zero_capacity.used(), 0);
    }

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
    fn oversized_entry_does_not_evict_unrelated_entries() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 4);
        cache.insert("b", 2, 4);

        cache.insert("oversized", 3, 11);

        assert_eq!(cache.get("a"), Some(&1));
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.get("oversized"), None);
        assert_eq!(cache.used(), 8);
    }

    #[test]
    fn oversized_replacement_removes_only_the_replaced_entry() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 4);
        cache.insert("b", 2, 4);

        cache.insert("a", 3, 11);

        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.used(), 4);
    }

    #[test]
    fn retain_releases_bytes_for_removed_entries() {
        let mut cache = ResourceCache::new(10);
        cache.insert("keep", 1, 4);
        cache.insert("drop", 2, 5);

        cache.retain(|entry| entry.key == "keep");

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used(), 4);
        assert_eq!(cache.get("keep"), Some(&1));
        assert_eq!(cache.get("drop"), None);
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
