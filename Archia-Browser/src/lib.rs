pub mod core;
pub mod css;
pub mod html;
pub mod net;
pub mod platform;
pub mod security;

pub struct Browser {
    version: &'static str,
    memory: core::memory::MemoryBudget,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            memory: core::memory::MemoryBudget::new(512 * 1024 * 1024),
        }
    }

    pub const fn version(&self) -> &'static str { self.version }
    pub fn memory(&self) -> &core::memory::MemoryBudget { &self.memory }
}

impl Default for Browser {
    fn default() -> Self { Self::new() }
}
