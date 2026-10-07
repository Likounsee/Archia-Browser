use std::sync::Arc;

use super::memory::{MemoryBudget, MemoryReservation};

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
        Self {
            budget: Arc::new(MemoryBudget::new(limit)),
            next_id: 1,
        }
    }

    pub fn allocate(&mut self, bytes: usize) -> Option<Resource> {
        let reservation = MemoryReservation::try_new(Arc::clone(&self.budget), bytes)?;
        let id = ResourceId::new(self.next_id);
        self.next_id = self.next_id.checked_add(1)?;
        Some(Resource { id, reservation })
    }

    pub fn memory(&self) -> &MemoryBudget {
        &self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
