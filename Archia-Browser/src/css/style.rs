use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputedStyle {
    properties: BTreeMap<String, String>,
}

impl ComputedStyle {
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.properties.insert(name.into(), value.into());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str)
    }

    pub fn len(&self) -> usize { self.properties.len() }
}
