use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub limit: usize,
    pub used: usize,
    pub available: usize,
}

#[derive(Debug)]
pub struct MemoryBudget {
    limit: usize,
    used: AtomicUsize,
}

impl MemoryBudget {
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
        }
    }

    pub fn try_reserve(&self, bytes: usize) -> bool {
        let mut current = self.used.load(Ordering::Acquire);
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
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn release(&self, bytes: usize) {
        let _ = self
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                Some(used.saturating_sub(bytes))
            });
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    pub fn available(&self) -> usize {
        self.limit.saturating_sub(self.used())
    }

    pub fn snapshot(&self) -> MemorySnapshot {
        let used = self.used();
        MemorySnapshot {
            limit: self.limit,
            used,
            available: self.limit.saturating_sub(used),
        }
    }
}

#[derive(Debug)]
pub struct MemoryReservation {
    budget: Arc<MemoryBudget>,
    bytes: usize,
}

impl MemoryReservation {
    pub fn try_new(budget: Arc<MemoryBudget>, bytes: usize) -> Option<Self> {
        budget.try_reserve(bytes).then_some(Self { budget, bytes })
    }

    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_releases_on_drop() {
        let budget = Arc::new(MemoryBudget::new(1024));
        {
            let reservation = MemoryReservation::try_new(Arc::clone(&budget), 512).unwrap();
            assert_eq!(reservation.bytes(), 512);
            assert_eq!(budget.used(), 512);
        }
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn budget_prevents_overcommit() {
        let budget = MemoryBudget::new(1024);
        assert!(budget.try_reserve(1024));
        assert!(!budget.try_reserve(1));
        budget.release(1024);
        assert_eq!(budget.available(), 1024);
    }
}
