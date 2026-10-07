use super::{parse_declarations, Property, Selector, Specificity, CssTokenizer};
use crate::html::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Property>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleSheet {
    pub rules: Vec<StyleRule>,
}

impl StyleSheet {
    pub fn parse(input: &str) -> Self {
        let mut rules = Vec::new();
        for block in input.split('}') {
            let Some((selector_text, declaration_text)) = block.split_once('{') else { continue; };
            let selectors = selector_text
                .split(',')
                .filter_map(Selector::parse)
                .collect::<Vec<_>>();
            if selectors.is_empty() { continue; }
            let declarations = parse_declarations(&CssTokenizer::tokenize(declaration_text));
            rules.push(StyleRule { selectors, declarations });
        }
        Self { rules }
    }

    pub fn matching_rules<'a>(&'a self, node: &Node) -> Vec<(&'a StyleRule, Specificity)> {
        let mut matches = Vec::new();
        for rule in &self.rules {
            if let Some(specificity) = rule.selectors.iter()
                .filter(|selector| selector.matches(node))
                .map(Selector::specificity)
                .max()
            {
                matches.push((rule, specificity));
            }
        }
        matches
    }

    pub fn compute_style(&self, node: &Node) -> super::ComputedStyle {
        let mut matched = self.matching_rules(node);
        matched.sort_by_key(|(_, specificity)| *specificity);
        let mut style = super::ComputedStyle::default();
        for (rule, _) in matched {
            for declaration in &rule.declarations {
                style.set(declaration.name.to_ascii_lowercase(), declaration.value.clone());
            }
        }
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::NodeKind;

    #[test]
    fn parses_rules_and_applies_cascade_order() {
        let sheet = StyleSheet::parse("div { color: red; } .card { color: blue; padding: 4px; }");
        let mut node = Node::element("div");
        if let NodeKind::Element { attributes, .. } = &mut node.kind {
            attributes.insert("class".into(), "card".into());
        }
        let style = sheet.compute_style(&node);
        assert_eq!(style.get("color"), Some("blue"));
        assert_eq!(style.get("padding"), Some("4px"));
    }
}
