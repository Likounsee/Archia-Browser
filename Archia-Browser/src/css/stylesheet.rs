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
                for declaration in parse_declarations(&CssTokenizer::tokenize(inline_style)) {
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
                let resolved = resolve_css_wide_value(name, value, &inherited);
                if local.is_important(name) {
                    computed.set_important(name, resolved);
                } else {
                    computed.set(name, resolved);
                }
            }
            inherited = computed;
        }

        inherited
    }
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Property) {
    let (value, important) = normalize_declaration_value(&declaration.value);
    let name = declaration.name.to_ascii_lowercase();

    let mut apply = |property: String, property_value: String| {
        if important {
            style.set_important(property, property_value);
        } else {
            style.set_if_unimportant(property, property_value);
        }
    };

    match name.as_str() {
        "margin" | "padding" => {
            if let Some(values) = expand_box_shorthand(&name, &value) {
                apply(name.clone(), value.clone());
                let prefix = name.as_str();
                for (property, property_value) in [
                    (format!("{prefix}-top"), values[0].clone()),
                    (format!("{prefix}-right"), values[1].clone()),
                    (format!("{prefix}-bottom"), values[2].clone()),
                    (format!("{prefix}-left"), values[3].clone()),
                ] {
                    apply(property, property_value);
                }
                return;
            }
        }
        _ => {}
    }

    apply(name, value);
}

fn resolve_css_wide_value(name: &str, value: &str, inherited: &ComputedStyle) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "inherit" => inherited
            .get(name)
            .map(str::to_owned)
            .unwrap_or_else(|| initial_value(name).to_owned()),
        "initial" => initial_value(name).to_owned(),
        "unset" => {
            if is_inherited_property(name) {
                inherited
                    .get(name)
                    .map(str::to_owned)
                    .unwrap_or_else(|| initial_value(name).to_owned())
            } else {
                initial_value(name).to_owned()
            }
        }
        _ => value.to_owned(),
    }
}

fn initial_value(name: &str) -> &'static str {
    match name {
        "display" => "inline",
        "color" => "black",
        "background-color" => "transparent",
        "width" | "height" | "min-width" | "max-width" | "min-height" | "max-height" => "auto",
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => "0",
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => "0",
        "border-width"
        | "border-top-width"
        | "border-right-width"
        | "border-bottom-width"
        | "border-left-width" => "0",
        "font-size" => "16px",
        "font-style" => "normal",
        "font-weight" => "normal",
        "line-height" => "normal",
        "text-align" => "start",
        "visibility" => "visible",
        _ => "initial",
    }
}

fn expand_box_shorthand(_name: &str, value: &str) -> Option<[String; 4]> {
    let values = value
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if values.is_empty() || values.len() > 4 {
        return None;
    }
    let expanded = match values.len() {
        1 => [
            values[0].clone(),
            values[0].clone(),
            values[0].clone(),
            values[0].clone(),
        ],
        2 => [
            values[0].clone(),
            values[1].clone(),
            values[0].clone(),
            values[1].clone(),
        ],
        3 => [
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[1].clone(),
        ],
        4 => [
            values[0].clone(),
            values[1].clone(),
            values[2].clone(),
            values[3].clone(),
        ],
        _ => return None,
    };
    Some(expanded)
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
    let compact = trimmed
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let suffix = "!important";

    if compact.len() >= suffix.len()
        && compact[compact.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    {
        let bang = trimmed.rfind('!').unwrap_or(trimmed.len());
        (trimmed[..bang].trim_end().to_owned(), true)
    } else {
        (trimmed.to_owned(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_and_applies_cascade_order() {
        let sheet = StyleSheet::parse("div { color: red; } .card { color: blue; padding: 4px; }");
        let mut node = Node::element("div");
        node.set_attribute("class", "card");

        let style = sheet.compute_style(&node);
        assert_eq!(style.get("color"), Some("blue"));
        assert_eq!(style.get("padding"), Some("4px"));
    }

    #[test]
    fn important_beats_later_non_important_declaration() {
        let sheet = StyleSheet::parse(".card { color: red !important; } .card { color: blue; }");
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
    fn css_wide_values_resolve_against_inheritance_and_initials() {
        let sheet = StyleSheet::parse(
            "body { color: green; margin: 10px; } span { color: inherit; margin: initial; padding: unset; }",
        );
        let body = Node::element("body");
        let span = Node::element("span");
        let path = [&body, &span];

        let style = sheet.compute_style_path(&path);
        assert_eq!(style.get("color"), Some("green"));
        assert_eq!(style.get("margin-top"), Some("0"));
        assert_eq!(style.get("padding-top"), Some("0"));
    }

    #[test]
    fn expands_box_shorthands() {
        let sheet = StyleSheet::parse("div { margin: 1px 2px 3px 4px; padding: 5px 6px; }");
        let node = Node::element("div");
        let style = sheet.compute_style(&node);

        assert_eq!(style.get("margin-top"), Some("1px"));
        assert_eq!(style.get("margin-right"), Some("2px"));
        assert_eq!(style.get("margin-bottom"), Some("3px"));
        assert_eq!(style.get("margin-left"), Some("4px"));
        assert_eq!(style.get("padding-top"), Some("5px"));
        assert_eq!(style.get("padding-right"), Some("6px"));
        assert_eq!(style.get("padding-bottom"), Some("5px"));
        assert_eq!(style.get("padding-left"), Some("6px"));
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
