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
    budget: Arc<MemoryBudget>,
}

impl Drop for Resource {
    fn drop(&mut self) {
        self.budget.release_resource_slot();
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
    next_id: u64,
}

impl ResourceManager {
    pub fn new(limit: usize) -> Self {
        Self::with_budget(Arc::new(MemoryBudget::new(limit)))
    }

    /// Create a resource manager that shares accounting with other browser subsystems.
    pub fn with_budget(budget: Arc<MemoryBudget>) -> Self {
        Self {
            budget,
            next_id: 1,
        }
    }

    pub fn allocate(&mut self, bytes: usize) -> Option<Resource> {
        // Byte budgets do not constrain zero-byte allocations. The object
        // quota lives in MemoryBudget so all managers sharing it share one cap.
        if !self.budget.try_reserve_resource_slot(MAX_LIVE_RESOURCES) {
            return None;
        }

        let Some(reservation) = MemoryReservation::try_new(Arc::clone(&self.budget), bytes) else {
            self.budget.release_resource_slot();
            return None;
        };
        let id = ResourceId::new(self.next_id);
        let Some(next_id) = self.next_id.checked_add(1) else {
            self.budget.release_resource_slot();
            return None;
        };
        self.next_id = next_id;
        Some(Resource {
            id,
            reservation,
            budget: Arc::clone(&self.budget),
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
    fn resource_manager_can_share_an_existing_memory_budget() {
        let budget = Arc::new(MemoryBudget::new(1024));
        let mut manager = ResourceManager::with_budget(Arc::clone(&budget));

        let resource = manager.allocate(768).unwrap();
        assert_eq!(budget.used(), 768);
        assert_eq!(manager.memory().available(), 256);
        drop(resource);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn resource_id_exhaustion_releases_the_failed_reservation() {
        let mut manager = ResourceManager::new(4096);
        manager.next_id = u64::MAX;

        assert!(manager.allocate(1024).is_none());
        assert_eq!(manager.memory().used(), 0);
        assert_eq!(manager.memory().available(), 4096);
        assert_eq!(
            manager.budget.live_resources(),
            0,
            "an exhausted resource ID must release its live-object slot"
        );
        assert!(manager.memory().try_reserve(4096));
        manager.memory().release(4096);
    }

    #[test]
    fn failed_memory_reservation_does_not_leak_live_resource_slots() {
        let mut manager = ResourceManager::new(0);

        for _ in 0..MAX_LIVE_RESOURCES * 2 {
            assert!(manager.allocate(1).is_none());
        }

        assert_eq!(manager.memory().used(), 0);
        assert_eq!(
            manager.budget.live_resources(),
            0,
            "failed byte-budget reservations must release the live-object slot"
        );

        let resource = manager
            .allocate(0)
            .expect("a failed reservation must not exhaust object slots");
        assert_eq!(manager.live_resources.load(Ordering::Acquire), 1);
        drop(resource);
        assert_eq!(manager.live_resources.load(Ordering::Acquire), 0);
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
