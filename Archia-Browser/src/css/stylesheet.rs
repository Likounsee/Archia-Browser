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
        Self::parse_for_viewport(input, 1024, 768)
    }

    /// Parse a stylesheet using the target document viewport for media queries.
    /// Unsupported media features fail closed rather than applying rules globally.
    pub fn parse_for_viewport(input: &str, viewport_width: u32, viewport_height: u32) -> Self {
        Self::parse_internal(input, viewport_width, viewport_height, 0)
    }

    fn parse_internal(
        input: &str,
        viewport_width: u32,
        viewport_height: u32,
        depth: usize,
    ) -> Self {
        if depth > 8 {
            return Self::default();
        }
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
        for (selector_text, declaration_text) in parse_rule_blocks(input, MAX_STYLE_RULES) {
            if rules.len() >= MAX_STYLE_RULES || total_selectors >= MAX_TOTAL_SELECTORS {
                break;
            }
            let prelude = selector_text.trim();
            let conditional_matches = if let Some(condition) = prelude.strip_prefix("@media") {
                media_query_matches(condition.trim(), viewport_width, viewport_height)
            } else if let Some(condition) = prelude.strip_prefix("@supports") {
                supports_condition_matches(condition.trim())
            } else {
                false
            };
            if prelude.starts_with("@media") || prelude.starts_with("@supports") {
                if conditional_matches {
                    let nested = Self::parse_internal(
                        &declaration_text,
                        viewport_width,
                        viewport_height,
                        depth + 1,
                    );
                    for rule in nested.rules {
                        if rules.len() >= MAX_STYLE_RULES {
                            break;
                        }
                        total_selectors = total_selectors.saturating_add(rule.selectors.len());
                        if total_selectors > MAX_TOTAL_SELECTORS {
                            break;
                        }
                        if rules.try_reserve(1).is_err() {
                            return Self { rules };
                        }
                        rules.push(rule);
                    }
                }
                continue;
            }
            // Other at-rules are deliberately ignored until their semantics are
            // implemented; never misinterpret them as selectors.
            if prelude.starts_with('@') || selector_text.len() > MAX_SELECTOR_BYTES {
                continue;
            }

            let selector_texts = split_selector_list(
                &selector_text,
                MAX_SELECTORS_PER_RULE.min(MAX_TOTAL_SELECTORS - total_selectors),
            );
            let mut selectors = Vec::new();
            for selector_text in selector_texts {
                if selector_text.len() > MAX_SELECTOR_BYTES {
                    continue;
                }
                let Some(selector) = Selector::parse(&selector_text) else {
                    continue;
                };
                if selector.parts.len() > MAX_SELECTOR_PARTS || selectors.try_reserve(1).is_err() {
                    continue;
                }
                selectors.push(selector);
            }
            if selectors.is_empty() {
                continue;
            }
            total_selectors += selectors.len();

            let declarations = parse_declarations(&CssTokenizer::tokenize(&declaration_text));
            if rules.try_reserve(1).is_err() {
                break;
            }
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
        if path.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        for rule in &self.rules {
            let matching = rule.selectors.iter().filter(|selector| {
                selector.matches_path_with_siblings(path, sibling_lists, sibling_positions)
            });
            if let Some(specificity) = matching.map(Selector::specificity).max() {
                if matches.try_reserve(1).is_err() {
                    return matches;
                }
                matches.push((rule, specificity));
            }
        }
        matches
    }

    pub fn matching_rules_path<'a>(&'a self, path: &[&Node]) -> Vec<(&'a StyleRule, Specificity)> {
        let mut matches = Vec::new();
        if path.is_empty() {
            return matches;
        }

        for rule in &self.rules {
            let matching = rule
                .selectors
                .iter()
                .filter(|selector| selector.matches_path(path));
            if let Some(specificity) = matching.map(Selector::specificity).max() {
                if matches.try_reserve(1).is_err() {
                    return matches;
                }
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

/// Evaluate the supported subset of CSS Conditional Rules declaration conditions.
fn supports_condition_matches(condition: &str) -> bool {
    let condition = condition.trim();
    if let Some(negated) = condition.strip_prefix("not ") {
        return !supports_condition_matches(negated);
    }
    if let Some(terms) = split_supports_expression(condition, "or") {
        return terms.iter().any(|term| supports_condition_matches(term));
    }
    if let Some(terms) = split_supports_expression(condition, "and") {
        return terms.iter().all(|term| supports_condition_matches(term));
    }

    let parenthesized = condition.starts_with('(') && condition.ends_with(')');
    let inner = if parenthesized {
        let Some(inner) = condition
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
        else {
            return false;
        };
        inner.trim()
    } else {
        condition
    };

    // selector() is a complete feature query; declaration queries still
    // require the surrounding parentheses from the CSS @supports grammar.
    if let Some(selector) = inner
        .strip_prefix("selector(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return !selector.trim().is_empty() && Selector::parse(selector.trim()).is_some();
    }
    if !parenthesized {
        return false;
    }

    let Some((name, value)) = inner.split_once(':') else {
        return false;
    };
    supports_declaration(name.trim(), value.trim())
}

fn split_supports_expression<'a>(input: &'a str, operator: &str) -> Option<Vec<&'a str>> {
    let needle = format!(" {operator} ");
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    while cursor < input.len() {
        let character = input[cursor..].chars().next()?;
        if character == '(' {
            depth = depth.saturating_add(1);
        } else if character == ')' {
            depth = depth.saturating_sub(1);
        }
        if depth == 0 && input[cursor..].starts_with(&needle) {
            let part = input[start..cursor].trim();
            if part.is_empty() {
                return None;
            }
            parts.push(part);
            cursor += needle.len();
            start = cursor;
            continue;
        }
        cursor += character.len_utf8();
    }
    if parts.is_empty() {
        return None;
    }
    let tail = input[start..].trim();
    if tail.is_empty() {
        return None;
    }
    parts.push(tail);
    Some(parts)
}

fn supports_declaration(name: &str, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let name = name.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    const IMPLEMENTED_PROPERTIES: &[&str] = &[
        "display",
        "position",
        "box-sizing",
        "width",
        "height",
        "aspect-ratio",
        "min-width",
        "max-width",
        "min-height",
        "max-height",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "margin-block",
        "margin-block-start",
        "margin-block-end",
        "margin-inline",
        "margin-inline-start",
        "margin-inline-end",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "padding-block",
        "padding-block-start",
        "padding-block-end",
        "padding-inline",
        "padding-inline-start",
        "padding-inline-end",
        "border",
        "border-width",
        "border-style",
        "border-color",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
        "border-block",
        "border-block-start",
        "border-block-end",
        "border-inline",
        "border-inline-start",
        "border-inline-end",
        "border-block-start-width",
        "border-block-start-style",
        "border-block-start-color",
        "border-block-end-width",
        "border-block-end-style",
        "border-block-end-color",
        "border-inline-start-width",
        "border-inline-start-style",
        "border-inline-start-color",
        "border-inline-end-width",
        "border-inline-end-style",
        "border-inline-end-color",
        "border-radius",
        "outline",
        "outline-width",
        "outline-style",
        "outline-color",
        "outline-offset",
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
        "background",
        "background-color",
        "color",
        "opacity",
        "visibility",
        "overflow",
        "overflow-x",
        "overflow-y",
        "font-size",
        "font-family",
        "font-weight",
        "font-style",
        "line-height",
        "text-align",
        "text-decoration",
        "text-decoration-line",
        "text-decoration-style",
        "text-decoration-color",
        "text-decoration-thickness",
        "white-space",
        "flex",
        "flex-grow",
        "flex-shrink",
        "flex-basis",
        "flex-direction",
        "flex-wrap",
        "flex-flow",
        "justify-content",
        "align-items",
        "align-content",
        "justify-items",
        "justify-self",
        "place-items",
        "place-content",
        "place-self",
        "align-self",
        "gap",
        "row-gap",
        "column-gap",
        "grid-template-columns",
        "grid-template-rows",
        "grid-column",
        "grid-row",
        "grid-auto-flow",
        "top",
        "right",
        "bottom",
        "left",
        "inset",
        "inset-block",
        "inset-block-start",
        "inset-block-end",
        "inset-inline",
        "inset-inline-start",
        "inset-inline-end",
        "z-index",
        "vertical-align",
        "text-transform",
        "letter-spacing",
        "word-spacing",
        "text-indent",
        "direction",
        "unicode-bidi",
        "word-break",
        "overflow-wrap",
        "hyphens",
        "tab-size",
        "quotes",
        "text-shadow",
        "font-stretch",
        "font-kerning",
        "font-variant",
        "text-align-last",
        "text-justify",
        "color-scheme",
        "caret-color",
        "scrollbar-color",
        "float",
        "clear",
        "cursor",
        "list-style",
        "list-style-type",
        "list-style-position",
        "list-style-image",
        "object-fit",
        "object-position",
        "writing-mode",
        "text-orientation",
        "text-combine-upright",
        "text-emphasis",
        "text-emphasis-color",
        "text-emphasis-style",
        "text-emphasis-position",
        "fill",
        "fill-rule",
        "fill-opacity",
        "stroke",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-dasharray",
        "stroke-dashoffset",
        "stroke-opacity",
    ];
    if !IMPLEMENTED_PROPERTIES.contains(&name.as_str()) {
        return false;
    }
    match name.as_str() {
        "display" => matches!(
            value.as_str(),
            "none"
                | "block"
                | "inline"
                | "inline-block"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "list-item"
        ),
        "position" => matches!(
            value.as_str(),
            "static" | "relative" | "absolute" | "fixed" | "sticky"
        ),
        "box-sizing" => matches!(value.as_str(), "content-box" | "border-box"),
        "flex-direction" => matches!(
            value.as_str(),
            "row" | "row-reverse" | "column" | "column-reverse"
        ),
        "flex-wrap" => matches!(value.as_str(), "nowrap" | "wrap" | "wrap-reverse"),
        "justify-content" => matches!(
            value.as_str(),
            "normal"
                | "start"
                | "end"
                | "center"
                | "left"
                | "right"
                | "flex-start"
                | "flex-end"
                | "space-between"
                | "space-around"
                | "space-evenly"
        ),
        "justify-items" | "justify-self" => matches!(
            value.as_str(),
            "normal"
                | "stretch"
                | "start"
                | "end"
                | "self-start"
                | "self-end"
                | "center"
                | "left"
                | "right"
        ),
        "place-items" | "place-content" | "place-self" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            (1..=2).contains(&values.len())
                && values.iter().all(|value| {
                    matches!(
                        *value,
                        "normal"
                            | "stretch"
                            | "start"
                            | "end"
                            | "self-start"
                            | "self-end"
                            | "center"
                            | "left"
                            | "right"
                            | "flex-start"
                            | "flex-end"
                            | "baseline"
                            | "space-between"
                            | "space-around"
                            | "space-evenly"
                    )
                })
        }
        "align-items" | "align-content" | "align-self" => matches!(
            value.as_str(),
            "normal"
                | "stretch"
                | "center"
                | "start"
                | "end"
                | "self-start"
                | "self-end"
                | "flex-start"
                | "flex-end"
                | "baseline"
                | "first baseline"
                | "last baseline"
                | "space-between"
                | "space-around"
                | "space-evenly"
        ),
        "gap" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            (1..=2).contains(&values.len())
                && values.iter().all(|value| is_valid_gap_component(value))
        }
        "row-gap" | "column-gap" => is_valid_gap_component(&value),
        "list-style" => expand_list_style(&value).is_some(),
        "background" => expand_background(&value).is_some(),
        "place-items" | "place-content" | "place-self" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            if (1..=2).contains(&values.len()) {
                let (first, second) = match name.as_str() {
                    "place-items" => ("align-items", "justify-items"),
                    "place-self" => ("align-self", "justify-self"),
                    _ => ("align-content", "justify-content"),
                };
                apply(name.clone(), value.clone());
                apply(first.to_owned(), values[0].to_owned());
                apply(
                    second.to_owned(),
                    values.get(1).copied().unwrap_or(values[0]).to_owned(),
                );
                return;
            }
        }
        "overflow" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            (1..=2).contains(&values.len())
                && values.iter().all(|value| {
                    matches!(*value, "visible" | "hidden" | "clip" | "scroll" | "auto")
                })
        }
        "overflow-x" | "overflow-y" => {
            matches!(
                value.as_str(),
                "visible" | "hidden" | "clip" | "scroll" | "auto"
            )
        }
        "flex-flow" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            (1..=2).contains(&values.len())
                && values.iter().all(|value| {
                    matches!(
                        *value,
                        "row"
                            | "row-reverse"
                            | "column"
                            | "column-reverse"
                            | "nowrap"
                            | "wrap"
                            | "wrap-reverse"
                    )
                })
                && values
                    .iter()
                    .filter(|value| {
                        matches!(**value, "row" | "row-reverse" | "column" | "column-reverse")
                    })
                    .count()
                    <= 1
                && values
                    .iter()
                    .filter(|value| matches!(**value, "nowrap" | "wrap" | "wrap-reverse"))
                    .count()
                    <= 1
        }
        "visibility" => matches!(value.as_str(), "visible" | "hidden" | "collapse"),
        "aspect-ratio" => {
            value == "auto" || parse_supported_aspect_ratio(&value)
        }
        _ => true,
    }
}

