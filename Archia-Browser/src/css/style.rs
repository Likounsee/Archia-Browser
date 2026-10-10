use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
}

fn normalize_property_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
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
        let name = normalize_property_name(&name.into());
        self.properties.insert(name.clone(), value.into());
        self.important.insert(name, true);
    }

    pub fn set_if_unimportant(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = normalize_property_name(&name.into());
        if self.important.get(&name).copied().unwrap_or(false) {
            return;
        }
        self.properties.insert(name.clone(), value.into());
        self.important.entry(name).or_insert(false);
    }

    pub fn is_important(&self, name: &str) -> bool {
        self.important
            .get(&normalize_property_name(name))
            .copied()
            .unwrap_or(false)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.properties
            .get(&normalize_property_name(name))
            .map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.properties.len()
    }

    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.properties
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_names_are_case_insensitive_and_trimmed() {
        let mut style = ComputedStyle::default();
        style.set("  BACKGROUND-COLOR  ", "red");
        assert_eq!(style.get("background-color"), Some("red"));
        assert_eq!(style.get("BACKGROUND-COLOR"), Some("red"));
        assert_eq!(style.len(), 1);
    }

    #[test]
    fn important_state_uses_normalized_property_names() {
        let mut style = ComputedStyle::default();
        style.set_important(" COLOR ", "red");
        style.set_if_unimportant("color", "blue");
        assert_eq!(style.get("color"), Some("red"));
        assert!(style.is_important(" COLOR "));
    }
}
