use crate::html::{Node, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Specificity {
    pub ids: u32,
    pub classes: u32,
    pub types: u32,
}

impl Specificity {
    pub const fn new(ids: u32, classes: u32, types: u32) -> Self {
        Self {
            ids,
            classes,
            types,
        }
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.ids, self.classes, self.types).cmp(&(other.ids, other.classes, other.types))
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::ops::AddAssign for Specificity {
    fn add_assign(&mut self, other: Self) {
        self.ids += other.ids;
        self.classes += other.classes;
        self.types += other.types;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleSelector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: Vec<AttributeSelector>,
    pub pseudo_classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSelector {
    pub name: String,
    pub operator: AttributeOperator,
    pub value: Option<String>,
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeOperator {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub parts: Vec<(Option<Combinator>, SimpleSelector)>,
}

impl Selector {
    pub fn parse(input: &str) -> Option<Self> {
        let chars: Vec<char> = input.trim().chars().collect();
        if chars.is_empty() {
            return None;
        }

        let mut parts = Vec::new();
        let mut i = 0;
        let mut pending = None;
        let mut had_space = false;

        while i < chars.len() {
            while i < chars.len() && chars[i].is_whitespace() {
                had_space = true;
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            if had_space && !parts.is_empty() && pending.is_none() {
                pending = Some(Combinator::Descendant);
            }

            if matches!(chars[i], '>' | '+' | '~') {
                pending = Some(match chars[i] {
                    '>' => Combinator::Child,
                    '+' => Combinator::AdjacentSibling,
                    '~' => Combinator::GeneralSibling,
                    _ => unreachable!(),
                });
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
            }

            let start = i;
            let mut bracket = 0;
            let mut paren = 0;
            let mut quote = None;
            while i < chars.len() {
                let c = chars[i];
                if let Some(q) = quote {
                    if c == q {
                        quote = None;
                    }
                    i += 1;
                    continue;
                }
                match c {
                    '\'' | '"' if bracket > 0 => quote = Some(c),
                    '[' => bracket += 1,
                    ']' if bracket > 0 => bracket -= 1,
                    '(' if bracket == 0 => paren += 1,
                    ')' if bracket == 0 && paren > 0 => paren -= 1,
                    c if bracket == 0
                        && paren == 0
                        && (c.is_whitespace() || matches!(c, '>' | '+' | '~')) =>
                    {
                        break
                    }
                    _ => {}
                }
                i += 1;
            }

            let simple = parse_simple(&chars[start..i])?;
            parts.push((pending.take(), simple));
            had_space = false;
        }

        (!parts.is_empty()).then_some(Self { parts })
    }

    pub fn specificity(&self) -> Specificity {
        let mut result = Specificity::default();
        for (_, part) in &self.parts {
            if part.id.is_some() {
                result.ids += 1;
            }
            result.classes += part.classes.len() as u32 + part.attributes.len() as u32;
            for pseudo in &part.pseudo_classes {
                if pseudo_argument(pseudo, "where").is_some() {
                    continue;
                }
                if let Some(argument) = pseudo_argument(pseudo, "not") {
                    result += pseudo_argument_specificity(argument);
                } else if let Some(argument) = pseudo_argument(pseudo, "is") {
                    result += pseudo_argument_specificity(argument);
                } else {
                    result.classes += 1;
                }
            }
            if part.tag.is_some() {
                result.types += 1;
            }
        }
        result
    }

    pub fn matches(&self, node: &Node) -> bool {
        self.parts.len() == 1 && matches_simple(&self.parts[0].1, node)
    }

    pub fn matches_path(&self, path: &[&Node]) -> bool {
        if self.parts.is_empty() || path.is_empty() || path.len() < self.parts.len() {
            return false;
        }
        self.matches_at(self.parts.len() - 1, path.len() - 1, path)
    }

    pub fn matches_path_with_siblings(
        &self,
        path: &[&Node],
        sibling_lists: &[&[Node]],
        sibling_positions: &[usize],
    ) -> bool {
        if self.parts.is_empty()
            || path.is_empty()
            || path.len() < self.parts.len()
            || sibling_lists.len() != path.len()
            || sibling_positions.len() != path.len()
        {
            return false;
        }
        self.matches_at_with_siblings(
            self.parts.len() - 1,
            path.len() - 1,
            path,
            sibling_lists,
            sibling_positions,
        )
    }

    fn matches_at_with_siblings(
        &self,
        selector_index: usize,
        node_index: usize,
        path: &[&Node],
        sibling_lists: &[&[Node]],
        sibling_positions: &[usize],
    ) -> bool {
        let Some((combinator, simple)) = self.parts.get(selector_index) else {
            return false;
        };
        if !matches_simple_with_siblings(
            simple,
            path[node_index],
            Some(sibling_lists[node_index]),
            Some(sibling_positions[node_index]),
        ) {
            return false;
        }
        if selector_index == 0 {
            return true;
        }

        match combinator.as_ref().unwrap_or(&Combinator::Descendant) {
            Combinator::Child => {
                node_index > 0
                    && self.matches_at_with_siblings(
                        selector_index - 1,
                        node_index - 1,
                        path,
                        sibling_lists,
                        sibling_positions,
                    )
            }
            Combinator::Descendant => (0..node_index).rev().any(|ancestor| {
                self.matches_at_with_siblings(
                    selector_index - 1,
                    ancestor,
                    path,
                    sibling_lists,
                    sibling_positions,
                )
            }),
            Combinator::AdjacentSibling => {
                previous_element(sibling_lists[node_index], sibling_positions[node_index])
                    .is_some_and(|(sibling, position)| {
                        matches_simple_with_siblings(
                            &self.parts[selector_index - 1].1,
                            sibling,
                            Some(sibling_lists[node_index]),
                            Some(position),
                        )
                    })
            }
            Combinator::GeneralSibling => {
                let position = sibling_positions[node_index];
                sibling_lists[node_index]
                    .get(..position)
                    .is_some_and(|siblings| {
                        siblings.iter().enumerate().rev().any(|(index, sibling)| {
                            matches_simple_with_siblings(
                                &self.parts[selector_index - 1].1,
                                sibling,
                                Some(sibling_lists[node_index]),
                                Some(index),
                            )
                        })
                    })
            }
        }
    }

    fn matches_at(&self, selector_index: usize, node_index: usize, path: &[&Node]) -> bool {
        let Some((combinator, simple)) = self.parts.get(selector_index) else {
            return false;
        };
        if !matches_simple(simple, path[node_index]) {
            return false;
        }
        if selector_index == 0 {
            return true;
        }

        match combinator.as_ref().unwrap_or(&Combinator::Descendant) {
            Combinator::Child => {
                node_index > 0 && self.matches_at(selector_index - 1, node_index - 1, path)
            }
            Combinator::Descendant => (0..node_index)
                .rev()
                .any(|ancestor| self.matches_at(selector_index - 1, ancestor, path)),
            Combinator::AdjacentSibling | Combinator::GeneralSibling => false,
        }
    }
}

fn parse_simple(chars: &[char]) -> Option<SimpleSelector> {
    let mut simple = SimpleSelector {
        tag: None,
        id: None,
        classes: Vec::new(),
        attributes: Vec::new(),
        pseudo_classes: Vec::new(),
    };
    let mut i = 0;

    if i < chars.len() && (chars[i].is_ascii_alphabetic() || chars[i] == '_' || chars[i] == '*') {
        let start = i;
        while i < chars.len() && is_name_char(chars[i]) {
            i += 1;
        }
        if chars[start] != '*' {
            simple.tag = Some(
                chars[start..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase(),
            );
        }
    }

    while i < chars.len() {
        match chars[i] {
            ':' => {
                i += 1;
                let start = i;
                while i < chars.len() && is_name_char(chars[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                if i < chars.len() && chars[i] == '(' {
                    let argument_start = i + 1;
                    let mut depth = 1;
                    i += 1;
                    while i < chars.len() && depth > 0 {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                        i += 1;
                    }
                    if depth != 0 || i <= argument_start {
                        return None;
                    }
                    let argument = chars[argument_start..i - 1]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string();
                    if argument.is_empty() {
                        return None;
                    }
                    simple.pseudo_classes.push(format!("{name}({argument})"));
                } else {
                    simple.pseudo_classes.push(name);
                }
            }
            '#' | '.' => {
                let kind = chars[i];
                i += 1;
                let start = i;
                while i < chars.len() && is_name_char(chars[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let value: String = chars[start..i].iter().collect();
                if kind == '#' {
                    simple.id = Some(value);
                } else {
                    simple.classes.push(value);
                }
            }
            '[' => {
                i += 1;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }

                let start = i;
                while i < chars.len() && is_name_char(chars[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name: String = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();

                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }

                let mut operator = AttributeOperator::Exists;
                let mut value = None;
                let mut parsed_case_insensitive = false;
                if i < chars.len() && chars[i] != ']' {
                    operator = match chars[i] {
                        '=' => {
                            i += 1;
                            AttributeOperator::Equals
                        }
                        '~' | '|' | '^' | '$' | '*' => {
                            let op = chars[i];
                            if chars.get(i + 1) != Some(&'=') {
                                return None;
                            }
                            i += 2;
                            match op {
                                '~' => AttributeOperator::Includes,
                                '|' => AttributeOperator::DashMatch,
                                '^' => AttributeOperator::Prefix,
                                '$' => AttributeOperator::Suffix,
                                '*' => AttributeOperator::Substring,
                                _ => unreachable!(),
                            }
                        }
                        _ => return None,
                    };

                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }

                    let quote = chars.get(i).copied().filter(|c| *c == '\'' || *c == '"');
                    if quote.is_some() {
                        i += 1;
                    }

                    let start_value = i;
                    while i < chars.len() && chars[i] != ']' && quote.is_none_or(|q| chars[i] != q)
                    {
                        i += 1;
                    }
                    if start_value == i {
                        return None;
                    }
                    let raw_value: String = chars[start_value..i].iter().collect();
                    if quote.is_none() {
                        let trimmed = raw_value.trim();
                        if let Some(value_text) = trimmed
                            .strip_suffix(" i")
                            .or_else(|| trimmed.strip_suffix(" I"))
                        {
                            value = Some(value_text.trim_end().to_string());
                            parsed_case_insensitive = true;
                        } else if let Some(value_text) = trimmed
                            .strip_suffix(" s")
                            .or_else(|| trimmed.strip_suffix(" S"))
                        {
                            value = Some(value_text.trim_end().to_string());
                        } else {
                            value = Some(trimmed.to_string());
                        }
                    } else {
                        value = Some(raw_value);
                    }

                    if quote.is_some() {
                        if chars.get(i) != quote.as_ref() {
                            return None;
                        }
                        i += 1;
                    }
                }

                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let mut case_insensitive = parsed_case_insensitive;
                if i < chars.len() && chars[i] != ']' {
                    match chars[i].to_ascii_lowercase() {
                        'i' => case_insensitive = true,
                        's' => {}
                        _ => return None,
                    }
                    i += 1;
                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }
                }
                if i >= chars.len() || chars[i] != ']' {
                    return None;
                }
                i += 1;
                simple.attributes.push(AttributeSelector {
                    name,
                    operator,
                    value,
                    case_insensitive,
                });
            }
            _ => return None,
        }
    }

    Some(simple)
}
fn matches_pseudo_class(
    pseudo: &str,
    node: &Node,
    attributes: &std::collections::BTreeMap<String, String>,
    siblings: Option<&[Node]>,
    position: Option<usize>,
) -> bool {
    match pseudo {
        "checked" => attributes.contains_key("checked"),
        "disabled" => attributes.contains_key("disabled"),
        "enabled" => !attributes.contains_key("disabled"),
        "required" => attributes.contains_key("required"),
        "optional" => !attributes.contains_key("required"),
        "read-only" => attributes.contains_key("readonly"),
        "read-write" => !attributes.contains_key("readonly"),
        "empty" => node.children.iter().all(|child| match &child.kind {
            NodeKind::Text(text) => text.is_empty(),
            NodeKind::Comment(_) => true,
            NodeKind::Element { .. } | NodeKind::Document => false,
        }),
        "first-child" => is_element_position(siblings, position, 0),
        "last-child" => siblings.zip(position).is_some_and(|(list, pos)| {
            list.get(pos).is_some_and(Node::is_element)
                && element_position(list, pos) + 1 == element_count(list)
        }),
        "only-child" => element_count(siblings.unwrap_or(&[])) == 1,
        "first-of-type" => is_type_position(node, siblings, position, 0),
        "last-of-type" => is_last_type_position(node, siblings, position),
        "only-of-type" => type_count(node, siblings.unwrap_or(&[])) == 1,
        _ if pseudo.starts_with("nth-child(") => nth_matches(pseudo, siblings, position, false),
        _ if pseudo.starts_with("nth-last-child(") => nth_matches(pseudo, siblings, position, true),
        _ if pseudo.starts_with("nth-of-type(") => {
            nth_type_matches(pseudo, node, siblings, position, false)
        }
        _ if pseudo.starts_with("nth-last-of-type(") => {
            nth_type_matches(pseudo, node, siblings, position, true)
        }
        _ if pseudo.starts_with("not(") => {
            let Some(argument) = pseudo_argument(pseudo, "not") else {
                return false;
            };
            !pseudo_argument_matches(argument, node, siblings, position)
        }
        _ if pseudo.starts_with("is(") || pseudo.starts_with("where(") => {
            let name = if pseudo.starts_with("is(") {
                "is"
            } else {
                "where"
            };
            let Some(argument) = pseudo_argument(pseudo, name) else {
                return false;
            };
            pseudo_argument_matches(argument, node, siblings, position)
        }
        _ => false,
    }
}

fn pseudo_argument<'a>(pseudo: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}(");
    pseudo.strip_prefix(&prefix)?.strip_suffix(')')
}

fn split_pseudo_arguments(argument: &str) -> impl Iterator<Item = &str> {
    argument
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn pseudo_argument_matches(
    argument: &str,
    node: &Node,
    siblings: Option<&[Node]>,
    position: Option<usize>,
) -> bool {
    split_pseudo_arguments(argument).any(|candidate| {
        let Some(selector) = Selector::parse(candidate) else {
            return false;
        };
        if selector.parts.len() != 1 {
            return false;
        }
        matches_simple_with_siblings(&selector.parts[0].1, node, siblings, position)
    })
}

fn pseudo_argument_specificity(argument: &str) -> Specificity {
    split_pseudo_arguments(argument)
        .filter_map(Selector::parse)
        .filter(|selector| selector.parts.len() == 1)
        .map(|selector| selector.specificity())
        .max()
        .unwrap_or_default()
}

fn element_count(siblings: &[Node]) -> usize {
    siblings.iter().filter(|node| node.is_element()).count()
}

fn element_position(siblings: &[Node], position: usize) -> usize {
    siblings
        .iter()
        .take(position + 1)
        .filter(|node| node.is_element())
        .count()
        .saturating_sub(1)
}

fn same_element_type(left: &Node, right: &Node) -> bool {
    left.tag_name()
        .zip(right.tag_name())
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn type_count(node: &Node, siblings: &[Node]) -> usize {
    siblings
        .iter()
        .filter(|sibling| sibling.is_element() && same_element_type(node, sibling))
        .count()
}

fn type_position(node: &Node, siblings: &[Node], position: usize) -> usize {
    siblings
        .iter()
        .take(position + 1)
        .filter(|sibling| sibling.is_element() && same_element_type(node, sibling))
        .count()
        .saturating_sub(1)
}

fn is_type_position(
    node: &Node,
    siblings: Option<&[Node]>,
    position: Option<usize>,
    expected: usize,
) -> bool {
    siblings.zip(position).is_some_and(|(list, pos)| {
        list.get(pos)
            .is_some_and(|candidate| candidate.is_element() && same_element_type(node, candidate))
            && type_position(node, list, pos) == expected
    })
}

fn is_last_type_position(node: &Node, siblings: Option<&[Node]>, position: Option<usize>) -> bool {
    siblings.zip(position).is_some_and(|(list, pos)| {
        list.get(pos)
            .is_some_and(|candidate| candidate.is_element() && same_element_type(node, candidate))
            && type_position(node, list, pos) + 1 == type_count(node, list)
    })
}

fn nth_type_matches(
    pseudo: &str,
    node: &Node,
    siblings: Option<&[Node]>,
    position: Option<usize>,
    from_end: bool,
) -> bool {
    let Some((list, pos)) = siblings.zip(position) else {
        return false;
    };
    let Some(candidate) = list.get(pos) else {
        return false;
    };
    if !candidate.is_element() || !same_element_type(node, candidate) {
        return false;
    }
    let count = type_count(node, list) as i32;
    let index = if from_end {
        count - type_position(node, list, pos) as i32
    } else {
        type_position(node, list, pos) as i32 + 1
    };
    let Some(argument) = pseudo
        .split_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
    else {
        return false;
    };
    parse_nth_formula(argument.trim(), index)
}

fn is_element_position(
    siblings: Option<&[Node]>,
    position: Option<usize>,
    expected: usize,
) -> bool {
    siblings.zip(position).is_some_and(|(list, pos)| {
        list.get(pos).is_some_and(Node::is_element) && element_position(list, pos) == expected
    })
}

fn nth_matches(
    pseudo: &str,
    siblings: Option<&[Node]>,
    position: Option<usize>,
    from_end: bool,
) -> bool {
    let Some((list, pos)) = siblings.zip(position) else {
        return false;
    };
    if list.get(pos).is_none_or(|node| !node.is_element()) {
        return false;
    }
    let count = element_count(list) as i32;
    let index = if from_end {
        count - element_position(list, pos) as i32
    } else {
        element_position(list, pos) as i32 + 1
    };
    let Some(argument) = pseudo
        .split_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
    else {
        return false;
    };
    parse_nth_formula(argument.trim(), index)
}

fn parse_nth_formula(formula: &str, index: i32) -> bool {
    let compact = formula.replace(' ', "").to_ascii_lowercase();
    match compact.as_str() {
        "odd" => return index % 2 == 1,
        "even" => return index % 2 == 0,
        _ => {}
    }
    if let Ok(value) = compact.parse::<i32>() {
        return index == value;
    }
    let Some(n_pos) = compact.find('n') else {
        return false;
    };
    let (a_text, b_text) = compact.split_at(n_pos);
    let a = match a_text {
        "" | "+" => 1,
        "-" => -1,
        _ => match a_text.parse::<i32>() {
            Ok(value) => value,
            Err(_) => return false,
        },
    };
    let b = if b_text.len() <= 1 {
        0
    } else {
        match b_text[1..].parse::<i32>() {
            Ok(value) => value,
            Err(_) => return false,
        }
    };
    let delta = index - b;
    delta >= 0 && a != 0 && delta % a == 0
}

fn previous_element(siblings: &[Node], position: usize) -> Option<(&Node, usize)> {
    siblings
        .get(..position)?
        .iter()
        .enumerate()
        .rev()
        .find(|(_, node)| node.is_element())
        .map(|(index, node)| (node, index))
}

fn is_name_char(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_alphanumeric() || !c.is_ascii()
}

fn attribute_value_matches(
    actual: &str,
    expected: &str,
    case_insensitive: bool,
    matches: impl FnOnce(&str, &str) -> bool,
) -> bool {
    if case_insensitive {
        matches(&actual.to_ascii_lowercase(), &expected.to_ascii_lowercase())
    } else {
        matches(actual, expected)
    }
}

fn matches_simple(simple: &SimpleSelector, node: &Node) -> bool {
    matches_simple_with_siblings(simple, node, None, None)
}

fn matches_simple_with_siblings(
    simple: &SimpleSelector,
    node: &Node,
    siblings: Option<&[Node]>,
    position: Option<usize>,
) -> bool {
    let NodeKind::Element { name, attributes } = &node.kind else {
        return false;
    };

    if let Some(tag) = &simple.tag {
        if !name.eq_ignore_ascii_case(tag) {
            return false;
        }
    }
    if let Some(id) = &simple.id {
        if attributes.get("id") != Some(id) {
            return false;
        }
    }

    let classes = attributes
        .get("class")
        .map(|value| value.split_whitespace().collect::<Vec<_>>())
        .unwrap_or_default();
    if simple
        .classes
        .iter()
        .any(|class| !classes.contains(&class.as_str()))
    {
        return false;
    }

    if simple
        .pseudo_classes
        .iter()
        .any(|pseudo| !matches_pseudo_class(pseudo, node, attributes, siblings, position))
    {
        return false;
    }

    simple.attributes.iter().all(|attribute| {
        let Some(actual) = attributes.get(&attribute.name) else {
            return false;
        };
        let Some(expected) = attribute.value.as_deref() else {
            return true;
        };

        match attribute.operator {
            AttributeOperator::Exists => true,
            AttributeOperator::Equals => {
                attribute_value_matches(actual, expected, attribute.case_insensitive, |a, b| a == b)
            }
            AttributeOperator::Includes => actual.split_whitespace().any(|part| {
                attribute_value_matches(part, expected, attribute.case_insensitive, |a, b| a == b)
            }),
            AttributeOperator::DashMatch => {
                let normalized_actual = if attribute.case_insensitive {
                    actual.to_ascii_lowercase()
                } else {
                    actual.to_string()
                };
                let normalized_expected = if attribute.case_insensitive {
                    expected.to_ascii_lowercase()
                } else {
                    expected.to_string()
                };
                normalized_actual == normalized_expected
                    || normalized_actual.starts_with(&format!("{normalized_expected}-"))
            }
            AttributeOperator::Prefix => {
                attribute_value_matches(actual, expected, attribute.case_insensitive, |a, b| {
                    a.starts_with(b)
                })
            }
            AttributeOperator::Suffix => {
                attribute_value_matches(actual, expected, attribute.case_insensitive, |a, b| {
                    a.ends_with(b)
                })
            }
            AttributeOperator::Substring => {
                attribute_value_matches(actual, expected, attribute.case_insensitive, |a, b| {
                    a.contains(b)
                })
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Node {
        let mut node = Node::element("div");
        if let NodeKind::Element { attributes, .. } = &mut node.kind {
            attributes.insert("id".into(), "main".into());
            attributes.insert("class".into(), "card active".into());
            attributes.insert("role".into(), "main".into());
            attributes.insert("lang".into(), "en-US".into());
        }
        node
    }

    #[test]
    fn manually_constructed_empty_selector_does_not_panic() {
        let selector = Selector { parts: Vec::new() };
        let node = Node::element("div");

        assert!(!selector.matches_path(&[&node]));
        assert!(!selector.matches_path_with_siblings(&[&node], &[&[] as &[Node]], &[0]));
    }

    #[test]
    fn matches_form_state_pseudo_classes() {
        let mut checked = Node::element("input");
        checked.set_attribute("checked", "");
        assert!(Selector::parse("input:checked").unwrap().matches(&checked));
        assert!(!Selector::parse("input:disabled").unwrap().matches(&checked));

        checked.set_attribute("disabled", "");
        assert!(Selector::parse("input:disabled").unwrap().matches(&checked));
        assert!(Selector::parse("input:checked:disabled")
            .unwrap()
            .matches(&checked));
    }

    #[test]
    fn parses_and_matches_simple_selector() {
        let selector = Selector::parse("div#main.card[role=main]").unwrap();
        assert!(selector.matches(&node()));
        assert_eq!(selector.specificity(), Specificity::new(1, 2, 1));
    }

    #[test]
    fn matches_descendant_and_child_paths() {
        let root = Node::element("section");
        let mut child = Node::element("div");
        if let NodeKind::Element { attributes, .. } = &mut child.kind {
            attributes.insert("class".into(), "card".into());
        }
        let leaf = Node::element("span");
        let path = [&root, &child, &leaf];

        assert!(Selector::parse("section .card span")
            .unwrap()
            .matches_path(&path));
        assert!(Selector::parse("section > div")
            .unwrap()
            .matches_path(&path[..2]));
        assert!(!Selector::parse("section > span")
            .unwrap()
            .matches_path(&path));
    }

    #[test]
    fn matches_adjacent_and_general_sibling_selectors() {
        let parent = Node::element("div");
        let first = Node::element("span");
        let mut second = Node::element("p");
        second.set_attribute("class", "target");
        let third = Node::element("p");
        let siblings_vec = vec![first, second, third];
        let siblings: [&[Node]; 3] = [
            &[] as &[Node],
            siblings_vec.as_slice(),
            siblings_vec.as_slice(),
        ];
        let positions = [0, 1];

        assert!(Selector::parse("span + p")
            .unwrap()
            .matches_path_with_siblings(&[&parent, &siblings_vec[1]], &siblings[..2], &positions));
        assert!(Selector::parse("span ~ p")
            .unwrap()
            .matches_path_with_siblings(&[&parent, &siblings_vec[1]], &siblings[..2], &positions));
        assert!(Selector::parse("div > span + p.target")
            .unwrap()
            .matches_path_with_siblings(
                &[&parent, &siblings_vec[0], &siblings_vec[1]],
                &siblings,
                &[0, 0, 1],
            ));
    }

    #[test]
    fn parses_attribute_operators() {
        let cases = [
            ("[role]", true),
            ("[role=main]", true),
            ("[class~=active]", true),
            ("[lang|=en]", true),
            ("[lang^=en]", true),
            ("[lang$=US]", true),
            ("[lang*=n-U]", true),
            ("[lang=EN-US i]", true),
        ];

        for (source, expected) in cases {
            assert_eq!(
                Selector::parse(source).unwrap().matches(&node()),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn matches_odd_and_even_nth_formulas() {
        let mut body = Node::element("body");
        for _ in 0..4 {
            body.append(Node::element("p"));
        }

        let siblings = body.children.as_slice();
        let lists: [&[Node]; 2] = [&[], siblings];

        assert!(Selector::parse("p:nth-child(odd)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[0]], &lists, &[0, 0]));
        assert!(Selector::parse("p:nth-child(odd)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[2]], &lists, &[0, 2]));
        assert!(Selector::parse("p:nth-child(even)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[1]], &lists, &[0, 1]));
        assert!(Selector::parse("p:nth-child(even)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[3]], &lists, &[0, 3]));
        assert!(!Selector::parse("p:nth-child(even)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[0]], &lists, &[0, 0]));
    }

    #[test]
    fn matches_structural_pseudo_classes() {
        let mut body = Node::element("body");
        let first = Node::element("p");
        let mut second = Node::element("p");
        second.append(Node::text("content"));
        let third = Node::element("p");
        body.append(Node::text("whitespace"));
        body.append(first);
        body.append(Node::text("between"));
        body.append(second);
        body.append(third);

        let siblings = body.children.as_slice();
        let lists: [&[Node]; 2] = [&[], siblings];

        assert!(Selector::parse("p:first-child")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[1]], &lists, &[0, 1]));
        assert!(Selector::parse("p:nth-child(2)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[3]], &lists, &[0, 3]));
        assert!(Selector::parse("p:nth-child(2n+1)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[4]], &lists, &[0, 4]));
        assert!(Selector::parse("p:nth-last-child(1)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[4]], &lists, &[0, 4]));
        assert!(Selector::parse("p:last-child")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[4]], &lists, &[0, 4]));
        assert!(!Selector::parse("p:only-child")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[1]], &lists, &[0, 1]));
        assert!(Selector::parse("p:empty")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[1]], &lists, &[0, 1]));
        assert!(!Selector::parse("p:empty")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[3]], &lists, &[0, 3]));
    }

    #[test]
    fn functional_pseudo_arguments_preserve_case() {
        let mut div = Node::element("div");
        div.set_attribute("class", "Card");

        assert!(Selector::parse("div:not(.missing)").unwrap().matches(&div));
        assert!(Selector::parse("div:is(.Card, .missing)")
            .unwrap()
            .matches(&div));
        assert!(!Selector::parse("div:not(.Card)").unwrap().matches(&div));
    }

    #[test]
    fn attribute_selector_case_sensitivity_flags_work() {
        let mut element = Node::element("div");
        element.set_attribute("data-mode", "Dark");

        assert!(!Selector::parse("[data-mode=dark]")
            .unwrap()
            .matches(&element));
        assert!(Selector::parse("[data-mode=dark i]")
            .unwrap()
            .matches(&element));
        assert!(Selector::parse("[data-mode=Dark s]")
            .unwrap()
            .matches(&element));
        assert!(!Selector::parse("[data-mode=dark s]")
            .unwrap()
            .matches(&element));
    }

    #[test]
    fn matches_not_is_and_where_pseudo_classes() {
        let mut div = Node::element("div");
        div.set_attribute("class", "card");

        assert!(Selector::parse("div:not(.missing)").unwrap().matches(&div));
        assert!(!Selector::parse("div:not(.card)").unwrap().matches(&div));
        assert!(Selector::parse("div:is(.card, .missing)")
            .unwrap()
            .matches(&div));
        assert!(Selector::parse("div:where(.card)").unwrap().matches(&div));
        assert_eq!(
            Selector::parse("div:not(#main)").unwrap().specificity(),
            Specificity::new(1, 0, 1)
        );
        assert_eq!(
            Selector::parse("div:where(#main)").unwrap().specificity(),
            Specificity::new(0, 0, 1)
        );
    }

    #[test]
    fn matches_type_based_structural_pseudo_classes() {
        let mut body = Node::element("body");
        let first_div = Node::element("div");
        let span = Node::element("span");
        let second_div = Node::element("div");
        let third_div = Node::element("div");
        body.append(first_div);
        body.append(span);
        body.append(second_div);
        body.append(third_div);

        let siblings = body.children.as_slice();
        let lists: [&[Node]; 2] = [&[], siblings];

        assert!(Selector::parse("div:first-of-type")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[0]], &lists, &[0, 0]));
        assert!(Selector::parse("div:nth-of-type(2)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[2]], &lists, &[0, 2]));
        assert!(Selector::parse("div:nth-last-of-type(1)")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[3]], &lists, &[0, 3]));
        assert!(Selector::parse("div:last-of-type")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[3]], &lists, &[0, 3]));
        assert!(Selector::parse("span:only-of-type")
            .unwrap()
            .matches_path_with_siblings(&[&body, &siblings[1]], &lists, &[0, 1]));
    }

    #[test]
    fn parses_type_and_class() {
        let selector = Selector::parse(".card").unwrap();
        assert!(selector.matches(&node()));
        assert!(!Selector::parse("span").unwrap().matches(&node()));
    }
}