fn parse_supported_aspect_ratio(value: &str) -> bool {
    let value = value.strip_prefix("auto").map(str::trim).unwrap_or(value);
    if value.is_empty() {
        return false;
    }
    let (width, height) = value.split_once('/').unwrap_or((value, "1"));
    let (Ok(width), Ok(height)) = (width.trim().parse::<f64>(), height.trim().parse::<f64>())
    else {
        return false;
    };
    width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0
}

/// Evaluate the bounded, commonly used subset of CSS media queries.
fn media_query_matches(query: &str, viewport_width: u32, viewport_height: u32) -> bool {
    query.split(',').any(|alternative| {
        let mut query = alternative.trim().to_ascii_lowercase();
        if query.is_empty() {
            return false;
        }
        let negate = query.starts_with("not ");
        if negate {
            query = query[4..].trim_start().to_owned();
        }
        if let Some(rest) = query.strip_prefix("only ") {
            query = rest.trim_start().to_owned();
        }

        let (media_type, conditions) = if let Some((kind, rest)) = query.split_once(" and ") {
            (kind.trim(), rest)
        } else {
            (query.as_str(), "")
        };
        let mut matches = match media_type {
            "" | "all" | "screen" => true,
            "print" => false,
            _ if media_type.starts_with('(') => true,
            _ => false,
        };
        if !matches {
            return negate;
        }

        let mut remaining = if media_type.starts_with('(') {
            query.as_str()
        } else {
            conditions
        };
        while let Some(open) = remaining.find('(') {
            let after_open = &remaining[open + 1..];
            let Some(close) = after_open.find(')') else {
                return false;
            };
            let feature = after_open[..close].trim();
            matches &= media_feature_matches(feature, viewport_width, viewport_height);
            remaining = &after_open[close + 1..];
        }
        if conditions.trim().is_empty() && !media_type.starts_with('(') {
            matches = true;
        }
        if negate {
            !matches
        } else {
            matches
        }
    })
}

fn media_feature_matches(feature: &str, viewport_width: u32, viewport_height: u32) -> bool {
    if let Some((name, value)) = feature.split_once(':') {
        let name = name.trim();
        let value = value.trim();
        let expected = parse_media_length(value);
        return match name {
            "min-width" => expected.is_some_and(|v| f64::from(viewport_width) >= v),
            "max-width" => expected.is_some_and(|v| f64::from(viewport_width) <= v),
            "min-height" => expected.is_some_and(|v| f64::from(viewport_height) >= v),
            "max-height" => expected.is_some_and(|v| f64::from(viewport_height) <= v),
            "orientation" if value == "portrait" => viewport_height >= viewport_width,
            "orientation" if value == "landscape" => viewport_width > viewport_height,
            "aspect-ratio" => parse_media_ratio(value).is_some_and(|(w, h)| {
                u64::from(viewport_width) * u64::from(h)
                    == u64::from(viewport_height) * u64::from(w)
            }),
            "min-aspect-ratio" => parse_media_ratio(value).is_some_and(|(w, h)| {
                u64::from(viewport_width) * u64::from(h)
                    >= u64::from(viewport_height) * u64::from(w)
            }),
            "max-aspect-ratio" => parse_media_ratio(value).is_some_and(|(w, h)| {
                u64::from(viewport_width) * u64::from(h)
                    <= u64::from(viewport_height) * u64::from(w)
            }),
            _ => false,
        };
    }

    let terms = feature.split_whitespace().collect::<Vec<_>>();
    let dimension = |name: &str| match name {
        "width" => Some(f64::from(viewport_width)),
        "height" => Some(f64::from(viewport_height)),
        "aspect-ratio" if viewport_height > 0 => {
            Some(f64::from(viewport_width) / f64::from(viewport_height))
        }
        _ => None,
    };
    let parse_value = |name: &str, value: &str| {
        if name == "aspect-ratio" {
            parse_media_ratio(value).map(|(width, height)| f64::from(width) / f64::from(height))
        } else {
            parse_media_length(value)
        }
    };
    let compare = |left: f64, operator: &str, right: f64| match operator {
        "<" => left < right,
        "<=" => left <= right,
        "=" => left == right,
        ">=" => left >= right,
        ">" => left > right,
        _ => false,
    };
    let invert = |operator: &str| match operator {
        "<" => ">",
        "<=" => ">=",
        "=" => "=",
        ">=" => "<=",
        ">" => "<",
        _ => "",
    };

    match terms.as_slice() {
        [name, operator, value] if dimension(name).is_some() => {
            let Some(expected) = parse_value(name, value) else {
                return false;
            };
            compare(dimension(name).unwrap(), operator, expected)
        }
        [value, operator, name] if dimension(name).is_some() => {
            let Some(expected) = parse_value(name, value) else {
                return false;
            };
            compare(dimension(name).unwrap(), invert(operator), expected)
        }
        [lower, lower_operator, name, upper_operator, upper] if dimension(name).is_some() => {
            let (Some(lower), Some(upper)) = (parse_value(name, lower), parse_value(name, upper))
            else {
                return false;
            };
            let current = dimension(name).unwrap();
            compare(lower, lower_operator, current) && compare(current, upper_operator, upper)
        }
        _ => false,
    }
}

fn parse_media_ratio(value: &str) -> Option<(u32, u32)> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.trim().parse::<u32>().ok()?;
    let denominator = denominator.trim().parse::<u32>().ok()?;
    (numerator > 0 && denominator > 0).then_some((numerator, denominator))
}

fn parse_media_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let units = [
        ("rem", 16.0_f64),
        ("em", 16.0),
        ("px", 1.0),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("q", 96.0 / 101.6),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
    ];
    for (unit, multiplier) in units {
        let Some(number) = value.strip_suffix(unit) else {
            continue;
        };
        let number = number.trim().parse::<f64>().ok()?;
        let pixels = number * multiplier;
        return (pixels.is_finite() && pixels >= 0.0).then_some(pixels);
    }
    None
}

