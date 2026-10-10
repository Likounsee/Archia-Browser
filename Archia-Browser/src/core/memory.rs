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
        let mut current = self.used.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_sub(bytes) else {
                return;
            };
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
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
        // Do not use bool::then_some here: its argument is evaluated eagerly,
        // so a failed reservation would construct and drop a reservation,
        // releasing bytes that were never acquired and corrupting accounting.
        if budget.try_reserve(bytes) {
            Some(Self { budget, bytes })
        } else {
            None
        }
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

    #[test]
    fn failed_reservation_does_not_release_bytes_owned_by_another_reservation() {
        let budget = Arc::new(MemoryBudget::new(1024));
        assert!(budget.try_reserve(700));

        assert!(MemoryReservation::try_new(Arc::clone(&budget), 400).is_none());

        assert_eq!(budget.used(), 700);
        assert_eq!(budget.available(), 324);
        assert!(!budget.try_reserve(325));
    }

    #[test]
    fn reservation_accounting_stays_balanced_over_one_thousand_cycles() {
        let budget = Arc::new(MemoryBudget::new(4096));
        for iteration in 0..1_000 {
            let bytes = iteration % 512 + 1;
            let reservation =
                MemoryReservation::try_new(Arc::clone(&budget), bytes).unwrap();
            assert_eq!(budget.used(), bytes);
            drop(reservation);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn concurrent_reservations_never_overcommit_the_budget() {
        use std::sync::Barrier;

        let budget = Arc::new(MemoryBudget::new(1024));
        let barrier = Arc::new(Barrier::new(16));
        let mut workers = Vec::new();
        for _ in 0..16 {
            let budget = Arc::clone(&budget);
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                let reserved = budget.try_reserve(128);
                barrier.wait();
                if reserved {
                    budget.release(128);
                }
                reserved
            }));
        }

        let mut successful_reservations = 0;
        for worker in workers {
            if worker.join().unwrap() {
                successful_reservations += 1;
            }
        }
        assert!(successful_reservations <= 8);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn over_release_does_not_erase_existing_reservations() {
        let budget = MemoryBudget::new(1024);
        assert!(budget.try_reserve(700));

        budget.release(701);

        assert_eq!(budget.used(), 700);
        assert_eq!(budget.available(), 324);
        assert!(budget.try_reserve(324));
        assert!(!budget.try_reserve(1));
    }
}
