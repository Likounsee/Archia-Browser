pub mod filter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub raw: String,
}

impl Url {
    pub fn parse(raw: impl Into<String>) -> Self {
        Self { raw: raw.into() }
    }
}