/// Scan only top-level qualified rules. Unlike splitting on braces, this respects
/// comments, quoted strings, escaped characters and nested delimiters.
fn parse_rule_blocks(input: &str, max_blocks: usize) -> Vec<(String, String)> {
    if max_blocks == 0 {
        return Vec::new();
    }
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
            } else if ch == active_quote || matches!(ch, '\n' | '\r' | '\u{000C}') {
                // CSS strings cannot contain an unescaped newline. End the
                // quoted region so a malformed value cannot hide later rules.
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
                        if blocks.try_reserve(1).is_err() {
                            return blocks;
                        }
                        blocks.push((selector, std::mem::take(&mut body)));
                        if blocks.len() >= max_blocks {
                            return blocks;
                        }
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
                    if selectors.try_reserve(1).is_err() {
                        return selectors;
                    }
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
        if selectors.try_reserve(1).is_err() {
            return selectors;
        }
        selectors.push(selector.to_owned());
    }
    selectors
}

fn is_valid_gap_component(value: &str) -> bool {
    if value == "normal" || value == "0" {
        return true;
    }

    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let units = [
        "px", "in", "cm", "mm", "q", "pt", "pc", "em", "rem", "ex", "ch", "vw", "vh", "vmin",
        "vmax", "%",
    ];
    let Some(unit) = units.iter().find(|unit| lower.ends_with(**unit)) else {
        return false;
    };
    let number = &value[..value.len() - unit.len()];
    number
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite() && number >= 0.0)
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Property) {
    let value = declaration.value.trim().to_owned();
    let important = declaration.important;
    let name = if declaration.name.starts_with("--") {
        declaration.name.clone()
    } else {
        declaration.name.to_ascii_lowercase()
    };

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
        "inset" => {
            if let Some(values) = expand_box_shorthand(&name, &value) {
                apply(name.clone(), value.clone());
                for (property, property_value) in [
                    ("top", values[0].clone()),
                    ("right", values[1].clone()),
                    ("bottom", values[2].clone()),
                    ("left", values[3].clone()),
                ] {
                    apply(property.to_owned(), property_value);
                }
                return;
            }
        }
        "margin-block" | "margin-inline" | "padding-block" | "padding-inline" | "inset-block"
        | "inset-inline" => {
            if let Some([start, end]) = expand_pair_shorthand(&value) {
                let (first, second) = match name.as_str() {
                    "margin-block" => ("margin-block-start", "margin-block-end"),
                    "margin-inline" => ("margin-inline-start", "margin-inline-end"),
                    "padding-block" => ("padding-block-start", "padding-block-end"),
                    "padding-inline" => ("padding-inline-start", "padding-inline-end"),
                    "inset-block" => ("inset-block-start", "inset-block-end"),
                    _ => ("inset-inline-start", "inset-inline-end"),
                };
                apply(name.clone(), value.clone());
                apply(first.to_owned(), start);
                apply(second.to_owned(), end);
                return;
            }
        }
        "border-width" | "border-style" | "border-color" => {
            if let Some(values) = expand_box_shorthand(&name, &value) {
                apply(name.clone(), value.clone());
                for (property, property_value) in [
                    (format!("{name}-top"), values[0].clone()),
                    (format!("{name}-right"), values[1].clone()),
                    (format!("{name}-bottom"), values[2].clone()),
                    (format!("{name}-left"), values[3].clone()),
                ] {
                    apply(property, property_value);
                }
                return;
            }
        }
        "border-radius" => {
            if let Some(values) = expand_border_radius(&value) {
                apply(name.clone(), value.clone());
                for (property, property_value) in [
                    ("border-top-left-radius", values[0].clone()),
                    ("border-top-right-radius", values[1].clone()),
                    ("border-bottom-right-radius", values[2].clone()),
                    ("border-bottom-left-radius", values[3].clone()),
                ] {
                    apply(property.to_owned(), property_value);
                }
                return;
            }
        }
        "outline" => {
            if let Some((width, style, color)) = expand_border_shorthand(&value) {
                apply(name.clone(), value.clone());
                apply("outline-width".to_owned(), width);
                apply("outline-style".to_owned(), style);
                apply("outline-color".to_owned(), color);
                return;
            }
        }
        "text-decoration" => {
            if let Some((line, style, color, thickness)) = expand_text_decoration(&value) {
                apply(name.clone(), value.clone());
                apply("text-decoration-line".to_owned(), line);
                apply("text-decoration-style".to_owned(), style);
                apply("text-decoration-color".to_owned(), color);
                apply("text-decoration-thickness".to_owned(), thickness);
                return;
            }
        }
        "list-style" => {
            if let Some((list_type, position, image)) = expand_list_style(&value) {
                apply(name.clone(), value.clone());
                apply("list-style-type".to_owned(), list_type);
                apply("list-style-position".to_owned(), position);
                apply("list-style-image".to_owned(), image);
                return;
            }
        }
        "background" => {
            if let Some((color, image, repeat, position, size)) = expand_background(&value) {
                apply(name.clone(), value.clone());
                apply("background-color".to_owned(), color);
                apply("background-image".to_owned(), image);
                apply("background-repeat".to_owned(), repeat);
                apply("background-position".to_owned(), position);
                apply("background-size".to_owned(), size);
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
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            if let Some((width, style, color)) = expand_border_shorthand(&value) {
                apply(format!("{name}-width"), width);
                apply(format!("{name}-style"), style);
                apply(format!("{name}-color"), color);
                return;
            }
        }
        "border-block" | "border-inline" => {
            if let Some((width, style, color)) = expand_border_shorthand(&value) {
                apply(name.clone(), value.clone());
                let (start, end) = if name == "border-block" {
                    ("border-block-start", "border-block-end")
                } else {
                    ("border-inline-start", "border-inline-end")
                };
                for side in [start, end] {
                    apply(format!("{side}-width"), width.clone());
                    apply(format!("{side}-style"), style.clone());
                    apply(format!("{side}-color"), color.clone());
                }
                return;
            }
        }
        "border-block-start" | "border-block-end" | "border-inline-start" | "border-inline-end" => {
            if let Some((width, style, color)) = expand_border_shorthand(&value) {
                apply(format!("{name}-width"), width);
                apply(format!("{name}-style"), style);
                apply(format!("{name}-color"), color);
                return;
            }
        }
        "overflow" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            if (1..=2).contains(&values.len())
                && values.iter().all(|value| {
                    matches!(
                        value.to_ascii_lowercase().as_str(),
                        "visible" | "hidden" | "clip" | "scroll" | "auto"
                    )
                })
            {
                apply(name.clone(), value.clone());
                apply("overflow-x".to_owned(), values[0].to_owned());
                apply(
                    "overflow-y".to_owned(),
                    values.get(1).copied().unwrap_or(values[0]).to_owned(),
                );
                return;
            }
        }
        "flex-flow" => {
            let values = value.split_whitespace().collect::<Vec<_>>();
            if (1..=2).contains(&values.len()) {
                let mut direction = None;
                let mut wrapping = None;
                let mut valid = true;
                for token in values {
                    let token = token.to_ascii_lowercase();
                    if matches!(
                        token.as_str(),
                        "row" | "row-reverse" | "column" | "column-reverse"
                    ) {
                        if direction.replace(token).is_some() {
                            valid = false;
                        }
                    } else if matches!(token.as_str(), "nowrap" | "wrap" | "wrap-reverse") {
                        if wrapping.replace(token).is_some() {
                            valid = false;
                        }
                    } else {
                        valid = false;
                    }
                }
                if valid {
                    apply(name.clone(), value.clone());
                    apply(
                        "flex-direction".to_owned(),
                        direction.unwrap_or_else(|| "row".to_owned()),
                    );
                    apply(
                        "flex-wrap".to_owned(),
                        wrapping.unwrap_or_else(|| "nowrap".to_owned()),
                    );
                    return;
                }
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
    let Some((start, open, end, comma)) = find_var_function(value) else {
        return value.to_owned();
    };
    let inside = &value[open + 1..end];
    let (name, fallback) = comma.map_or((inside, None), |comma| {
        (&inside[..comma - open - 1], Some(&inside[comma - open..]))
    });
    let name = name.trim();
    let replacement = variables
        .get(name)
        .map(str::to_owned)
        .or_else(|| fallback.map(str::trim).map(str::to_owned));
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

/// Locate the next var() function without mistaking quoted text for a
/// function, and split its fallback only at a top-level comma.
fn find_var_function(value: &str) -> Option<(usize, usize, usize, Option<usize>)> {
    let bytes = value.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut start = None;
    let mut open = 0usize;

    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if index + 4 <= bytes.len()
            && bytes[index..index + 3].eq_ignore_ascii_case(b"var")
            && bytes[index + 3] == b'('
            && (index == 0
                || !(bytes[index - 1].is_ascii_alphanumeric()
                    || matches!(bytes[index - 1], b'_' | b'-')))
        {
            start = Some(index);
            open = index + 3;
            break;
        }
        index += 1;
    }

    let start = start?;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut comma = None;
    for (index, &byte) in bytes.iter().enumerate().skip(open + 1) {
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == delimiter {
                quote = None;
            }
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            continue;
        }
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => return Some((start, open, index, comma)),
            b')' => depth -= 1,
            b',' if depth == 0 && comma.is_none() => comma = Some(index),
            _ => {}
        }
    }
    None
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
        "border-style"
        | "border-top-style"
        | "border-right-style"
        | "border-bottom-style"
        | "border-left-style" => "none",
        "border-color"
        | "border-top-color"
        | "border-right-color"
        | "border-bottom-color"
        | "border-left-color" => "currentcolor",
        "background-image" => "none",
        "background-repeat" => "repeat",
        "background-position" => "0% 0%",
        "background-size" => "auto",
        "box-sizing" => "content-box",
        "position" => "static",
        "overflow" | "overflow-x" | "overflow-y" => "visible",
        "opacity" => "1",
        "z-index" => "auto",
        "vertical-align" => "baseline",
        "font-size" => "16px",
        "font-family" => "serif",
        "font-style" => "normal",
        "font-weight" => "normal",
        "line-height" => "normal",
        "text-align" => "start",
        "text-decoration" | "text-decoration-line" | "text-transform" => "none",
        "text-decoration-style" => "solid",
        "text-decoration-color" => "currentcolor",
        "text-decoration-thickness" => "auto",
        "white-space" => "normal",
        "letter-spacing" | "word-spacing" => "normal",
        "text-indent" => "0",
        "direction" => "ltr",
        "unicode-bidi" => "normal",
        "word-break" | "overflow-wrap" => "normal",
        "hyphens" => "manual",
        "tab-size" => "8",
        "quotes" => "auto",
        "text-shadow" => "none",
        "font-stretch" => "normal",
        "font-kerning" => "auto",
        "font-feature-settings" | "font-variation-settings" => "normal",
        "writing-mode" => "horizontal-tb",
        "text-orientation" => "mixed",
        "text-combine-upright" => "none",
        "text-emphasis" => "none",
        "text-emphasis-color" => "currentcolor",
        "text-emphasis-style" => "none",
        "text-emphasis-position" => "over right",
        "text-align-last" => "auto",
        "text-justify" => "auto",
        "color-scheme" => "normal",
        "caret-color" => "auto",
        "scrollbar-color" => "auto",
        "fill" => "black",
        "fill-rule" => "nonzero",
        "fill-opacity" | "stroke-opacity" => "1",
        "stroke" => "none",
        "stroke-width" => "1",
        "stroke-linecap" => "butt",
        "stroke-linejoin" => "miter",
        "stroke-dasharray" | "stroke-dashoffset" => "none",
        "list-style" => "disc outside none",
        "visibility" => "visible",
        "flex-grow" => "0",
        "flex-shrink" => "1",
        "flex-basis" => "auto",
        "flex-direction" => "row",
        "flex-wrap" => "nowrap",
        "justify-content" => "normal",
        "align-items" | "align-content" => "normal",
        "align-self" => "auto",
        "gap" | "row-gap" | "column-gap" => "normal",
        "grid-template-columns" | "grid-template-rows" => "none",
        "grid-column" | "grid-row" => "auto",
        "grid-auto-flow" => "row",
        "float" | "clear" => "none",
        "cursor" => "auto",
        "list-style-type" => "disc",
        "list-style-position" => "outside",
        "list-style-image" => "none",
        "object-fit" => "fill",
        "object-position" => "50% 50%",
        _ => "initial",
    }
}

