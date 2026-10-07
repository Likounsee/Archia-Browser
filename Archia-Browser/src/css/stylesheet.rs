use super::{parse_declarations, ComputedStyle, CssTokenizer, Property, Selector, Specificity};
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
            let Some((selector_text, declaration_text)) = block.split_once('{') else {
                continue;
            };

            let selectors = selector_text
                .split(',')
                .filter_map(Selector::parse)
                .collect::<Vec<_>>();
            if selectors.is_empty() {
                continue;
            }

            let declarations = parse_declarations(&CssTokenizer::tokenize(declaration_text));
            rules.push(StyleRule {
                selectors,
                declarations,
            });
        }
        Self { rules }
    }

    pub fn matching_rules<'a>(&'a self, node: &Node) -> Vec<(&'a StyleRule, Specificity)> {
        self.matching_rules_path(&[node])
    }

    pub fn matching_rules_path<'a>(&'a self, path: &[&Node]) -> Vec<(&'a StyleRule, Specificity)> {
        let mut matches = Vec::new();
        let Some(node) = path.last() else {
            return matches;
        };

        for rule in &self.rules {
            let matching = rule.selectors.iter().filter(|selector| {
                if selector.parts.len() == 1 {
                    selector.matches(node)
                } else {
                    selector.matches_path(path)
                }
            });
            if let Some(specificity) = matching.map(Selector::specificity).max() {
                matches.push((rule, specificity));
            }
        }
        matches
    }

    pub fn compute_style(&self, node: &Node) -> ComputedStyle {
        self.compute_style_path(&[node])
    }

    pub fn compute_style_path(&self, path: &[&Node]) -> ComputedStyle {
        let mut inherited = ComputedStyle::default();

        for index in 0..path.len() {
            let mut matched = self.matching_rules_path(&path[..=index]);
            matched.sort_by_key(|(_, specificity)| *specificity);

            let mut local = ComputedStyle::default();
            for (rule, _) in matched {
                for declaration in &rule.declarations {
                    apply_declaration(&mut local, declaration);
                }
            }

            if let Some(inline_style) = path[index].attribute("style") {
                for declaration in
                    parse_declarations(&CssTokenizer::tokenize(inline_style))
                {
                    apply_declaration(&mut local, &declaration);
                }
            }

            let mut computed = ComputedStyle::default();
            for (name, value) in inherited.iter() {
                if is_inherited_property(name) {
                    computed.set(name, value);
                }
            }
            for (name, value) in local.iter() {
                computed.set(name, value);
            }
            inherited = computed;
        }

        inherited
    }
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Property) {
    let (value, important) = normalize_declaration_value(&declaration.value);
    let name = declaration.name.to_ascii_lowercase();

    if important {
        style.set_important(name, value);
    } else {
        style.set_if_unimportant(name, value);
    }
}

fn is_inherited_property(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "font"
            | "font-family"
            | "font-size"
            | "font-style"
            | "font-variant"
            | "font-weight"
            | "line-height"
            | "letter-spacing"
            | "word-spacing"
            | "text-align"
            | "text-indent"
            | "text-transform"
            | "visibility"
            | "white-space"
            | "cursor"
    )
}

fn normalize_declaration_value(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let suffix = "!important";
    if trimmed.len() >= suffix.len()
        && trimmed[trimmed.len() - suffix.len()..]
            .eq_ignore_ascii_case(suffix)
    {
        (
            trimmed[..trimmed.len() - suffix.len()]
                .trim_end()
                .to_owned(),
            true,
        )
    } else {
        (trimmed.to_owned(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_and_applies_cascade_order() {
        let sheet =
            StyleSheet::parse("div { color: red; } .card { color: blue; padding: 4px; }");
        let mut node = Node::element("div");
        node.set_attribute("class", "card");

        let style = sheet.compute_style(&node);
        assert_eq!(style.get("color"), Some("blue"));
        assert_eq!(style.get("padding"), Some("4px"));
    }

    #[test]
    fn important_beats_later_non_important_declaration() {
        let sheet =
            StyleSheet::parse(".card { color: red !important; } .card { color: blue; }");
        let mut node = Node::element("div");
        node.set_attribute("class", "card");

        let style = sheet.compute_style(&node);
        assert_eq!(style.get("color"), Some("red"));
        assert!(style.is_important("color"));
    }

    #[test]
    fn inline_style_overrides_normal_stylesheet_declarations() {
        let sheet = StyleSheet::parse(".card { color: red; padding: 2px; }");
        let mut node = Node::element("div");
        node.set_attribute("class", "card");
        node.set_attribute("style", "color: blue; padding: 8px !important;");

        let style = sheet.compute_style(&node);
        assert_eq!(style.get("color"), Some("blue"));
        assert_eq!(style.get("padding"), Some("8px"));
        assert!(style.is_important("padding"));
    }

    #[test]
    fn inherited_properties_reach_descendants() {
        let sheet = StyleSheet::parse("body { color: green; margin: 10px; }");
        let body = Node::element("body");
        let child = Node::element("span");
        let path = [&body, &child];

        let style = sheet.compute_style_path(&path);
        assert_eq!(style.get("color"), Some("green"));
        assert_eq!(style.get("margin"), None);
    }
}
