use super::{parse_declarations, ComputedStyle, CssTokenizer, Property, Selector, Specificity};
use crate::html::Node;

const MAX_STYLESHEET_BYTES: usize = 256 * 1024;
const MAX_STYLE_RULES: usize = 256;
const MAX_SELECTOR_BYTES: usize = 512;
const MAX_SELECTORS_PER_RULE: usize = 32;
const MAX_TOTAL_SELECTORS: usize = 64;
const MAX_SELECTOR_PARTS: usize = 16;

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
        // CSS can be supplied directly by callers as well as through the
        // document loader. Bound both scanned bytes and retained rule/selector
        // complexity so a hostile stylesheet cannot multiply work per DOM node.
        let input = if input.len() > MAX_STYLESHEET_BYTES {
            let mut end = MAX_STYLESHEET_BYTES;
            while !input.is_char_boundary(end) {
                end -= 1;
            }
            &input[..end]
        } else {
            input
        };
        let mut rules = Vec::new();
        let mut total_selectors = 0usize;
        for (selector_text, declaration_text) in
            parse_rule_blocks(input).into_iter().take(MAX_STYLE_RULES)
        {
            if total_selectors >= MAX_TOTAL_SELECTORS {
                break;
            }
            // Nested at-rules (for example @media) are intentionally ignored until
            // the engine implements their conditions; never misinterpret them as selectors.
            if selector_text.trim_start().starts_with('@')
                || selector_text.len() > MAX_SELECTOR_BYTES
            {
                continue;
            }

            let selectors = split_selector_list(
                &selector_text,
                MAX_SELECTORS_PER_RULE.min(MAX_TOTAL_SELECTORS - total_selectors),
            )
            .into_iter()
            .filter(|selector| selector.len() <= MAX_SELECTOR_BYTES)
            .filter_map(|selector| Selector::parse(&selector))
            .filter(|selector| selector.parts.len() <= MAX_SELECTOR_PARTS)
            .collect::<Vec<_>>();
            if selectors.is_empty() {
                continue;
            }
            total_selectors += selectors.len();

            let declarations = parse_declarations(&CssTokenizer::tokenize(&declaration_text));
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

    pub fn matching_rules_path_with_siblings<'a>(
        &'a self,
        path: &[&Node],
        sibling_lists: &[&[Node]],
        sibling_positions: &[usize],
    ) -> Vec<(&'a StyleRule, Specificity)> {
        let Some(node) = path.last() else {
            return Vec::new();
        };
        let mut matches = Vec::new();
        for rule in &self.rules {
            let matching = rule.selectors.iter().filter(|selector| {
                if selector.parts.len() == 1 {
                    selector.matches(node)
                } else {
                    selector.matches_path_with_siblings(path, sibling_lists, sibling_positions)
                }
            });
            if let Some(specificity) = matching.map(Selector::specificity).max() {
                matches.push((rule, specificity));
            }
        }
        matches
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

    pub fn compute_style_path_with_siblings(
        &self,
        path: &[&Node],
        sibling_lists: &[&[Node]],
        sibling_positions: &[usize],
    ) -> ComputedStyle {
        let mut inherited = ComputedStyle::default();

        for index in 0..path.len() {
            inherited = self.compute_style_node_with_siblings(
                &path[..=index],
                &sibling_lists[..=index],
                &sibling_positions[..=index],
                &inherited,
            );
        }

        inherited
    }

    /// Compute only the current node's style, inheriting from its already
    /// computed parent. Recomputing every ancestor for every node makes
    /// style resolution grow with the sum of all ancestor-path lengths.
    pub(crate) fn compute_style_node_with_siblings(
        &self,
        path: &[&Node],
        sibling_lists: &[&[Node]],
        sibling_positions: &[usize],
        inherited: &ComputedStyle,
    ) -> ComputedStyle {
        let Some(node) = path.last() else {
            return inherited.clone();
        };
        let mut matched =
            self.matching_rules_path_with_siblings(path, sibling_lists, sibling_positions);
        matched.sort_by_key(|(_, specificity)| *specificity);

        let mut local = ComputedStyle::default();
        for (rule, _) in matched {
            for declaration in &rule.declarations {
                apply_declaration(&mut local, declaration);
            }
        }

        if let Some(inline_style) = node.attribute("style") {
            for declaration in parse_declarations(&CssTokenizer::tokenize(inline_style)) {
                apply_declaration(&mut local, &declaration);
            }
        }

        let mut computed = ComputedStyle::default();
        for (name, value) in inherited.iter() {
            if is_inherited_property(name) || name.starts_with("--") {
                computed.set(name, value);
            }
        }
        for (name, value) in local.iter() {
            if name.starts_with("--") {
                computed.set(name, value);
            }
        }
        for (name, value) in local.iter() {
            let resolved = resolve_css_value(name, value, &computed, inherited);
            if local.is_important(name) {
                computed.set_important(name, resolved);
            } else {
                computed.set(name, resolved);
            }
        }

        computed
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
                if is_inherited_property(name) || name.starts_with("--") {
                    computed.set(name, value);
                }
            }
            for (name, value) in local.iter() {
                if name.starts_with("--") {
                    computed.set(name, value);
                }
            }
            for (name, value) in local.iter() {
                let resolved = resolve_css_value(name, value, &computed, &inherited);
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

/// Scan only top-level qualified rules. Unlike splitting on braces, this respects
/// comments, quoted strings, escaped characters and nested delimiters.
fn parse_rule_blocks(input: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut prelude = String::new();
    let mut body = String::new();
    let mut in_body = false;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if quote.is_none() && ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous_star = false;
            for comment_char in chars.by_ref() {
                if previous_star && comment_char == '/' {
                    break;
                }
                previous_star = comment_char == '*';
            }
            continue;
        }

        if let Some(active_quote) = quote {
            if in_body {
                body.push(ch);
            } else {
                prelude.push(ch);
            }
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            if in_body {
                body.push(ch);
            } else {
                prelude.push(ch);
            }
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' if paren_depth == 0 && bracket_depth == 0 => {
                if in_body {
                    brace_depth += 1;
                    body.push(ch);
                } else {
                    in_body = true;
                    brace_depth = 1;
                    body.clear();
                }
                continue;
            }
            '}' if in_body && paren_depth == 0 && bracket_depth == 0 => {
                brace_depth = brace_depth.saturating_sub(1);
                if brace_depth == 0 {
                    let selector = prelude.trim().to_owned();
                    if !selector.is_empty() {
                        blocks.push((selector, std::mem::take(&mut body)));
                    }
                    prelude.clear();
                    body.clear();
                    in_body = false;
                } else {
                    body.push(ch);
                }
                continue;
            }
            ';' if !in_body && paren_depth == 0 && bracket_depth == 0 => {
                // Drop top-level at-rule statements such as @import; the document
                // loader handles supported imports separately.
                prelude.clear();
                continue;
            }
            _ => {}
        }

        if in_body {
            body.push(ch);
        } else {
            prelude.push(ch);
        }
    }

    blocks
}