fn expand_border_shorthand(value: &str) -> Option<(String, String, String)> {
    let mut width = None;
    let mut style = None;
    let mut color = None;

    for token in value.split_whitespace() {
        if width.is_none() && is_border_width_token(token) {
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

fn expand_background(value: &str) -> Option<(String, String, String, String, String)> {
    if value.contains(',') {
        return None;
    }

    let mut color = None;
    let mut image = None;
    let mut repeat = Vec::new();
    let mut position = Vec::new();
    let mut size = Vec::new();
    let mut after_slash = false;

    for raw_token in value.split_whitespace() {
        let mut token = raw_token;
        if let Some((before, after)) = raw_token.split_once('/') {
            if after_slash || before.is_empty() || after.is_empty() {
                return None;
            }
            token = before;
            after_slash = true;
            let lower_after = after.to_ascii_lowercase();
            if !is_background_size_token(&lower_after) {
                return None;
            }
            size.push(after.to_owned());
        }

        let lower = token.to_ascii_lowercase();
        if lower == "none" || lower.starts_with("url(") || lower.starts_with("image(") {
            if image.replace(token.to_owned()).is_some() {
                return None;
            }
        } else if matches!(
            lower.as_str(),
            "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round"
        ) {
            if after_slash || repeat.len() == 2 {
                return None;
            }
            repeat.push(lower);
        } else if matches!(
            lower.as_str(),
            "left" | "right" | "top" | "bottom" | "center"
        ) || is_background_position_length(&lower)
        {
            if after_slash {
                if !is_background_size_token(&lower) || size.len() == 2 {
                    return None;
                }
                size.push(token.to_owned());
            } else {
                if position.len() == 2 {
                    return None;
                }
                position.push(lower);
            }
        } else if after_slash {
            if size.len() == 2 || !is_background_size_token(&lower) {
                return None;
            }
            size.push(token.to_owned());
        } else if color.replace(token.to_owned()).is_some() {
            return None;
        }
    }

    Some((
        color.unwrap_or_else(|| "transparent".to_owned()),
        image.unwrap_or_else(|| "none".to_owned()),
        if repeat.is_empty() {
            "repeat".to_owned()
        } else {
            repeat.join(" ")
        },
        if position.is_empty() {
            "0% 0%".to_owned()
        } else {
            position.join(" ")
        },
        if size.is_empty() {
            "auto".to_owned()
        } else {
            size.join(" ")
        },
    ))
}

fn is_background_position_length(value: &str) -> bool {
    value == "0" || value.ends_with('%') || is_border_width_token(value)
}

fn is_background_size_token(value: &str) -> bool {
    matches!(value, "auto" | "cover" | "contain") || is_background_position_length(value)
}

fn expand_list_style(value: &str) -> Option<(String, String, String)> {
    let mut list_type = None;
    let mut position = None;
    let mut image = None;

    for token in value.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "disc"
                | "circle"
                | "square"
                | "decimal"
                | "decimal-leading-zero"
                | "lower-roman"
                | "upper-roman"
                | "lower-alpha"
                | "upper-alpha"
                | "lower-latin"
                | "upper-latin"
                | "lower-greek"
                | "armenian"
                | "georgian"
                | "none"
                | "disclosure-open"
                | "disclosure-closed"
        ) {
            if list_type.replace(lower).is_some() {
                return None;
            }
        } else if matches!(lower.as_str(), "inside" | "outside") {
            if position.replace(lower).is_some() {
                return None;
            }
        } else if lower.starts_with("url(") {
            if image.replace(token.to_owned()).is_some() {
                return None;
            }
        } else {
            return None;
        }
    }

    let list_type = list_type.unwrap_or_else(|| "disc".to_owned());
    let image = image.unwrap_or_else(|| "none".to_owned());
    Some((
        list_type,
        position.unwrap_or_else(|| "outside".to_owned()),
        image,
    ))
}

fn expand_text_decoration(value: &str) -> Option<(String, String, String, String)> {
    let mut lines = Vec::new();
    let mut none_line = false;
    let mut style = None;
    let mut color = None;
    let mut thickness = None;

    for token in value.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower == "none" {
            if none_line || !lines.is_empty() {
                return None;
            }
            none_line = true;
        } else if matches!(
            lower.as_str(),
            "underline" | "overline" | "line-through" | "blink"
        ) {
            if none_line || lines.contains(&lower) {
                return None;
            }
            lines.push(lower);
        } else if matches!(
            lower.as_str(),
            "solid" | "double" | "dotted" | "dashed" | "wavy"
        ) {
            if style.replace(lower).is_some() {
                return None;
            }
        } else if matches!(lower.as_str(), "auto" | "from-font")
            || is_border_width_token(token)
            || lower.strip_suffix('%').is_some_and(|number| {
                number
                    .parse::<f64>()
                    .is_ok_and(|value| value.is_finite() && value >= 0.0)
            })
        {
            if thickness.replace(token.to_owned()).is_some() {
                return None;
            }
        } else if color.replace(token.to_owned()).is_some() {
            return None;
        }
    }

    Some((
        if none_line || lines.is_empty() {
            "none".to_owned()
        } else {
            lines.join(" ")
        },
        style.unwrap_or_else(|| "solid".to_owned()),
        color.unwrap_or_else(|| "currentcolor".to_owned()),
        thickness.unwrap_or_else(|| "auto".to_owned()),
    ))
}

fn is_border_width_token(value: &str) -> bool {
    if matches!(value, "thin" | "medium" | "thick" | "0") {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    const UNITS: &[&str] = &[
        "px", "em", "rem", "ex", "ch", "ic", "cap", "lh", "rlh", "vw", "vh", "vmin", "vmax", "vi",
        "vb", "cm", "mm", "q", "in", "pt", "pc",
    ];
    let Some(unit) = UNITS.iter().find(|unit| lower.ends_with(**unit)) else {
        return false;
    };
    let number = &value[..value.len() - unit.len()];
    number
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite() && number >= 0.0)
}

fn parse_px_token(value: &str) -> Option<u32> {
    value.strip_suffix("px")?.trim().parse().ok()
}

fn expand_pair_shorthand(value: &str) -> Option<[String; 2]> {
    let mut values = value.split_whitespace();
    let first = values.next()?;
    let second = values.next().unwrap_or(first);
    if values.next().is_some() {
        return None;
    }
    Some([first.to_owned(), second.to_owned()])
}

