use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::memory::{MemoryBudget, MemoryReservation};

const MAX_LIVE_RESOURCES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceId(u64);

impl ResourceId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
pub struct Resource {
    id: ResourceId,
    reservation: MemoryReservation,
    live_resources: Arc<AtomicUsize>,
}

impl Drop for Resource {
    fn drop(&mut self) {
        self.live_resources.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Resource {
    pub fn id(&self) -> ResourceId {
        self.id
    }
    pub fn bytes(&self) -> usize {
        self.reservation.bytes()
    }
}

#[derive(Debug)]
pub struct ResourceManager {
    budget: Arc<MemoryBudget>,
    live_resources: Arc<AtomicUsize>,
    next_id: u64,
}

impl ResourceManager {
    pub fn new(limit: usize) -> Self {
        Self {
            budget: Arc::new(MemoryBudget::new(limit)),
            live_resources: Arc::new(AtomicUsize::new(0)),
            next_id: 1,
        }
    }

    pub fn allocate(&mut self, bytes: usize) -> Option<Resource> {
        // Byte budgets do not constrain zero-byte allocations. Bound live
        // objects independently so callers cannot exhaust memory with
        // arbitrarily many empty resources.
        let mut live = self.live_resources.load(Ordering::Acquire);
        loop {
            if live >= MAX_LIVE_RESOURCES {
                return None;
            }
            match self.live_resources.compare_exchange_weak(
                live,
                live + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => live = actual,
            }
        }

        let Some(reservation) = MemoryReservation::try_new(Arc::clone(&self.budget), bytes) else {
            self.live_resources.fetch_sub(1, Ordering::AcqRel);
            return None;
        };
        let id = ResourceId::new(self.next_id);
        let Some(next_id) = self.next_id.checked_add(1) else {
            self.live_resources.fetch_sub(1, Ordering::AcqRel);
            return None;
        };
        self.next_id = next_id;
        Some(Resource {
            id,
            reservation,
            live_resources: Arc::clone(&self.live_resources),
        })
    }

    pub fn memory(&self) -> &MemoryBudget {
        &self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_id_exhaustion_releases_the_failed_reservation() {
        let mut manager = ResourceManager::new(4096);
        manager.next_id = u64::MAX;

        assert!(manager.allocate(1024).is_none());
        assert_eq!(manager.memory().used(), 0);
        assert_eq!(manager.memory().available(), 4096);
        assert!(manager.memory().try_reserve(4096));
        manager.memory().release(4096);
    }

    #[test]
    fn zero_byte_allocations_are_bounded_and_slots_are_released() {
        let mut manager = ResourceManager::new(0);
        let mut resources = Vec::new();

        for _ in 0..MAX_LIVE_RESOURCES {
            resources.push(manager.allocate(0).expect("slot should be available"));
        }
        assert!(manager.allocate(0).is_none());
        assert_eq!(
            manager.live_resources.load(Ordering::Acquire),
            MAX_LIVE_RESOURCES
        );

        resources.pop();
        assert_eq!(
            manager.live_resources.load(Ordering::Acquire),
            MAX_LIVE_RESOURCES - 1
        );
        let replacement = manager
            .allocate(0)
            .expect("released slot should be reusable");
        assert_eq!(
            manager.live_resources.load(Ordering::Acquire),
            MAX_LIVE_RESOURCES
        );
        drop(replacement);
    }

    #[test]
    fn resources_consume_and_release_budget() {
        let mut manager = ResourceManager::new(4096);
        let resource = manager.allocate(1024).unwrap();
        assert_eq!(resource.bytes(), 1024);
        assert_eq!(manager.memory().used(), 1024);
        drop(resource);
        assert_eq!(manager.memory().used(), 0);
    }
}
