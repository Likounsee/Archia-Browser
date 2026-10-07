pub mod core;
pub mod platform;
pub mod security;
pub mod net;

pub struct Browser {
    version: &'static str,
}

impl Browser {
    pub const fn new() -> Self {
        Self { version: env!("CARGO_PKG_VERSION") }
    }

    pub const fn version(&self) -> &'static str {
        self.version
    }
}
