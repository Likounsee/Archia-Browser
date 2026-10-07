pub mod origin;

pub use origin::Origin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityOrigin {
    Opaque,
    SameOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    pub name: &'static str,
    pub granted: bool,
}
