use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryPressure {
    Normal = 0,
    Moderate = 1,
    Critical = 2,
}

#[derive(Debug, Default)]
pub struct MemoryPressureState {
    level: AtomicU8,
}

impl MemoryPressureState {
    pub const fn new() -> Self {
        Self {
            level: AtomicU8::new(MemoryPressure::Normal as u8),
        }
    }

    pub fn set(&self, pressure: MemoryPressure) {
        self.level.store(pressure as u8, Ordering::Release);
    }

    pub fn get(&self) -> MemoryPressure {
        match self.level.load(Ordering::Acquire) {
            1 => MemoryPressure::Moderate,
            2 => MemoryPressure::Critical,
            _ => MemoryPressure::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_state_is_observable() {
        let state = MemoryPressureState::new();
        assert_eq!(state.get(), MemoryPressure::Normal);
        state.set(MemoryPressure::Critical);
        assert_eq!(state.get(), MemoryPressure::Critical);
    }
}