/// Split selector groups only on commas outside strings, attribute selectors and
/// functional pseudo-class arguments (e.g. :is(.a, .b)).
fn split_selector_list(input: &str, max_selectors: usize) -> Vec<String> {
    let mut selectors = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for ch in input.chars() {
        if let Some(active_quote) = quote {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 => {
                let selector = current.trim();
                if !selector.is_empty() {
                    selectors.push(selector.to_owned());
                    if selectors.len() >= max_selectors {
                        return selectors;
                    }
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    let selector = current.trim();
    if !selector.is_empty() && selectors.len() < max_selectors {
        selectors.push(selector.to_owned());
    }
    selectors
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
        "border" => {
            if let Some((width, style, color)) = expand_border_shorthand(&value) {
                apply("border-width".to_owned(), width.to_owned());
                apply("border-style".to_owned(), style.to_owned());
                apply("border-color".to_owned(), color.to_owned());
                return;
            }
        }
        _ => {}
    }

    apply(name, value);
}

fn resolve_css_value(
    name: &str,
    value: &str,
    variables: &ComputedStyle,
    inherited: &ComputedStyle,
) -> String {
    if value.len() > MAX_RESOLVED_CSS_VALUE_BYTES {
        return initial_value(name).to_owned();
    }
    let wide = match value.trim().to_ascii_lowercase().as_str() {
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
    };
    resolve_variables(&wide, variables, 0)
}

const MAX_RESOLVED_CSS_VALUE_BYTES: usize = 64 * 1024;

fn resolve_variables(value: &str, variables: &ComputedStyle, depth: usize) -> String {
    // Variable substitution can amplify a short declaration into a huge string
    // when custom properties recursively repeat other properties.
    if depth >= 16 || value.len() > MAX_RESOLVED_CSS_VALUE_BYTES {
        return value.to_owned();
    }
    let Some(start) = value.find("var(") else {
        return value.to_owned();
    };
    let mut level = 0usize;
    let mut end = None;
    for (offset, character) in value[start + 4..].char_indices() {
        match character {
            '(' => level += 1,
            ')' if level == 0 => {
                end = Some(start + 4 + offset);
                break;
            }
            ')' => level -= 1,
            _ => {}
        }
    }
    let Some(end) = end else {
        return value.to_owned();
    };
    let inside = &value[start + 4..end];
    let mut parts = inside.splitn(2, ',');
    let name = parts.next().unwrap_or("").trim();
    let replacement = variables
        .get(name)
        .map(str::to_owned)
        .or_else(|| parts.next().map(str::trim).map(str::to_owned));
    let Some(replacement) = replacement else {
        return value.to_owned();
    };

    let resulting_len = value
        .len()
        .saturating_sub(end + 1 - start)
        .saturating_add(replacement.len());
    if resulting_len > MAX_RESOLVED_CSS_VALUE_BYTES {
        return value.to_owned();
    }

    let replacement = resolve_variables(&replacement, variables, depth + 1);
    let mut output = String::with_capacity(resulting_len);
    output.push_str(&value[..start]);
    output.push_str(&replacement);
    output.push_str(&value[end + 1..]);
    resolve_variables(&output, variables, depth + 1)
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

fn expand_border_shorthand(value: &str) -> Option<(String, String, String)> {
    let mut width = None;
    let mut style = None;
    let mut color = None;

    for token in value.split_whitespace() {
        if width.is_none() && matches!(token, "thin" | "medium" | "thick") {
            width = Some(token.to_owned());
        } else if width.is_none() && parse_px_token(token).is_some() {
            width = Some(token.to_owned());
        } else if style.is_none()
            && matches!(
                token.to_ascii_lowercase().as_str(),
                "none"
                    | "hidden"
                    | "dotted"
                    | "dashed"
                    | "solid"
                    | "double"
                    | "groove"
                    | "ridge"
                    | "inset"
                    | "outset"
            )
        {
            style = Some(token.to_owned());
        } else if color.is_none() {
            color = Some(token.to_owned());
        } else {
            return None;
        }
    }

    Some((
        width.unwrap_or_else(|| "medium".to_owned()),
        style.unwrap_or_else(|| "none".to_owned()),
        color.unwrap_or_else(|| "currentcolor".to_owned()),
    ))
}

fn parse_px_token(value: &str) -> Option<u32> {
    value.strip_suffix("px")?.trim().parse().ok()
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
    fn matching_rules_with_siblings_handles_empty_path() {
        let sheet = StyleSheet::parse("div { color: red; }");
        assert!(sheet
            .matching_rules_path_with_siblings(&[], &[], &[])
            .is_empty());
    }

    #[test]
    fn custom_property_expansion_is_bounded() {
        let mut variables = ComputedStyle::default();
        variables.set("--large", "x".repeat(40 * 1024));
        let value = "var(--large)var(--large)";

        let resolved = resolve_variables(value, &variables, 0);

        assert!(resolved.len() <= MAX_RESOLVED_CSS_VALUE_BYTES);
        assert!(resolved.contains("var(--large)"));
    }

    #[test]
    fn stylesheet_parser_caps_total_selector_count_and_parts() {
        let rule = (0..MAX_SELECTORS_PER_RULE)
            .map(|index| format!(".item{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let input = (0..8)
            .map(|_| format!("{rule} {{ color: red; }}"))
            .collect::<Vec<_>>()
            .join(" ");
        let sheet = StyleSheet::parse(&input);
        let total = sheet
            .rules
            .iter()
            .map(|rule| rule.selectors.len())
            .sum::<usize>();

        assert_eq!(total, MAX_TOTAL_SELECTORS);

        let too_complex = format!(
            "{} {{ color: red; }}",
            (0..MAX_SELECTOR_PARTS + 2)
                .map(|_| "div")
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(StyleSheet::parse(&too_complex).rules.is_empty());
    }

    #[test]
    fn stylesheet_parser_caps_rule_count() {
        let input = ".item { color: red; }".repeat(MAX_STYLE_RULES + 20);
        let sheet = StyleSheet::parse(&input);
        assert_eq!(sheet.rules.len(), MAX_STYLE_RULES.min(MAX_TOTAL_SELECTORS));
    }

    #[test]
    fn stylesheet_parser_caps_direct_input_bytes() {
        let input = format!(
            "{} .late {{ color: red; }}",
            " ".repeat(MAX_STYLESHEET_BYTES)
        );
        let sheet = StyleSheet::parse(&input);
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn stylesheet_parser_caps_selector_list_complexity() {
        let selectors = (0..MAX_SELECTORS_PER_RULE + 10)
            .map(|index| format!(".item{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sheet = StyleSheet::parse(&format!("{selectors} {{ color: red; }}"));

        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), MAX_SELECTORS_PER_RULE);
    }

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
    fn expands_border_shorthand_into_painting_properties() {
        let sheet = StyleSheet::parse("div { border: 2px solid #102030; }");
        let node = Node::element("div");
        let style = sheet.compute_style(&node);

        assert_eq!(style.get("border-width"), Some("2px"));
        assert_eq!(style.get("border-style"), Some("solid"));
        assert_eq!(style.get("border-color"), Some("#102030"));
    }

    #[test]
    fn custom_properties_inherit_and_resolve_var_functions() {
        let sheet = StyleSheet::parse(
            "body { --accent: #123456; color: var(--accent); } span { background-color: var(--missing, var(--accent)); }",
        );
        let body = Node::element("body");
        let span = Node::element("span");
        let path = [&body, &span];

        let style = sheet.compute_style_path(&path);
        assert_eq!(style.get("--accent"), Some("#123456"));
        assert_eq!(style.get("color"), Some("#123456"));
        assert_eq!(style.get("background-color"), Some("#123456"));
    }

    #[test]
    fn parses_comments_and_braces_inside_quoted_values() {
        let sheet = StyleSheet::parse(
            "/* leading } comment */ .card { content: \"a } { /* not a comment */\"; color: red; } /* trailing { */ .next { color: blue; }",
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors.len(), 1);
        assert_eq!(
            sheet.rules[0]
                .declarations
                .iter()
                .find(|p| p.name == "color")
                .map(|p| p.value.as_str()),
            Some("red")
        );
        assert_eq!(
            sheet.rules[1]
                .declarations
                .iter()
                .find(|p| p.name == "color")
                .map(|p| p.value.as_str()),
            Some("blue")
        );
    }

    #[test]
    fn selector_commas_inside_function_arguments_do_not_split_rule_groups() {
        let sheet = StyleSheet::parse(":is(.card, .panel), .fallback { color: red; }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert!(sheet.rules[0]
            .selectors
            .iter()
            .any(|selector| selector.parts[0]
                .1
                .pseudo_classes
                .iter()
                .any(|p| p.starts_with("is("))));
        assert!(sheet.rules[0]
            .selectors
            .iter()
            .any(|selector| selector.parts[0].1.classes.contains(&"fallback".to_owned())));
    }

    #[test]
    fn ignores_unclosed_rules_instead_of_merging_them_into_following_rules() {
        let sheet = StyleSheet::parse(".broken { color: red; .valid { color: blue; }");
        assert!(sheet.rules.is_empty());
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
