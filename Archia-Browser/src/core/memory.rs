pub(crate) const MAX_LIVE_BUDGETED_OBJECTS: usize = 4096;

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
    live_resources: AtomicUsize,
}

impl MemoryBudget {
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
            live_resources: AtomicUsize::new(0),
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

    /// Reserve one live-object slot shared by every subsystem using this budget.
    /// This independently bounds zero-byte resources and metadata allocations.
    pub fn try_reserve_resource_slot(&self, max_live: usize) -> bool {
        let mut current = self.live_resources.load(Ordering::Acquire);
        loop {
            if current >= max_live {
                return false;
            }
            let Some(next) = current.checked_add(1) else {
                return false;
            };
            match self.live_resources.compare_exchange_weak(
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

    /// Release a previously reserved live-object slot.
    pub fn release_resource_slot(&self) {
        let mut current = self.live_resources.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_sub(1) else {
                return;
            };
            match self.live_resources.compare_exchange_weak(
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

    /// Acquire an RAII guard for a live-object slot.
    ///
    /// The slot is released on every exit path, including early returns and
    /// unwinding, rather than relying on callers to pair manual increments
    /// and decrements correctly.
    pub fn try_reserve_resource_slot_guard(
        budget: Arc<MemoryBudget>,
        max_live: usize,
    ) -> Option<MemoryResourceSlot> {
        if budget.try_reserve_resource_slot(max_live) {
            Some(MemoryResourceSlot { budget })
        } else {
            None
        }
    }

    pub fn live_resources(&self) -> usize {
        self.live_resources.load(Ordering::Acquire)
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

/// RAII reservation for one live-object slot in a shared memory budget.
#[derive(Debug)]
pub struct MemoryResourceSlot {
    budget: Arc<MemoryBudget>,
}

impl Drop for MemoryResourceSlot {
    fn drop(&mut self) {
        self.budget.release_resource_slot();
    }
}

#[derive(Debug)]
pub struct MemoryReservation {
    budget: Arc<MemoryBudget>,
    bytes: AtomicUsize,
}

impl MemoryReservation {
    pub fn try_new(budget: Arc<MemoryBudget>, bytes: usize) -> Option<Self> {
        // Do not use bool::then_some here: its argument is evaluated eagerly,
        // so a failed reservation would construct and drop a reservation,
        // releasing bytes that were never acquired and corrupting accounting.
        if budget.try_reserve(bytes) {
            Some(Self {
                budget,
                bytes: AtomicUsize::new(bytes),
            })
        } else {
            None
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }

    /// Reserve an independent charge for a deep copy of this allocation-backed value.
    /// The returned reservation is not shared with the original, so either copy can
    /// be dropped independently without undercounting the other copy.
    pub fn try_clone_reservation(&self) -> Option<Self> {
        Self::try_new(Arc::clone(&self.budget), self.bytes())
    }

    /// Extend an existing reservation without temporarily releasing its current
    /// charge. Returns false without changing accounting if the extra budget
    /// cannot be acquired.
    pub fn try_grow_by(&self, additional: usize) -> bool {
        if additional == 0 {
            return true;
        }
        if !self.budget.try_reserve(additional) {
            return false;
        }

        // Growth and shrinkage can race through shared reservations. A CAS loop
        // prevents a concurrent shrink from being overwritten by fetch_add.
        let mut current = self.bytes.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(additional) else {
                self.budget.release(additional);
                return false;
            };
            match self.bytes.compare_exchange_weak(
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

    /// Reduce an existing reservation and immediately return the excess budget.
    /// Reservations can be shared by cache entries through `Arc`, so the
    /// byte count is atomic and remains consistent for every owner.
    pub fn shrink_to(&self, target: usize) {
        let mut current = self.bytes.load(Ordering::Acquire);
        loop {
            if target >= current {
                return;
            }
            match self.bytes.compare_exchange_weak(
                current,
                target,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.budget.release(current - target);
                    return;
                }
                Err(actual) => current = actual,
            }
        }
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        let bytes = self.bytes.swap(0, Ordering::AcqRel);
        self.budget.release(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_slot_guard_releases_on_drop_and_rejects_overcommit() {
        let budget = Arc::new(MemoryBudget::new(0));
        let slot = MemoryBudget::try_reserve_resource_slot_guard(Arc::clone(&budget), 1)
            .expect("first live-object slot should fit");
        assert_eq!(budget.live_resources(), 1);
        assert!(MemoryBudget::try_reserve_resource_slot_guard(Arc::clone(&budget), 1).is_none());
        assert_eq!(budget.live_resources(), 1);
        drop(slot);
        assert_eq!(budget.live_resources(), 0);
        assert!(MemoryBudget::try_reserve_resource_slot_guard(Arc::clone(&budget), 1).is_some());
    }

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
    fn shrinking_shared_reservation_releases_only_excess_bytes() {
        let budget = Arc::new(MemoryBudget::new(1024));
        let reservation = Arc::new(MemoryReservation::try_new(Arc::clone(&budget), 700).unwrap());
        let shared = Arc::clone(&reservation);

        reservation.shrink_to(256);

        assert_eq!(reservation.bytes(), 256);
        assert_eq!(shared.bytes(), 256);
        assert_eq!(budget.used(), 256);
        drop(reservation);
        assert_eq!(
            budget.used(),
            256,
            "shared owner still holds its reservation"
        );
        drop(shared);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn reservation_growth_is_atomic_with_respect_to_budget_failure() {
        let budget = Arc::new(MemoryBudget::new(10));
        let reservation = MemoryReservation::try_new(Arc::clone(&budget), 6).unwrap();

        assert!(!reservation.try_grow_by(5));
        assert_eq!(reservation.bytes(), 6);
        assert_eq!(budget.used(), 6);

        assert!(reservation.try_grow_by(4));
        assert_eq!(reservation.bytes(), 10);
        assert_eq!(budget.used(), 10);
        assert!(!reservation.try_grow_by(1));
        assert_eq!(reservation.bytes(), 10);
        assert_eq!(budget.used(), 10);

        drop(reservation);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn concurrent_reservation_growth_never_exceeds_shared_budget() {
        let budget = Arc::new(MemoryBudget::new(100));
        let reservation = Arc::new(MemoryReservation::try_new(Arc::clone(&budget), 20).unwrap());
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let reservation = Arc::clone(&reservation);
                std::thread::spawn(move || reservation.try_grow_by(20))
            })
            .collect();

        let successful_growths = workers
            .into_iter()
            .map(|worker| worker.join().expect("growth worker must not panic"))
            .filter(|grew| *grew)
            .count();

        assert_eq!(successful_growths, 4);
        assert_eq!(reservation.bytes(), 100);
        assert_eq!(budget.used(), 100);

        drop(reservation);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn concurrent_growth_and_shrink_keep_reservation_and_budget_in_sync() {
        let budget = Arc::new(MemoryBudget::new(4096));
        let reservation = Arc::new(MemoryReservation::try_new(Arc::clone(&budget), 1024).unwrap());
        let grower = {
            let reservation = Arc::clone(&reservation);
            std::thread::spawn(move || {
                for _ in 0..2_000 {
                    let _ = reservation.try_grow_by(1);
                }
            })
        };
        let shrinker = {
            let reservation = Arc::clone(&reservation);
            std::thread::spawn(move || {
                for _ in 0..2_000 {
                    reservation.shrink_to(1024);
                }
            })
        };

        grower.join().unwrap();
        shrinker.join().unwrap();
        assert_eq!(budget.used(), reservation.bytes());
        assert!(reservation.bytes() <= budget.limit());
        drop(reservation);
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
    fn budget_handles_limit_boundaries_zero_and_integer_overflow() {
        let budget = MemoryBudget::new(8);

        assert!(budget.try_reserve(7), "N-1 must fit");
        assert_eq!(budget.used(), 7);
        assert_eq!(budget.available(), 1);
        assert!(budget.try_reserve(1), "the exact limit N must fit");
        assert_eq!(budget.used(), 8);
        assert_eq!(budget.available(), 0);
        assert!(!budget.try_reserve(1), "N+1 must be rejected");
        assert!(
            !budget.try_reserve(usize::MAX),
            "addition overflow must be rejected"
        );
        assert_eq!(
            budget.used(),
            8,
            "rejected reservations must not change accounting"
        );

        budget.release(8);
        assert!(
            budget.try_reserve(0),
            "zero-byte reservations are valid at an empty budget"
        );
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.available(), 8);
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
            let reservation = MemoryReservation::try_new(Arc::clone(&budget), bytes).unwrap();
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

    #[test]
    fn concurrent_resource_slot_guards_never_exceed_the_shared_limit() {
        use std::sync::Barrier;

        let budget = Arc::new(MemoryBudget::new(0));
        let barrier = Arc::new(Barrier::new(16));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let budget = Arc::clone(&budget);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let slot =
                        MemoryBudget::try_reserve_resource_slot_guard(Arc::clone(&budget), 4);
                    barrier.wait();
                    let acquired = slot.is_some();
                    drop(slot);
                    acquired
                })
            })
            .collect();

        let acquired = workers
            .into_iter()
            .map(|worker| worker.join().expect("slot worker must not panic"))
            .filter(|acquired| *acquired)
            .count();

        assert_eq!(acquired, 4, "only four concurrent slots may be granted");
        assert_eq!(budget.live_resources(), 0, "all RAII slots must be released");
        assert_eq!(budget.used(), 0, "slot accounting must not affect byte accounting");
    }
}
