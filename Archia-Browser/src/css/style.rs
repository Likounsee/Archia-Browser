use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputedStyle {
    properties: BTreeMap<String, String>,
    important: BTreeMap<String, bool>,
}

impl ComputedStyle {
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.set_if_unimportant(name, value);
    }

    pub fn set_important(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.properties.insert(name.clone(), value.into());
        self.important.insert(name, true);
    }

    pub fn set_if_unimportant(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        if self.important.get(&name).copied().unwrap_or(false) {
            return;
        }
        self.properties.insert(name.clone(), value.into());
        self.important.entry(name).or_insert(false);
    }

    pub fn is_important(&self, name: &str) -> bool {
        self.important.get(name).copied().unwrap_or(false)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.properties.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.properties
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}
