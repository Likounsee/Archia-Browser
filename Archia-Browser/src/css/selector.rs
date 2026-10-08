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
                    c if bracket == 0 && (c.is_whitespace() || matches!(c, '>' | '+' | '~')) => {
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
            result.classes += part.classes.len() as u32
                + part.attributes.len() as u32
                + part.pseudo_classes.len() as u32;
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
        if path.is_empty() || path.len() < self.parts.len() {
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
        if path.is_empty()
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
        if !matches_simple(simple, path[node_index]) {
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
                let position = sibling_positions[node_index];
                position > 0
                    && sibling_lists[node_index]
                        .get(position - 1)
                        .is_some_and(|sibling| {
                            matches_simple(&self.parts[selector_index - 1].1, sibling)
                        })
            }
            Combinator::GeneralSibling => {
                let position = sibling_positions[node_index];
                sibling_lists[node_index]
                    .get(..position)
                    .is_some_and(|siblings| {
                        siblings.iter().rev().any(|sibling| {
                            matches_simple(&self.parts[selector_index - 1].1, sibling)
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
                simple.pseudo_classes.push(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .to_ascii_lowercase(),
                );
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
                    while i < chars.len()
                        && chars[i] != ']'
                        && quote.map_or(true, |q| chars[i] != q)
                    {
                        i += 1;
                    }
                    if start_value == i {
                        return None;
                    }
                    value = Some(chars[start_value..i].iter().collect());

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
                if i >= chars.len() || chars[i] != ']' {
                    return None;
                }
                i += 1;
                simple.attributes.push(AttributeSelector {
                    name,
                    operator,
                    value,
                });
            }
            _ => return None,
        }
    }

    Some(simple)
}
fn matches_pseudo_class(
    pseudo: &str,
    attributes: &std::collections::BTreeMap<String, String>,
) -> bool {
    match pseudo {
        "checked" => attributes.contains_key("checked"),
        "disabled" => attributes.contains_key("disabled"),
        "enabled" => !attributes.contains_key("disabled"),
        "required" => attributes.contains_key("required"),
        "optional" => !attributes.contains_key("required"),
        "read-only" => attributes.contains_key("readonly"),
        "read-write" => !attributes.contains_key("readonly"),
        _ => false,
    }
}

fn is_name_char(c: char) -> bool {
    c == '_' || c == '-' || c.is_ascii_alphanumeric() || !c.is_ascii()
}

fn matches_simple(simple: &SimpleSelector, node: &Node) -> bool {
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
        .any(|pseudo| !matches_pseudo_class(pseudo, attributes))
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
            AttributeOperator::Equals => actual == expected,
            AttributeOperator::Includes => actual.split_whitespace().any(|part| part == expected),
            AttributeOperator::DashMatch => {
                actual == expected || actual.starts_with(&format!("{expected}-"))
            }
            AttributeOperator::Prefix => actual.starts_with(expected),
            AttributeOperator::Suffix => actual.ends_with(expected),
            AttributeOperator::Substring => actual.contains(expected),
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
    fn parses_type_and_class() {
        let selector = Selector::parse(".card").unwrap();
        assert!(selector.matches(&node()));
        assert!(!Selector::parse("span").unwrap().matches(&node()));
    }
}
