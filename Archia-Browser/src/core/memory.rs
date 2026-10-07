use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
pub struct MemoryBudget {
    limit: usize,
    used: AtomicUsize,
}

impl MemoryBudget {
    pub const fn new(limit: usize) -> Self {
        Self { limit, used: AtomicUsize::new(0) }
    }

    pub fn try_reserve(&self, bytes: usize) -> bool {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return false;
            };
            if next > self.limit {
                return false;
            }
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }

    pub fn limit(&self) -> usize { self.limit }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    pub fn available(&self) -> usize {
        self.limit.saturating_sub(self.used())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_prevents_overcommit() {
        let budget = MemoryBudget::new(1024);
        assert!(budget.try_reserve(512));
        assert_eq!(budget.used(), 512);
        assert!(!budget.try_reserve(513));
        budget.release(256);
        assert_eq!(budget.available(), 768);
    }
}