fn expand_border_radius(value: &str) -> Option<[String; 4]> {
    let mut halves = value.split('/');
    let horizontal = expand_box_shorthand("border-radius", halves.next()?.trim())?;
    let vertical = match halves.next() {
        Some(value) => expand_box_shorthand("border-radius", value.trim())?,
        None => horizontal.clone(),
    };
    if halves.next().is_some() {
        return None;
    }
    Some(std::array::from_fn(|index| {
        if horizontal[index] == vertical[index] {
            horizontal[index].clone()
        } else {
            format!("{} {}", horizontal[index], vertical[index])
        }
    }))
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
            | "direction"
            | "list-style"
            | "list-style-image"
            | "list-style-position"
            | "list-style-type"
            | "quotes"
            | "tab-size"
            | "word-break"
            | "overflow-wrap"
            | "hyphens"
            | "font-stretch"
            | "font-kerning"
            | "font-feature-settings"
            | "font-variation-settings"
            | "text-shadow"
            | "writing-mode"
            | "text-orientation"
            | "text-combine-upright"
            | "text-emphasis"
            | "text-emphasis-color"
            | "text-emphasis-style"
            | "text-emphasis-position"
            | "text-align-last"
            | "text-justify"
            | "color-scheme"
            | "caret-color"
            | "scrollbar-color"
            | "fill"
            | "fill-rule"
            | "fill-opacity"
            | "stroke"
            | "stroke-width"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "stroke-opacity"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylesheet_parser_stops_collecting_after_rule_limit() {
        let css = (0..(MAX_STYLE_RULES + 100))
            .map(|index| format!(".class-{index} {{ color: red; }}"))
            .collect::<String>();

        let blocks = parse_rule_blocks(&css, MAX_STYLE_RULES);

        assert_eq!(blocks.len(), MAX_STYLE_RULES);
    }

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
    fn inset_shorthand_expands_one_to_four_values() {
        let node = Node::element("div");

        let one = StyleSheet::parse("div { inset: 5px; }").compute_style(&node);
        assert_eq!(one.get("top"), Some("5px"));
        assert_eq!(one.get("right"), Some("5px"));
        assert_eq!(one.get("bottom"), Some("5px"));
        assert_eq!(one.get("left"), Some("5px"));

        let four = StyleSheet::parse("div { inset: 1px 2px 3px 4px; }").compute_style(&node);
        assert_eq!(four.get("top"), Some("1px"));
        assert_eq!(four.get("right"), Some("2px"));
        assert_eq!(four.get("bottom"), Some("3px"));
        assert_eq!(four.get("left"), Some("4px"));
    }

    #[test]
    fn inset_shorthand_respects_important_longhands() {
        let sheet = StyleSheet::parse("div { top: 9px !important; inset: 1px 2px; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("top"), Some("9px"));
        assert_eq!(computed.get("right"), Some("2px"));
        assert_eq!(computed.get("bottom"), Some("1px"));
        assert_eq!(computed.get("left"), Some("2px"));
    }

    #[test]
    fn logical_border_shorthands_expand_start_and_end_components() {
        let sheet = StyleSheet::parse(
            "div { border-block: 2px solid red; border-inline-start: dashed blue; border-inline-end: 3px double; }",
        );
        let computed = sheet.compute_style(&Node::element("div"));
        for (property, expected) in [
            ("border-block-start-width", "2px"),
            ("border-block-start-style", "solid"),
            ("border-block-start-color", "red"),
            ("border-block-end-width", "2px"),
            ("border-block-end-style", "solid"),
            ("border-block-end-color", "red"),
            ("border-inline-start-width", "medium"),
            ("border-inline-start-style", "dashed"),
            ("border-inline-start-color", "blue"),
            ("border-inline-end-width", "3px"),
            ("border-inline-end-style", "double"),
            ("border-inline-end-color", "currentcolor"),
        ] {
            assert_eq!(computed.get(property), Some(expected), "{property}");
        }
    }

    #[test]
    fn logical_border_shorthands_preserve_important_components() {
        let sheet = StyleSheet::parse(
            "div { border-block-start-width: 9px !important; border-block: 2px solid red; }",
        );
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("border-block-start-width"), Some("9px"));
        assert_eq!(computed.get("border-block-start-style"), Some("solid"));
        assert_eq!(computed.get("border-block-end-width"), Some("2px"));
    }

    #[test]
    fn outline_shorthand_expands_width_style_and_color() {
        let sheet = StyleSheet::parse(
            "div { outline: 2px solid red; } span { outline: dashed blue; } p { outline: 3px; }",
        );
        let cases = [
            ("div", "2px", "solid", "red"),
            ("span", "medium", "dashed", "blue"),
            ("p", "3px", "none", "currentcolor"),
        ];
        for (tag, width, style, color) in cases {
            let computed = sheet.compute_style(&Node::element(tag));
            assert_eq!(computed.get("outline-width"), Some(width));
            assert_eq!(computed.get("outline-style"), Some(style));
            assert_eq!(computed.get("outline-color"), Some(color));
        }
    }

    #[test]
    fn outline_shorthand_preserves_important_longhands() {
        let sheet =
            StyleSheet::parse("div { outline-width: 7px !important; outline: 2px solid red; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("outline-width"), Some("7px"));
        assert_eq!(computed.get("outline-style"), Some("solid"));
        assert_eq!(computed.get("outline-color"), Some("red"));
    }

    #[test]
    fn background_shorthand_expands_color_image_repeat_position_and_size() {
        let sheet = StyleSheet::parse(
            "body { background: #102030; } .hero { background: url(hero.png) no-repeat center/cover #fff; } .tile { background: repeat-x url(tile.png) left top; }",
        );
        let cases = [
            ("body", "#102030", "none", "repeat", "0% 0%", "auto"),
            (
                "hero",
                "#fff",
                "url(hero.png)",
                "no-repeat",
                "center",
                "cover",
            ),
            (
                "tile",
                "transparent",
                "url(tile.png)",
                "repeat-x",
                "left top",
                "auto",
            ),
        ];
        for (tag, color, image, repeat, position, size) in cases {
            let computed = sheet.compute_style(&Node::element(tag));
            assert_eq!(computed.get("background-color"), Some(color));
            assert_eq!(computed.get("background-image"), Some(image));
            assert_eq!(computed.get("background-repeat"), Some(repeat));
            assert_eq!(computed.get("background-position"), Some(position));
            assert_eq!(computed.get("background-size"), Some(size));
        }
    }

    #[test]
    fn background_shorthand_respects_important_longhands() {
        let sheet = StyleSheet::parse(
            ".hero { background-image: url(locked.png) !important; background: url(hero.png) no-repeat red; }",
        );
        let mut hero = Node::element("div");
        hero.set_attribute("class", "hero");
        let computed = sheet.compute_style(&hero);
        assert_eq!(computed.get("background-image"), Some("url(locked.png)"));
        assert_eq!(computed.get("background-color"), Some("red"));
        assert_eq!(computed.get("background-repeat"), Some("no-repeat"));
    }

    #[test]
    fn invalid_multi_layer_background_is_not_partially_expanded() {
        let sheet = StyleSheet::parse(".hero { background: url(a.png), url(b.png); }");
        let mut hero = Node::element("div");
        hero.set_attribute("class", "hero");
        let computed = sheet.compute_style(&hero);
        assert_eq!(computed.get("background-image"), None);
        assert_eq!(computed.get("background-repeat"), None);
    }

    #[test]
    fn list_style_shorthand_expands_type_position_and_image() {
        let sheet = StyleSheet::parse(
            "ul { list-style: square inside; } ol { list-style: upper-roman outside url(mark.png); } li { list-style: none; }",
        );
        let cases = [
            ("ul", "square", "inside", "none"),
            ("ol", "upper-roman", "outside", "url(mark.png)"),
            ("li", "none", "outside", "none"),
        ];
        for (tag, list_type, position, image) in cases {
            let computed = sheet.compute_style(&Node::element(tag));
            assert_eq!(computed.get("list-style-type"), Some(list_type));
            assert_eq!(computed.get("list-style-position"), Some(position));
            assert_eq!(computed.get("list-style-image"), Some(image));
        }
    }

    #[test]
    fn list_style_shorthand_preserves_important_longhands() {
        let sheet = StyleSheet::parse(
            "ul { list-style-type: decimal !important; list-style: square inside; }",
        );
        let computed = sheet.compute_style(&Node::element("ul"));
        assert_eq!(computed.get("list-style-type"), Some("decimal"));
        assert_eq!(computed.get("list-style-position"), Some("inside"));
    }

    #[test]
    fn invalid_list_style_shorthand_does_not_expand() {
        let sheet = StyleSheet::parse("ul { list-style: square inside outside; }");
        let computed = sheet.compute_style(&Node::element("ul"));
        assert_eq!(computed.get("list-style-type"), None);
        assert_eq!(computed.get("list-style-position"), None);
    }

    #[test]
    fn text_decoration_shorthand_expands_line_style_color_and_thickness() {
        let sheet = StyleSheet::parse(
            "a { text-decoration: underline wavy red 2px; } p { text-decoration: overline line-through dotted blue; } span { text-decoration: none; }",
        );
        let cases = [
            ("a", "underline", "wavy", "red", "2px"),
            ("p", "overline line-through", "dotted", "blue", "auto"),
            ("span", "none", "solid", "currentcolor", "auto"),
        ];
        for (tag, line, style, color, thickness) in cases {
            let computed = sheet.compute_style(&Node::element(tag));
            assert_eq!(computed.get("text-decoration-line"), Some(line));
            assert_eq!(computed.get("text-decoration-style"), Some(style));
            assert_eq!(computed.get("text-decoration-color"), Some(color));
            assert_eq!(computed.get("text-decoration-thickness"), Some(thickness));
        }
    }

    #[test]
    fn text_decoration_shorthand_preserves_important_longhands() {
        let sheet = StyleSheet::parse(
            "a { text-decoration-color: green !important; text-decoration: underline wavy red 3px; }",
        );
        let computed = sheet.compute_style(&Node::element("a"));
        assert_eq!(computed.get("text-decoration-line"), Some("underline"));
        assert_eq!(computed.get("text-decoration-style"), Some("wavy"));
        assert_eq!(computed.get("text-decoration-color"), Some("green"));
        assert_eq!(computed.get("text-decoration-thickness"), Some("3px"));
    }

    #[test]
    fn invalid_text_decoration_shorthand_does_not_expand() {
        let sheet = StyleSheet::parse("a { text-decoration: underline wavy dotted; }");
        let computed = sheet.compute_style(&Node::element("a"));
        assert_eq!(computed.get("text-decoration-line"), None);
        assert_eq!(computed.get("text-decoration-style"), None);
    }

    #[test]
    fn border_radius_shorthand_expands_one_to_four_corners() {
        let sheet = StyleSheet::parse(
            "div { border-radius: 8px; } span { border-radius: 1px 2px; } p { border-radius: 1px 2px 3px; } a { border-radius: 1px 2px 3px 4px; }",
        );
        let cases = [
            ("div", ["8px", "8px", "8px", "8px"]),
            ("span", ["1px", "2px", "1px", "2px"]),
            ("p", ["1px", "2px", "3px", "2px"]),
            ("a", ["1px", "2px", "3px", "4px"]),
        ];
        for (tag, expected) in cases {
            let computed = sheet.compute_style(&Node::element(tag));
            for (property, value) in [
                ("border-top-left-radius", expected[0]),
                ("border-top-right-radius", expected[1]),
                ("border-bottom-right-radius", expected[2]),
                ("border-bottom-left-radius", expected[3]),
            ] {
                assert_eq!(computed.get(property), Some(value), "{tag} {property}");
            }
        }
    }

    #[test]
    fn border_radius_shorthand_supports_elliptical_slash_values() {
        let sheet = StyleSheet::parse(
            "div { border-radius: 10px / 20px; } span { border-radius: 1px 2px / 3px 4px; }",
        );
        let div = sheet.compute_style(&Node::element("div"));
        assert_eq!(div.get("border-top-left-radius"), Some("10px 20px"));
        assert_eq!(div.get("border-bottom-right-radius"), Some("10px 20px"));

        let span = sheet.compute_style(&Node::element("span"));
        assert_eq!(span.get("border-top-left-radius"), Some("1px 3px"));
        assert_eq!(span.get("border-top-right-radius"), Some("2px 4px"));
        assert_eq!(span.get("border-bottom-right-radius"), Some("1px 3px"));
        assert_eq!(span.get("border-bottom-left-radius"), Some("2px 4px"));
    }

    #[test]
    fn border_radius_shorthand_respects_important_corner_longhands() {
        let sheet = StyleSheet::parse(
            "div { border-top-left-radius: 12px !important; border-radius: 4px 8px; }",
        );
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("border-top-left-radius"), Some("12px"));
        assert_eq!(computed.get("border-top-right-radius"), Some("8px"));
        assert_eq!(computed.get("border-bottom-right-radius"), Some("4px"));
        assert_eq!(computed.get("border-bottom-left-radius"), Some("8px"));
    }

    #[test]
    fn supports_aspect_ratio_declarations_and_rejects_invalid_ratios() {
        assert!(supports_declaration("aspect-ratio", "16 / 9"));
        assert!(supports_declaration("aspect-ratio", "auto"));
        assert!(supports_declaration("aspect-ratio", "auto 4 / 3"));
        assert!(!supports_declaration("aspect-ratio", "0 / 1"));
        assert!(!supports_declaration("aspect-ratio", "1 / 0"));
        assert!(!supports_declaration("aspect-ratio", "NaN"));
    }

    #[test]
    fn supports_recognizes_border_radius_corner_longhands() {
        assert!(supports_declaration("border-top-left-radius", "8px"));
        assert!(supports_declaration("border-bottom-right-radius", "1rem"));
    }

    #[test]
    fn border_side_shorthands_expand_components_independently() {
        let sheet = StyleSheet::parse(
            "div { border-top: 2px solid red; border-right: dashed blue; border-bottom: 4px double; border-left: green; }",
        );
        let computed = sheet.compute_style(&Node::element("div"));
        for (property, expected) in [
            ("border-top-width", "2px"),
            ("border-top-style", "solid"),
            ("border-top-color", "red"),
            ("border-right-width", "medium"),
            ("border-right-style", "dashed"),
            ("border-right-color", "blue"),
            ("border-bottom-width", "4px"),
            ("border-bottom-style", "double"),
            ("border-bottom-color", "currentcolor"),
            ("border-left-width", "medium"),
            ("border-left-style", "none"),
            ("border-left-color", "green"),
        ] {
            assert_eq!(computed.get(property), Some(expected), "{property}");
        }
    }

    #[test]
    fn border_side_shorthand_accepts_fractional_and_relative_widths() {
        let sheet = StyleSheet::parse(
            "div { border-top: 0.5px solid red; border-right: 0.125rem dashed blue; }",
        );
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("border-top-width"), Some("0.5px"));
        assert_eq!(computed.get("border-top-style"), Some("solid"));
        assert_eq!(computed.get("border-right-width"), Some("0.125rem"));
        assert_eq!(computed.get("border-right-style"), Some("dashed"));
    }

    #[test]
    fn border_side_shorthands_respect_important_longhands() {
        let sheet = StyleSheet::parse(
            "div { border-top-width: 9px !important; border-top: 2px solid red; }",
        );
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("border-top-width"), Some("9px"));
        assert_eq!(computed.get("border-top-style"), Some("solid"));
        assert_eq!(computed.get("border-top-color"), Some("red"));
    }

    #[test]
    fn border_component_shorthands_expand_one_to_four_sides() {
        let node = Node::element("div");
        let sheet = StyleSheet::parse(
            "div { border-width: 1px 2px 3px 4px; border-style: solid dashed; border-color: red green blue; }",
        );
        let computed = sheet.compute_style(&node);
        for (property, expected) in [
            ("border-top-width", "1px"),
            ("border-right-width", "2px"),
            ("border-bottom-width", "3px"),
            ("border-left-width", "4px"),
            ("border-top-style", "solid"),
            ("border-right-style", "dashed"),
            ("border-bottom-style", "solid"),
            ("border-left-style", "dashed"),
            ("border-top-color", "red"),
            ("border-right-color", "green"),
            ("border-bottom-color", "blue"),
            ("border-left-color", "green"),
        ] {
            assert_eq!(computed.get(property), Some(expected), "{property}");
        }
    }

    #[test]
    fn border_component_shorthands_respect_important_longhands() {
        let sheet =
            StyleSheet::parse("div { border-top-width: 9px !important; border-width: 1px 2px; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("border-top-width"), Some("9px"));
        assert_eq!(computed.get("border-right-width"), Some("2px"));
        assert_eq!(computed.get("border-bottom-width"), Some("1px"));
    }

    #[test]
    fn logical_axis_shorthands_expand_one_and_two_values() {
        let node = Node::element("div");
        let sheet = StyleSheet::parse(
            "div { margin-block: 1px 2px; margin-inline: 3px; padding-block: 4px 5px; padding-inline: 6px 7px; inset-block: 8px 9px; inset-inline: 10px; }",
        );
        let computed = sheet.compute_style(&node);
        for (property, expected) in [
            ("margin-block-start", "1px"),
            ("margin-block-end", "2px"),
            ("margin-inline-start", "3px"),
            ("margin-inline-end", "3px"),
            ("padding-block-start", "4px"),
            ("padding-block-end", "5px"),
            ("padding-inline-start", "6px"),
            ("padding-inline-end", "7px"),
            ("inset-block-start", "8px"),
            ("inset-block-end", "9px"),
            ("inset-inline-start", "10px"),
            ("inset-inline-end", "10px"),
        ] {
            assert_eq!(computed.get(property), Some(expected), "{property}");
        }
    }

    #[test]
    fn logical_axis_shorthands_preserve_important_longhands() {
        let sheet =
            StyleSheet::parse("div { margin-block-start: 9px !important; margin-block: 1px 2px; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("margin-block-start"), Some("9px"));
        assert_eq!(computed.get("margin-block-end"), Some("2px"));
    }

    #[test]
    fn logical_axis_shorthands_reject_more_than_two_values() {
        let sheet = StyleSheet::parse("div { padding-inline: 1px 2px 3px; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("padding-inline-start"), None);
        assert_eq!(computed.get("padding-inline-end"), None);
    }

    #[test]
    fn gap_shorthand_expands_one_and_two_values() {
        let node = Node::element("div");

        let one = StyleSheet::parse("div { gap: 12px; }").compute_style(&node);
        assert_eq!(one.get("row-gap"), Some("12px"));
        assert_eq!(one.get("column-gap"), Some("12px"));

        let two = StyleSheet::parse("div { gap: 8px 16px; }").compute_style(&node);
        assert_eq!(two.get("row-gap"), Some("8px"));
        assert_eq!(two.get("column-gap"), Some("16px"));
    }

    #[test]
    fn invalid_gap_shorthand_does_not_expand() {
        let sheet = StyleSheet::parse("div { gap: nonsense%; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("row-gap"), None);
        assert_eq!(computed.get("column-gap"), None);
    }

    #[test]
    fn gap_shorthand_respects_important_longhands() {
        let sheet = StyleSheet::parse("div { row-gap: 3px !important; gap: 8px 16px; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("row-gap"), Some("3px"));
        assert_eq!(computed.get("column-gap"), Some("16px"));
    }

    #[test]
    fn overflow_shorthand_expands_one_and_two_values() {
        let one = StyleSheet::parse("div { overflow: hidden; }");
        let node = Node::element("div");
        let computed = one.compute_style(&node);
        assert_eq!(computed.get("overflow"), Some("hidden"));
        assert_eq!(computed.get("overflow-x"), Some("hidden"));
        assert_eq!(computed.get("overflow-y"), Some("hidden"));

        let two = StyleSheet::parse("div { overflow: clip auto; }");
        let computed = two.compute_style(&node);
        assert_eq!(computed.get("overflow-x"), Some("clip"));
        assert_eq!(computed.get("overflow-y"), Some("auto"));
    }

    #[test]
    fn flex_flow_shorthand_expands_direction_and_wrapping() {
        let node = Node::element("div");
        let sheet = StyleSheet::parse("div { flex-flow: column-reverse wrap-reverse; }");
        let computed = sheet.compute_style(&node);
        assert_eq!(computed.get("flex-direction"), Some("column-reverse"));
        assert_eq!(computed.get("flex-wrap"), Some("wrap-reverse"));

        let direction_only = StyleSheet::parse("div { flex-flow: row-reverse; }");
        let computed = direction_only.compute_style(&node);
        assert_eq!(computed.get("flex-direction"), Some("row-reverse"));
        assert_eq!(computed.get("flex-wrap"), Some("nowrap"));

        let wrap_only = StyleSheet::parse("div { flex-flow: wrap; }");
        let computed = wrap_only.compute_style(&node);
        assert_eq!(computed.get("flex-direction"), Some("row"));
        assert_eq!(computed.get("flex-wrap"), Some("wrap"));
    }

    #[test]
    fn invalid_flex_flow_shorthand_does_not_expand() {
        let sheet = StyleSheet::parse("div { flex-flow: row wrap column; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("flex-direction"), None);
        assert_eq!(computed.get("flex-wrap"), None);
    }

    #[test]
    fn invalid_overflow_shorthand_does_not_expand() {
        let sheet = StyleSheet::parse("div { overflow: hidden nonsense; }");
        let computed = sheet.compute_style(&Node::element("div"));
        assert_eq!(computed.get("overflow-x"), None);
        assert_eq!(computed.get("overflow-y"), None);
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
    fn css_wide_initial_values_cover_common_paint_and_layout_properties() {
        for (property, expected) in [
            ("border-top-style", "none"),
            ("border-right-color", "currentcolor"),
            ("background-image", "none"),
            ("background-repeat", "repeat"),
            ("box-sizing", "content-box"),
            ("position", "static"),
            ("overflow-x", "visible"),
            ("opacity", "1"),
            ("z-index", "auto"),
            ("vertical-align", "baseline"),
            ("font-family", "serif"),
            ("text-transform", "none"),
            ("white-space", "normal"),
            ("flex-grow", "0"),
            ("flex-shrink", "1"),
            ("flex-basis", "auto"),
            ("flex-direction", "row"),
            ("flex-wrap", "nowrap"),
            ("align-items", "normal"),
            ("grid-template-columns", "none"),
            ("float", "none"),
            ("cursor", "auto"),
            ("unicode-bidi", "normal"),
            ("word-break", "normal"),
            ("overflow-wrap", "normal"),
            ("hyphens", "manual"),
            ("tab-size", "8"),
            ("quotes", "auto"),
            ("text-shadow", "none"),
            ("font-stretch", "normal"),
            ("font-kerning", "auto"),
            ("font-feature-settings", "normal"),
            ("font-variation-settings", "normal"),
            ("writing-mode", "horizontal-tb"),
            ("text-orientation", "mixed"),
            ("text-emphasis", "none"),
            ("text-emphasis-position", "over right"),
            ("text-align-last", "auto"),
            ("color-scheme", "normal"),
            ("caret-color", "auto"),
            ("scrollbar-color", "auto"),
            ("fill", "black"),
            ("fill-rule", "nonzero"),
            ("fill-opacity", "1"),
            ("stroke", "none"),
            ("stroke-width", "1"),
            ("stroke-linecap", "butt"),
            ("stroke-linejoin", "miter"),
            ("stroke-opacity", "1"),
            ("list-style", "disc outside none"),
            ("list-style-type", "disc"),
            ("object-fit", "fill"),
        ] {
            assert_eq!(initial_value(property), expected, "{property}");
        }
    }

    #[test]
    fn expands_place_self_shorthand() {
        let sheet = StyleSheet::parse("div { place-self: end center; }");
        let style = sheet.compute_style(&Node::element("div"));

        assert_eq!(style.get("align-self"), Some("end"));
        assert_eq!(style.get("justify-self"), Some("center"));
    }

    #[test]
    fn expands_place_items_and_place_content_shorthands() {
        let sheet =
            StyleSheet::parse("div { place-items: center end; place-content: end space-between; }");
        let style = sheet.compute_style(&Node::element("div"));

        assert_eq!(style.get("align-items"), Some("center"));
        assert_eq!(style.get("justify-items"), Some("end"));
        assert_eq!(style.get("align-content"), Some("end"));
        assert_eq!(style.get("justify-content"), Some("space-between"));

        let single = StyleSheet::parse("div { place-items: center; }");
        let style = single.compute_style(&Node::element("div"));
        assert_eq!(style.get("align-items"), Some("center"));
        assert_eq!(style.get("justify-items"), Some("center"));
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
    fn expands_individual_border_side_shorthands() {
        let sheet = StyleSheet::parse(
            "div { border-top: 2px solid red; border-right: dashed blue; border-bottom: 4px double; border-left: 1px dotted green; }",
        );
        let node = Node::element("div");
        let style = sheet.compute_style(&node);

        assert_eq!(style.get("border-top-width"), Some("2px"));
        assert_eq!(style.get("border-top-style"), Some("solid"));
        assert_eq!(style.get("border-top-color"), Some("red"));
        assert_eq!(style.get("border-right-width"), Some("medium"));
        assert_eq!(style.get("border-right-style"), Some("dashed"));
        assert_eq!(style.get("border-right-color"), Some("blue"));
        assert_eq!(style.get("border-bottom-width"), Some("4px"));
        assert_eq!(style.get("border-bottom-style"), Some("double"));
        assert_eq!(style.get("border-bottom-color"), Some("currentcolor"));
        assert_eq!(style.get("border-left-width"), Some("1px"));
        assert_eq!(style.get("border-left-style"), Some("dotted"));
        assert_eq!(style.get("border-left-color"), Some("green"));
    }

    #[test]
    fn var_fallback_splits_only_at_top_level_commas() {
        let variables = ComputedStyle::default();
        assert_eq!(
            resolve_variables("var(--missing, rgb(1, 2, 3))", &variables, 0),
            "rgb(1, 2, 3)"
        );
        assert_eq!(
            resolve_variables("var(--missing, var(--also-missing, blue))", &variables, 0),
            "blue"
        );
    }

    #[test]
    fn var_resolution_ignores_quoted_function_text_and_handles_quotes_in_fallbacks() {
        let variables = ComputedStyle::default();
        assert_eq!(
            resolve_variables("content: \"var(--missing, red)\"", &variables, 0),
            "content: \"var(--missing, red)\""
        );
        assert_eq!(
            resolve_variables("var(--missing, \"a,b)\")", &variables, 0),
            "\"a,b)\""
        );
    }

    #[test]
    fn var_function_names_are_ascii_case_insensitive() {
        let mut variables = ComputedStyle::default();
        variables.set("--accent", "teal");
        assert_eq!(resolve_variables("VAR(--accent)", &variables, 0), "teal");
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
    fn custom_property_names_are_case_sensitive_during_var_resolution() {
        let sheet = StyleSheet::parse(
            ".box { --Theme: red; --theme: blue; color: var(--Theme); background-color: var(--theme); }",
        );
        let mut node = Node::element("div");
        node.set_attribute("class", "box");

        let style = sheet.compute_style(&node);
        assert_eq!(style.get("--Theme"), Some("red"));
        assert_eq!(style.get("--theme"), Some("blue"));
        assert_eq!(style.get("color"), Some("red"));
        assert_eq!(style.get("background-color"), Some("blue"));
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
    fn unescaped_newline_in_string_does_not_hide_following_stylesheet_rules() {
        let sheet =
            StyleSheet::parse(".broken { content: \"bad\n; color: red; } .valid { color: blue; }");

        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(
            sheet.rules[1]
                .declarations
                .iter()
                .find(|declaration| declaration.name == "color")
                .map(|declaration| declaration.value.as_str()),
            Some("blue")
        );
    }

    #[test]
    fn inherited_properties_reach_descendants() {
        let sheet = StyleSheet::parse(
            "body { color: green; direction: rtl; list-style-type: square; word-break: break-word; writing-mode: vertical-rl; text-orientation: upright; text-emphasis: filled; caret-color: red; fill: blue; stroke-width: 3; margin: 10px; }",
        );
        let body = Node::element("body");
        let child = Node::element("span");
        let path = [&body, &child];

        let style = sheet.compute_style_path(&path);
        assert_eq!(style.get("color"), Some("green"));
        assert_eq!(style.get("direction"), Some("rtl"));
        assert_eq!(style.get("list-style-type"), Some("square"));
        assert_eq!(style.get("word-break"), Some("break-word"));
        assert_eq!(style.get("writing-mode"), Some("vertical-rl"));
        assert_eq!(style.get("text-orientation"), Some("upright"));
        assert_eq!(style.get("text-emphasis"), Some("filled"));
        assert_eq!(style.get("caret-color"), Some("red"));
        assert_eq!(style.get("fill"), Some("blue"));
        assert_eq!(style.get("stroke-width"), Some("3"));
        assert_eq!(style.get("margin"), None);
    }

    #[test]
    fn media_queries_apply_only_when_viewport_matches() {
        let css = "@media screen and (max-width: 600px) { .card { color: red; } } @media (min-width: 601px) { .card { color: blue; } }";
        let mut card = Node::element("div");
        card.set_attribute("class", "card");

        let narrow = StyleSheet::parse_for_viewport(css, 400, 800);
        let wide = StyleSheet::parse_for_viewport(css, 900, 700);
        assert_eq!(narrow.compute_style(&card).get("color"), Some("red"));
        assert_eq!(wide.compute_style(&card).get("color"), Some("blue"));
    }

    #[test]
    fn media_query_range_syntax_supports_width_and_height() {
        let css = "@media (width >= 600px) { .wide { color: green; } } @media (400px < width <= 900px) { .middle { color: blue; } } @media (height < 700px) { .short { color: purple; } }";
        let sheet = StyleSheet::parse_for_viewport(css, 800, 600);

        for (class, expected) in [
            ("wide", Some("green")),
            ("middle", Some("blue")),
            ("short", Some("purple")),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get("color"), expected);
        }

        let narrow = StyleSheet::parse_for_viewport(css, 350, 900);
        let mut wide = Node::element("div");
        wide.set_attribute("class", "wide");
        let mut middle = Node::element("div");
        middle.set_attribute("class", "middle");
        assert_eq!(narrow.compute_style(&wide).get("color"), None);
        assert_eq!(narrow.compute_style(&middle).get("color"), None);
    }

    #[test]
    fn media_query_lists_and_orientation_are_supported() {
        let css = "@media (max-width: 400px), screen and (orientation: landscape) { p { color: green; } }";
        let paragraph = Node::element("p");
        let portrait = StyleSheet::parse_for_viewport(css, 360, 800);
        let landscape = StyleSheet::parse_for_viewport(css, 900, 600);
        assert_eq!(
            portrait.compute_style(&paragraph).get("color"),
            Some("green")
        );
        assert_eq!(
            landscape.compute_style(&paragraph).get("color"),
            Some("green")
        );
    }

    #[test]
    fn unsupported_media_features_do_not_leak_rules() {
        let sheet = StyleSheet::parse_for_viewport(
            "@media (hover: hover) { p { color: red; } }",
            1200,
            800,
        );
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn nested_media_rules_are_depth_bounded() {
        let nested = "@media all { ".repeat(12) + "p { color: red; }" + &"} ".repeat(12);
        let sheet = StyleSheet::parse_for_viewport(&nested, 800, 600);
        assert!(sheet.rules.is_empty());
    }
    #[test]
    fn supports_selector_conditions_use_the_selector_parser() {
        let css = "@supports selector(.card > .title) { .supported { color: green; } } @supports selector(:has(> img)) { .has { color: blue; } } @supports selector(.card >) { .invalid { color: red; } }";
        let sheet = StyleSheet::parse(css);

        for (class, expected) in [
            ("supported", Some("green")),
            ("has", Some("blue")),
            ("invalid", None),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get("color"), expected);
        }
    }

    #[test]
    fn supports_rules_recognize_border_side_components() {
        let css = "@supports (border-top-width: 2px) { .width { border-top-width: 2px; } } @supports (border-left-style: dashed) { .style { border-left-style: dashed; } } @supports (border-right-color: red) { .color { border-right-color: red; } }";
        let sheet = StyleSheet::parse(css);
        for (class, property, expected) in [
            ("width", "border-top-width", "2px"),
            ("style", "border-left-style", "dashed"),
            ("color", "border-right-color", "red"),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get(property), Some(expected));
        }
    }

    #[test]
    fn supports_rules_recognize_additional_stored_text_and_layout_properties() {
        let css = "@supports (text-transform: uppercase) { .transform { text-transform: uppercase; } } @supports (letter-spacing: 0.1em) { .spacing { letter-spacing: 0.1em; } } @supports (float: left) { .float { float: left; } } @supports (cursor: pointer) { .cursor { cursor: pointer; } } @supports (object-fit: cover) { .object { object-fit: cover; } }";
        let sheet = StyleSheet::parse(css);
        for (class, property, expected) in [
            ("transform", "text-transform", "uppercase"),
            ("spacing", "letter-spacing", "0.1em"),
            ("float", "float", "left"),
            ("cursor", "cursor", "pointer"),
            ("object", "object-fit", "cover"),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get(property), Some(expected));
        }
    }

    #[test]
    fn supports_rules_apply_only_for_implemented_declarations() {
        let css = "@supports (display: grid) { .grid { display: grid; } } @supports (display: subgrid) { .grid { color: red; } }";
        let mut node = Node::element("div");
        node.set_attribute("class", "grid");
        let sheet = StyleSheet::parse(css);

        assert_eq!(sheet.compute_style(&node).get("display"), Some("grid"));
        assert_eq!(sheet.compute_style(&node).get("color"), None);
    }

    #[test]
    fn supports_rules_evaluate_nested_and_or_and_not_conditions() {
        let css = "@supports (display: flex) and (gap: 8px) { .a { color: green; } } @supports (display: flex) and (display: subgrid) { .b { color: red; } } @supports not (display: subgrid) { .c { color: blue; } } @supports (display: grid) or (display: subgrid) { .d { color: purple; } }";
        let sheet = StyleSheet::parse(css);
        for (class, expected) in [
            ("a", Some("green")),
            ("b", None),
            ("c", Some("blue")),
            ("d", Some("purple")),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get("color"), expected);
        }
    }

    #[test]
    fn supports_rules_reject_unknown_properties_and_malformed_conditions() {
        let css = "@supports (unknown-feature: yes) { p { color: red; } } @supports display: grid { p { color: blue; } }";
        let sheet = StyleSheet::parse(css);
        assert!(sheet.rules.is_empty());
    }
    #[test]
    fn media_queries_support_aspect_ratio_features() {
        let css = "@media (min-aspect-ratio: 16/9) { .wide { color: green; } } @media (max-aspect-ratio: 4/3) { .narrow { color: blue; } } @media (aspect-ratio: 16/9) { .exact { color: purple; } }";
        let sheet = StyleSheet::parse_for_viewport(css, 1920, 1080);
        for (class, expected) in [
            ("wide", Some("green")),
            ("narrow", None),
            ("exact", Some("purple")),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get("color"), expected);
        }

        let portrait = StyleSheet::parse_for_viewport(css, 800, 1200);
        let mut wide = Node::element("div");
        wide.set_attribute("class", "wide");
        let mut narrow = Node::element("div");
        narrow.set_attribute("class", "narrow");
        assert_eq!(portrait.compute_style(&wide).get("color"), None);
        assert_eq!(portrait.compute_style(&narrow).get("color"), Some("blue"));
    }

    #[test]
    fn media_query_lengths_support_absolute_css_units() {
        assert!(media_query_matches("(min-width: 1in)", 96, 100));
        assert!(media_query_matches("(min-width: 2.54cm)", 96, 100));
        assert!(media_query_matches("(min-width: 72pt)", 96, 100));
        assert!(media_query_matches("(max-width: 6pc)", 96, 100));
        assert!(!media_query_matches("(min-width: 1cm)", 30, 100));
    }

    #[test]
    fn media_query_lengths_preserve_fractional_css_pixels() {
        let css = "@media (min-width: 600.5px) { .min { color: green; } } @media (width > 799.5px) { .range { color: blue; } }";
        let below_minimum = StyleSheet::parse_for_viewport(css, 600, 800);
        let above_minimum = StyleSheet::parse_for_viewport(css, 601, 800);
        let above_range = StyleSheet::parse_for_viewport(css, 800, 800);
        let mut min = Node::element("div");
        min.set_attribute("class", "min");
        let mut range = Node::element("div");
        range.set_attribute("class", "range");

        assert_eq!(below_minimum.compute_style(&min).get("color"), None);
        assert_eq!(
            above_minimum.compute_style(&min).get("color"),
            Some("green")
        );
        assert_eq!(above_range.compute_style(&range).get("color"), Some("blue"));
    }

    #[test]
    fn media_query_range_syntax_supports_aspect_ratio() {
        let css = "@media (aspect-ratio >= 16/9) { .wide { color: green; } } @media (4/3 < aspect-ratio < 2/1) { .middle { color: blue; } } @media (aspect-ratio < 1/1) { .portrait { color: purple; } }";
        let landscape = StyleSheet::parse_for_viewport(css, 1600, 900);
        let portrait = StyleSheet::parse_for_viewport(css, 800, 1200);

        for (sheet, class, expected) in [
            (&landscape, "wide", Some("green")),
            (&landscape, "middle", Some("blue")),
            (&landscape, "portrait", None),
            (&portrait, "wide", None),
            (&portrait, "middle", None),
            (&portrait, "portrait", Some("purple")),
        ] {
            let mut node = Node::element("div");
            node.set_attribute("class", class);
            assert_eq!(sheet.compute_style(&node).get("color"), expected);
        }
    }

    #[test]
    fn media_queries_reject_invalid_aspect_ratio_values() {
        let sheet = StyleSheet::parse_for_viewport(
            "@media (min-aspect-ratio: 16/0) { p { color: red; } } @media (aspect-ratio: nope) { p { color: blue; } }",
            1200,
            800,
        );
        assert!(sheet.rules.is_empty());
    }
}
