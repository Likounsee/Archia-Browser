use std::sync::Arc;

use super::memory::{MemoryBudget, MemoryReservation, MemoryResourceSlot, MAX_LIVE_BUDGETED_OBJECTS};

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
    _slot: MemoryResourceSlot,
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
        Self { budget, next_id: 1 }
    }

    pub fn allocate(&mut self, bytes: usize) -> Option<Resource> {
        // Byte budgets do not constrain zero-byte allocations. The object
        // quota lives in MemoryBudget so all managers sharing it share one cap.
        let slot = MemoryBudget::try_reserve_resource_slot_guard(
            Arc::clone(&self.budget),
            MAX_LIVE_BUDGETED_OBJECTS,
        )?;

        // The slot guard and byte reservation both roll back automatically if
        // a later allocation or ID check fails.
        let reservation = MemoryReservation::try_new(Arc::clone(&self.budget), bytes)?;
        let id = ResourceId::new(self.next_id);
        let next_id = self.next_id.checked_add(1)?;
        self.next_id = next_id;
        Some(Resource {
            id,
            reservation,
            _slot: slot,
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

        for _ in 0..MAX_LIVE_BUDGETED_OBJECTS * 2 {
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
        assert_eq!(manager.budget.live_resources(), 1);
        drop(resource);
        assert_eq!(manager.budget.live_resources(), 0);
    }

    #[test]
    fn zero_byte_allocations_are_bounded_and_slots_are_released() {
        let mut manager = ResourceManager::new(0);
        let mut resources = Vec::new();

        for _ in 0..MAX_LIVE_BUDGETED_OBJECTS {
            resources.push(manager.allocate(0).expect("slot should be available"));
        }
        assert!(manager.allocate(0).is_none());
        assert_eq!(manager.budget.live_resources(), MAX_LIVE_BUDGETED_OBJECTS);

        resources.pop();
        assert_eq!(
            manager.budget.live_resources(),
            MAX_LIVE_BUDGETED_OBJECTS - 1
        );
        let replacement = manager
            .allocate(0)
            .expect("released slot should be reusable");
        assert_eq!(manager.budget.live_resources(), MAX_LIVE_BUDGETED_OBJECTS);
        drop(replacement);
    }

    #[test]
    fn shared_budget_enforces_live_resource_limit_across_managers() {
        let budget = Arc::new(MemoryBudget::new(0));
        let mut first = ResourceManager::with_budget(Arc::clone(&budget));
        let mut second = ResourceManager::with_budget(Arc::clone(&budget));
        let mut resources = Vec::with_capacity(MAX_LIVE_BUDGETED_OBJECTS);

        for _ in 0..MAX_LIVE_BUDGETED_OBJECTS {
            resources.push(first.allocate(0).expect("shared slot should be available"));
        }
        assert!(second.allocate(0).is_none());
        assert_eq!(budget.live_resources(), MAX_LIVE_BUDGETED_OBJECTS);

        drop(resources.pop());
        let replacement = second
            .allocate(0)
            .expect("a slot released by another manager must be reusable");
        assert_eq!(budget.live_resources(), MAX_LIVE_BUDGETED_OBJECTS);
        drop(replacement);
        drop(resources);
        assert_eq!(budget.live_resources(), 0);
    }

    #[test]
    fn concurrent_managers_cannot_overrun_shared_live_resource_quota() {
        const WORKERS: usize = 16;
        let budget = Arc::new(MemoryBudget::new(0));
        let workers: Vec<_> = (0..WORKERS)
            .map(|_| {
                let budget = Arc::clone(&budget);
                std::thread::spawn(move || {
                    let mut manager = ResourceManager::with_budget(budget);
                    let mut resources = Vec::new();
                    while let Some(resource) = manager.allocate(0) {
                        resources.push(resource);
                    }
                    resources
                })
            })
            .collect();

        let resources: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().expect("resource worker must not panic"))
            .collect();
        let allocated: usize = resources.iter().map(Vec::len).sum();

        assert_eq!(allocated, MAX_LIVE_BUDGETED_OBJECTS);
        assert_eq!(budget.live_resources(), MAX_LIVE_BUDGETED_OBJECTS);
        drop(resources);
        assert_eq!(budget.live_resources(), 0);
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
