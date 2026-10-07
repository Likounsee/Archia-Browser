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
        self.entries.push_back(CacheEntry {
            key: key.into(),
            value,
            bytes,
        });
    }

    pub fn get(&mut self, key: &str) -> Option<&T> {
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| &entry.value)
    }

    pub fn used(&self) -> usize {
        self.used
    }
    pub fn len(&self) -> usize {
        self.entries.len()
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
    fn cache_reclaims_old_entries() {
        let mut cache = ResourceCache::new(10);
        cache.insert("a", 1, 6);
        cache.insert("b", 2, 6);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.used(), 6);
    }
}
