use std::collections::HashMap;

use super::{dom::Node, tokenizer::HtmlToken};

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

const MAX_DOM_DEPTH: usize = 256;
const MAX_DOM_TAG_NAME_BYTES: usize = 1_024;
const MAX_DOM_NODES: usize = 50_000;
const MAX_DOM_ATTRIBUTE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DOM_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DOM_TOKENS: usize = 100_000;

const BLOCK_CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "table",
    "ul",
];

pub fn parse(tokens: &[HtmlToken]) -> Node {
    let mut root = Node::document();
    let mut stack: Vec<Node> = Vec::new();
    // Keep malformed hostile nesting from creating a tree too deep for
    // recursive DOM consumers. Once the limit is reached, discard that subtree;
    // an indexed tag stack lets matching end tags recover without repeated
    // linear scans through attacker-controlled nesting.
    // Borrow tag names from the input tokens instead of cloning attacker-controlled strings.
    // This keeps discarded deep subtrees from multiplying tag-name storage in memory.
    let mut suppressed_stack: Vec<&str> = Vec::new();
    // Index open suppressed tags by name so matching close tags do not scan the
    // entire attacker-controlled nesting stack on every token.
    let mut suppressed_positions: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut created_nodes = 0usize;
    // Direct-token callers can bypass the tokenizer's input-byte ceiling. Bound
    // copied attribute storage independently across the retained DOM.
    let mut retained_attribute_bytes = 0usize;
    // Direct-token callers can bypass the tokenizer's input-byte ceiling. Bound
    // copied text and comments so retained DOM payloads stay finite as well.
    let mut retained_text_bytes = 0usize;

    for token in tokens.iter().take(MAX_DOM_TOKENS) {
        if !suppressed_stack.is_empty() {
            match token {
                HtmlToken::StartTag { name, .. } if !VOID_ELEMENTS.contains(&name.as_str()) => {
                    let position = suppressed_stack.len();
                    if !push_suppressed(
                        &mut suppressed_stack,
                        &mut suppressed_positions,
                        name,
                        position,
                    ) {
                        break;
                    }
                }
                HtmlToken::EndTag(name) if !VOID_ELEMENTS.contains(&name.as_str()) => {
                    // Mirror the parser's matching-end-tag behavior inside the
                    // discarded subtree. Unmatched/void end tags must not reduce
                    // the suppression depth and expose hostile nested markup.
                    if let Some(position) = suppressed_positions
                        .get(name.as_str())
                        .and_then(|positions| positions.last())
                        .copied()
                    {
                        while suppressed_stack.len() > position {
                            if let Some(open) = suppressed_stack.pop() {
                                let remove_entry =
                                    if let Some(positions) = suppressed_positions.get_mut(open) {
                                        positions.pop();
                                        positions.is_empty()
                                    } else {
                                        false
                                    };
                                if remove_entry {
                                    suppressed_positions.remove(open);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        match token {
            HtmlToken::Doctype(_) => {}
            HtmlToken::Comment(comment) => {
                if created_nodes >= MAX_DOM_NODES {
                    continue;
                }
                let remaining = MAX_DOM_TEXT_BYTES.saturating_sub(retained_text_bytes);
                let retained = bounded_utf8_prefix(comment, remaining);
                if retained.is_empty() {
                    continue;
                }
                if !append_node(&mut root, &mut stack, Node::comment(retained)) {
                    break;
                }
                retained_text_bytes += retained.len();
                created_nodes += 1;
            }
            HtmlToken::Text(text) => {
                if text.is_empty() || created_nodes >= MAX_DOM_NODES {
                    continue;
                }
                let remaining = MAX_DOM_TEXT_BYTES.saturating_sub(retained_text_bytes);
                let retained = bounded_utf8_prefix(text, remaining);
                if retained.is_empty() {
                    continue;
                }
                if !append_node(&mut root, &mut stack, Node::text(retained)) {
                    break;
                }
                retained_text_bytes += retained.len();
                created_nodes += 1;
            }
            HtmlToken::StartTag {
                name,
                attributes,
                self_closing: _,
            } => {
                // Direct-token callers can bypass the tokenizer's tag-name cap.
                // Reject oversized names before cloning them into the DOM or stacks.
                if name.len() > MAX_DOM_TAG_NAME_BYTES {
                    continue;
                }
                // HTML ignores a nested form start tag while a form is open.
                if name == "form" && has_open(&stack, "form") {
                    continue;
                }
                // Bound retained DOM nodes independently of token and byte limits.
                // Once saturated, discard each new subtree without allocating a Node
                // or cloning its attributes; matching end tags recover below.
                if created_nodes >= MAX_DOM_NODES {
                    if !VOID_ELEMENTS.contains(&name.as_str())
                        && !push_suppressed(
                            &mut suppressed_stack,
                            &mut suppressed_positions,
                            name,
                            0,
                        )
                    {
                        break;
                    }
                    continue;
                }

                // Apply the most common HTML implied-end-tag rules before
                // inserting a new element. Without these, malformed-but-common
                // markup such as adjacent anchors or options produces a DOM
                // tree different from browser expectations.
                for implied_end in implied_end_tags_for_start(name) {
                    if has_open(&stack, implied_end) {
                        close_element(&mut root, &mut stack, implied_end);
                    }
                }

                if (name == "p" || is_block_closing_p(name)) && has_open(&stack, "p") {
                    close_element(&mut root, &mut stack, "p");
                }

                // A row directly inside a table starts an implicit tbody in
                // the HTML tree builder. Keep this scoped to the table being
                // the current insertion parent; foster parenting is separate.
                if name == "tr"
                    && stack
                        .last()
                        .is_some_and(|node| node.tag_name() == Some("table"))
                {
                    if created_nodes >= MAX_DOM_NODES - 1 {
                        if !push_suppressed(
                            &mut suppressed_stack,
                            &mut suppressed_positions,
                            name,
                            0,
                        ) {
                            break;
                        }
                        continue;
                    }
                    if stack.try_reserve(1).is_err() {
                        break;
                    }
                    stack.push(Node::element("tbody"));
                    created_nodes += 1;
                }

                if name == "li" && has_open_list_item_in_current_list(&stack) {
                    close_element(&mut root, &mut stack, "li");
                }

                // Refuse over-deep elements before allocating a Node and copying
                // attacker-controlled attributes. Void elements remain valid children
                // at the maximum nesting depth.
                if !VOID_ELEMENTS.contains(&name.as_str()) && stack.len() >= MAX_DOM_DEPTH {
                    if !push_suppressed(&mut suppressed_stack, &mut suppressed_positions, name, 0) {
                        break;
                    }
                    continue;
                }

                let mut node = Node::element(name.clone());
                for (attribute, value) in attributes {
                    let Some(attribute_bytes) = attribute.len().checked_add(value.len()) else {
                        continue;
                    };
                    let Some(next_attribute_bytes) =
                        retained_attribute_bytes.checked_add(attribute_bytes)
                    else {
                        continue;
                    };
                    if next_attribute_bytes > MAX_DOM_ATTRIBUTE_BYTES {
                        continue;
                    }
                    node.set_attribute(attribute, value);
                    retained_attribute_bytes = next_attribute_bytes;
                }

                // In HTML syntax, the self-closing flag is ignored for
                // ordinary HTML elements. Only void elements close without
                // an end tag; treating <div/> as empty changes the DOM tree.
                created_nodes += 1;
                if VOID_ELEMENTS.contains(&name.as_str()) {
                    if !append_node(&mut root, &mut stack, node) {
                        break;
                    }
                } else {
                    // Depth is bounded, but Vec capacity growth can still fail.
                    // Stop consuming untrusted tokens instead of using infallible growth.
                    if stack.try_reserve(1).is_err() {
                        break;
                    }
                    stack.push(node);
                }
            }
            HtmlToken::EndTag(name) => {
                // Avoid expensive comparisons against every open element when a
                // direct-token caller supplies an unbounded closing-tag name.
                if name.len() <= MAX_DOM_TAG_NAME_BYTES {
                    close_element(&mut root, &mut stack, name);
                }
            }
        }
    }

    while let Some(node) = stack.pop() {
        append_node(&mut root, &mut stack, node);
    }

    normalize_document_structure(root)
}

fn bounded_utf8_prefix(input: &str, max_bytes: usize) -> &str {
    let mut end = input.len().min(max_bytes);
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

/// Add a tag to the suppression tracker without infallible collection growth.
/// Returning false tells the parser to stop consuming input.
fn push_suppressed<'a>(
    stack: &mut Vec<&'a str>,
    positions: &mut HashMap<&'a str, Vec<usize>>,
    name: &'a str,
    position: usize,
) -> bool {
    if stack.try_reserve(1).is_err() {
        return false;
    }
    if !positions.contains_key(name) && positions.try_reserve(1).is_err() {
        return false;
    }
    let entries = positions.entry(name).or_default();
    if entries.try_reserve(1).is_err() {
        return false;
    }
    stack.push(name);
    entries.push(position);
    true
}

fn append_node(root: &mut Node, stack: &mut [Node], node: Node) -> bool {
    let parent = stack.last_mut().unwrap_or(root);
    if let super::dom::NodeKind::Text(text) = &node.kind {
        if let Some(last) = parent.children.last_mut() {
            if let super::dom::NodeKind::Text(last_text) = &mut last.kind {
                if last_text.try_reserve(text.len()).is_err() {
                    return false;
                }
                last_text.push_str(text);
                return true;
            }
        }
    }
    parent.try_append(node).is_ok()
}

fn append_node_owned(root: &mut Node, stack: &mut [Node], node: Node) -> Result<(), Node> {
    if let Some(parent) = stack.last_mut() {
        parent.try_append(node)
    } else {
        root.try_append(node)
    }
}

fn has_open(stack: &[Node], name: &str) -> bool {
    stack.iter().any(|node| node.tag_name() == Some(name))
}

fn has_open_list_item_in_current_list(stack: &[Node]) -> bool {
    for node in stack.iter().rev() {
        match node.tag_name() {
            Some("li") => return true,
            Some("ol" | "ul") => return false,
            _ => {}
        }
    }
    false
}

fn implied_end_tags_for_start(name: &str) -> &'static [&'static str] {
    match name {
        "a" => &["a"],
        "button" => &["button"],
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => &["h1", "h2", "h3", "h4", "h5", "h6"],
        "dt" | "dd" => &["dt", "dd"],
        "option" => &["option"],
        "optgroup" => &["option", "optgroup"],
        "tr" => &["tr"],
        "td" | "th" => &["td", "th"],
        "thead" | "tbody" | "tfoot" => &["thead", "tbody", "tfoot"],
        "rt" | "rp" => &["rt", "rp"],
        _ => &[],
    }
}

fn is_block_closing_p(name: &str) -> bool {
    BLOCK_CLOSES_P.contains(&name)
}

fn close_element(root: &mut Node, stack: &mut Vec<Node>, name: &str) {
    let Some(position) = stack.iter().rposition(|node| node.tag_name() == Some(name)) else {
        return;
    };

    let mut completed = Vec::new();
    // Reserve before popping anything: if allocation fails, leave the open
    // element stack untouched instead of losing nodes during error recovery.
    if completed.try_reserve(stack.len() - position).is_err() {
        return;
    }
    while stack.len() > position {
        if let Some(node) = stack.pop() {
            completed.push(node);
        }
    }

    let Some(mut matching) = completed.pop() else {
        // A matching stack position should always yield a node, but malformed
        // input must never turn an internal invariant violation into a panic.
        return;
    };

    // Reserve all child slots before changing the detached subtree. If this
    // fails, restore the original open-element stack in its original order.
    if matching.children.try_reserve(completed.len()).is_err() {
        stack.push(matching);
        stack.extend(completed.into_iter().rev());
        return;
    }
    while let Some(node) = completed.pop() {
        // The capacity was reserved above, so this path cannot need to grow.
        matching.children.push(node);
    }

    if let Err(node) = append_node_owned(root, stack, matching) {
        // Keep the node alive and available for finalization if the destination
        // parent cannot grow. The parser's stack retains its previous capacity.
        stack.push(node);
    }
}

fn normalize_document_structure(mut root: Node) -> Node {
    let root_child_count = root.children.len();
    let Some(html_position) = root
        .children
        .iter()
        .position(|node| node.tag_name() == Some("html"))
    else {
        let mut html = Node::element("html");
        let mut head = Node::element("head");
        let mut body = Node::element("body");

        // Reserve every destination before moving any input children. Draining
        // keeps the root vector's allocation for the final push.
        if root.children.try_reserve(1).is_err()
            || html.children.try_reserve(2).is_err()
            || head.children.try_reserve(root_child_count).is_err()
            || body.children.try_reserve(root_child_count).is_err()
        {
            return root;
        }

        for child in root.children.drain(..) {
            if child.tag_name() == Some("head") {
                head.children.push(child);
            } else {
                body.children.push(child);
            }
        }

        html.children.push(head);
        html.children.push(body);
        root.children.push(html);
        return root;
    };

    // Preflight all vector growth before detaching the explicit html subtree.
    // Siblings around html are moved into body in source order.
    let html_child_count = root.children[html_position].children.len();
    let body_position = root.children[html_position]
        .children
        .iter()
        .position(|node| node.tag_name() == Some("body"));
    let body_child_count = body_position.map_or(0, |position| {
        root.children[html_position].children[position]
            .children
            .len()
    });
    let Some(body_capacity) = root_child_count
        .saturating_sub(1)
        .checked_add(body_child_count)
        .and_then(|count| count.checked_add(html_child_count))
    else {
        return root;
    };

    let mut preceding = Vec::new();
    let mut following = Vec::new();
    let mut body_extras = Vec::new();
    if root.children.try_reserve(1).is_err()
        || preceding.try_reserve(body_capacity).is_err()
        || following.try_reserve(root_child_count).is_err()
        || body_extras.try_reserve(html_child_count).is_err()
        || root.children[html_position]
            .children
            .try_reserve(2)
            .is_err()
    {
        return root;
    }

    let mut html = root.children.remove(html_position);
    for (position, child) in root.children.drain(..).enumerate() {
        if position < html_position {
            preceding.push(child);
        } else {
            following.push(child);
        }
    }

    let mut head = None;
    let mut body = None;
    for child in html.children.drain(..) {
        match child.tag_name() {
            Some("head") if head.is_none() => head = Some(child),
            Some("body") if body.is_none() => body = Some(child),
            _ => body_extras.push(child),
        }
    }

    let mut body = body.unwrap_or_else(|| Node::element("body"));
    preceding.append(&mut body.children);
    preceding.extend(body_extras);
    preceding.extend(following);
    body.children = preceding;

    html.children
        .push(head.unwrap_or_else(|| Node::element("head")));
    html.children.push(body);
    root.children.push(html);
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::tokenizer::HtmlTokenizer;

    #[test]
    fn suppressed_stack_tracker_keeps_stack_and_index_consistent() {
        let mut stack = Vec::new();
        let mut positions = HashMap::new();
        assert!(push_suppressed(&mut stack, &mut positions, "div", 0));
        assert!(push_suppressed(&mut stack, &mut positions, "span", 1));
        assert!(push_suppressed(&mut stack, &mut positions, "div", 2));
        assert_eq!(stack, vec!["div", "span", "div"]);
        assert_eq!(
            positions.get("div").map(Vec::as_slice),
            Some([0, 2].as_slice())
        );
        assert_eq!(
            positions.get("span").map(Vec::as_slice),
            Some([1].as_slice())
        );
    }

    #[test]
    fn excessive_nesting_is_discarded_without_corrupting_following_siblings() {
        let mut html = "<div>".repeat(MAX_DOM_DEPTH + 32);
        html.push_str("discarded");
        html.push_str(&"</div>".repeat(MAX_DOM_DEPTH + 32));
        html.push_str("<p>visible</p>");

        let root = parse(&HtmlTokenizer::tokenize(&html));

        assert_eq!(root.text_content(), "visible");
        assert!(root.find_first_element("p").is_some());
    }

    #[test]
    fn unmatched_and_void_end_tags_cannot_escape_suppressed_subtree() {
        let mut html = "<div>".repeat(MAX_DOM_DEPTH + 1);
        html.push_str("hidden</br></unknown></img>");
        html.push_str(&"<span>".repeat(8));
        html.push_str("still hidden</div>");
        html.push_str(&"</span>".repeat(8));
        html.push_str(&"</div>".repeat(MAX_DOM_DEPTH));
        html.push_str("<p>visible</p>");

        let root = parse(&HtmlTokenizer::tokenize(&html));

        assert_eq!(root.text_content(), "visible");
        assert!(root.find_first_element("p").is_some());
        assert!(root.find_first_element("span").is_none());
    }

    #[test]
    fn matching_ancestor_end_tag_recovers_after_suppressed_subtree() {
        let mut html = "<div>".repeat(MAX_DOM_DEPTH + 1);
        html.push_str("hidden</div>");
        html.push_str(&"</div>".repeat(MAX_DOM_DEPTH));
        html.push_str("<p>visible</p>");

        let root = parse(&HtmlTokenizer::tokenize(&html));

        assert_eq!(root.text_content(), "visible");
        assert!(root.find_first_element("p").is_some());
    }

    #[test]
    fn parser_caps_direct_token_input_even_without_the_tokenizer() {
        let mut tokens = vec![HtmlToken::Doctype(String::new()); MAX_DOM_TOKENS];
        tokens.push(HtmlToken::Text("beyond-limit".to_owned()));

        let root = parse(&tokens);

        assert!(root.text_content().is_empty());
    }

    #[test]
    fn parser_caps_retained_dom_nodes_even_for_direct_tokens() {
        let tokens = vec![HtmlToken::Text("x".to_owned()); MAX_DOM_NODES + 100];

        let root = parse(&tokens);
        let body = root
            .find_first_element("body")
            .expect("normalization always creates a body");

        // Adjacent text tokens coalesce into one DOM Text node, but each
        // retained token still counts against the parser's node budget.
        assert_eq!(body.child_count(), 1);
        assert_eq!(body.text_content().len(), MAX_DOM_NODES);
    }

    #[test]
    fn direct_tokens_cannot_bypass_the_dom_tag_name_byte_budget() {
        let oversized_name = "x".repeat(MAX_DOM_TAG_NAME_BYTES + 1);
        let tokens = vec![
            HtmlToken::StartTag {
                name: oversized_name.clone(),
                attributes: Default::default(),
                self_closing: false,
            },
            HtmlToken::EndTag(oversized_name),
            HtmlToken::StartTag {
                name: "p".to_owned(),
                attributes: Default::default(),
                self_closing: false,
            },
            HtmlToken::Text("retained".to_owned()),
            HtmlToken::EndTag("p".to_owned()),
        ];

        let root = parse(&tokens);
        let paragraph = root
            .find_first_element("p")
            .expect("valid paragraph is retained");
        assert_eq!(paragraph.text_content(), "retained");
        assert!(root
            .find_first_element(&"x".repeat(MAX_DOM_TAG_NAME_BYTES + 1))
            .is_none());
    }

    #[test]
    fn direct_tokens_cannot_bypass_the_dom_attribute_byte_budget() {
        let oversized_value = "x".repeat(MAX_DOM_ATTRIBUTE_BYTES + 1);
        let tokens = vec![
            HtmlToken::StartTag {
                name: "div".to_owned(),
                attributes: [("data-huge".to_owned(), oversized_value)]
                    .into_iter()
                    .collect(),
                self_closing: false,
            },
            HtmlToken::StartTag {
                name: "p".to_owned(),
                attributes: [("id".to_owned(), "retained".to_owned())]
                    .into_iter()
                    .collect(),
                self_closing: false,
            },
            HtmlToken::EndTag("p".to_owned()),
            HtmlToken::EndTag("div".to_owned()),
        ];

        let root = parse(&tokens);
        let paragraph = root.find_first_element("p").expect("paragraph is retained");

        assert_eq!(paragraph.attribute("id"), Some("retained"));
        assert_eq!(
            root.find_first_element("div")
                .expect("div is retained")
                .attribute("data-huge"),
            None,
            "an attribute beyond the shared byte budget must not be copied into the DOM"
        );
    }

    #[test]
    fn direct_tokens_cannot_bypass_the_dom_text_and_comment_byte_budget() {
        use super::super::dom::NodeKind;

        let comment = "c".repeat(MAX_DOM_TEXT_BYTES / 2);
        let text = "é".repeat(MAX_DOM_TEXT_BYTES);
        let tokens = vec![HtmlToken::Comment(comment), HtmlToken::Text(text)];

        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let NodeKind::Comment(comment) = &body.children[0].kind else {
            panic!("expected retained comment");
        };
        let NodeKind::Text(text) = &body.children[1].kind else {
            panic!("expected retained text");
        };

        assert_eq!(comment.len(), MAX_DOM_TEXT_BYTES / 2);
        assert_eq!(
            comment.len() + text.len(),
            MAX_DOM_TEXT_BYTES,
            "comment and text payloads share one retained-byte budget"
        );
        assert!(text.is_char_boundary(text.len()));
        assert_eq!(root.text_content().len(), text.len());
    }

    #[test]
    fn adjacent_text_tokens_are_merged_without_crossing_comments() {
        let tokens = vec![
            HtmlToken::Text("one".to_owned()),
            HtmlToken::Text("two".to_owned()),
            HtmlToken::Comment("separator".to_owned()),
            HtmlToken::Text("three".to_owned()),
            HtmlToken::Text("four".to_owned()),
        ];
        let root = parse(&tokens);
        let body = root.find_first_element("body").expect("body is normalized");
        assert_eq!(body.children.len(), 3);
        assert_eq!(body.children[0].text_content(), "onetwo");
        assert!(matches!(
            body.children[1].kind,
            super::super::dom::NodeKind::Comment(_)
        ));
        assert_eq!(body.children[2].text_content(), "threefour");
    }

    #[test]
    fn builds_basic_tree() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].tag_name(), Some("html"));
        assert_eq!(root.children[0].children[0].tag_name(), Some("head"));
        assert_eq!(root.children[0].children[1].tag_name(), Some("body"));
    }

    #[test]
    fn preserves_nested_children_when_closing_parent() {
        let tokens = HtmlTokenizer::tokenize("<div><span>hello</span><b>world</b></div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let div = &body.children[0];
        assert_eq!(div.children.len(), 2);
        assert_eq!(root.text_content(), "helloworld");
    }

    #[test]
    fn self_closing_syntax_does_not_close_non_void_html_elements() {
        let tokens = HtmlTokenizer::tokenize("<div/>inside</div><p/>text</p>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];

        assert_eq!(body.children[0].tag_name(), Some("div"));
        assert_eq!(body.children[0].text_content(), "inside");
        assert_eq!(body.children[1].tag_name(), Some("p"));
        assert_eq!(body.children[1].text_content(), "text");
    }

    #[test]
    fn obsolete_void_elements_do_not_capture_following_text() {
        for tag in ["basefont", "bgsound", "frame", "keygen"] {
            let markup = format!("<div><{tag}>after</div>");
            let tokens = HtmlTokenizer::tokenize(&markup);
            let root = parse(&tokens);
            let body = &root.children[0].children[1];
            let div = &body.children[0];

            assert_eq!(div.children[0].tag_name(), Some(tag));
            assert_eq!(div.children[1].text_content(), "after");
        }
    }

    #[test]
    fn void_elements_do_not_capture_following_text() {
        let tokens = HtmlTokenizer::tokenize("<div><br>after</div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let div = &body.children[0];
        assert_eq!(div.children[0].tag_name(), Some("br"));
        assert_eq!(div.children[1].text_content(), "after");
    }

    #[test]
    fn p_auto_closes_before_additional_html_block_elements() {
        for tag in [
            "details",
            "figcaption",
            "figure",
            "hgroup",
            "main",
            "search",
        ] {
            let markup = format!("<p>before<{tag}>inside</{tag}>");
            let tokens = HtmlTokenizer::tokenize(&markup);
            let root = parse(&tokens);
            let body = &root.children[0].children[1];

            assert_eq!(body.children[0].tag_name(), Some("p"), "tag: {tag}");
            assert_eq!(body.children[0].text_content(), "before", "tag: {tag}");
            assert_eq!(body.children[1].tag_name(), Some(tag), "tag: {tag}");
        }
    }

    #[test]
    fn p_auto_closes_before_block_content() {
        let tokens = HtmlTokenizer::tokenize("<p>one<div>two</div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        assert_eq!(body.children.len(), 2);
        assert_eq!(body.children[0].tag_name(), Some("p"));
        assert_eq!(body.children[1].tag_name(), Some("div"));
    }

    #[test]
    fn nested_list_items_do_not_close_the_outer_list_item() {
        let tokens = HtmlTokenizer::tokenize(
            "<ul><li>outer<ul><li>inner</li></ul></li><li>second</li></ul>",
        );
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let outer_list = &body.children[0];
        let outer_item = &outer_list.children[0];
        let nested_list = outer_item.find_first_element("ul").unwrap();

        assert_eq!(outer_item.text_content(), "outerinner");
        assert_eq!(nested_list.children.len(), 1);
        assert_eq!(nested_list.children[0].text_content(), "inner");
        assert_eq!(outer_list.children[1].text_content(), "second");
    }

    #[test]
    fn adjacent_anchors_and_buttons_do_not_nest() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<a href='/first'>one<a href='/second'>two</a><button>first<button>second</button>",
        ));
        let body = &root.children[0].children[1];

        assert_eq!(body.children[0].tag_name(), Some("a"));
        assert_eq!(body.children[0].text_content(), "one");
        assert_eq!(body.children[1].tag_name(), Some("a"));
        assert_eq!(body.children[1].text_content(), "two");
        assert_eq!(body.children[2].tag_name(), Some("button"));
        assert_eq!(body.children[2].text_content(), "first");
        assert_eq!(body.children[3].tag_name(), Some("button"));
        assert_eq!(body.children[3].text_content(), "second");
    }

    #[test]
    fn headings_and_definition_terms_close_previous_peers() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<h1>one<h2>two</h2><dl><dt>term<dd>definition</dd></dl>",
        ));
        let body = &root.children[0].children[1];

        assert_eq!(body.children[0].tag_name(), Some("h1"));
        assert_eq!(body.children[0].text_content(), "one");
        assert_eq!(body.children[1].tag_name(), Some("h2"));
        let list = &body.children[2];
        assert_eq!(list.children[0].tag_name(), Some("dt"));
        assert_eq!(list.children[0].text_content(), "term");
        assert_eq!(list.children[1].tag_name(), Some("dd"));
        assert_eq!(list.children[1].text_content(), "definition");
    }

    #[test]
    fn options_close_before_the_next_option_or_group() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<select><option>one<option>two<optgroup><option>three</optgroup></select>",
        ));
        let select = root.find_first_element("select").unwrap();

        assert_eq!(select.children[0].tag_name(), Some("option"));
        assert_eq!(select.children[0].text_content(), "one");
        assert_eq!(select.children[1].tag_name(), Some("option"));
        assert_eq!(select.children[1].text_content(), "two");
        assert_eq!(select.children[2].tag_name(), Some("optgroup"));
        assert_eq!(select.children[2].children[0].tag_name(), Some("option"));
        assert_eq!(select.children[2].children[0].text_content(), "three");
    }

    #[test]
    fn table_rows_cells_and_sections_close_previous_peers() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<table><tbody><tr><td>one<td>two<tr><th>three</tbody><tbody><tr><td>four",
        ));
        let table = root.find_first_element("table").expect("table retained");
        assert_eq!(table.children.len(), 2);
        let first_section = &table.children[0];
        assert_eq!(first_section.tag_name(), Some("tbody"));
        assert_eq!(first_section.children.len(), 2);
        assert_eq!(first_section.children[0].tag_name(), Some("tr"));
        assert_eq!(first_section.children[0].children.len(), 2);
        assert_eq!(first_section.children[0].children[0].text_content(), "one");
        assert_eq!(first_section.children[0].children[1].text_content(), "two");
        assert_eq!(
            first_section.children[1].children[0].text_content(),
            "three"
        );
        let second_section = &table.children[1];
        assert_eq!(second_section.tag_name(), Some("tbody"));
        assert_eq!(
            second_section.children[0].children[0].text_content(),
            "four"
        );
    }

    #[test]
    fn nested_form_start_tags_are_ignored() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<form><input id='first'><form><input id='second'></form><input id='third'>",
        ));
        let forms: Vec<_> = root
            .descendants()
            .filter(|node| node.tag_name() == Some("form"))
            .collect();
        assert_eq!(forms.len(), 1);
        assert!(forms[0].find_by_id("first").is_some());
        assert!(forms[0].find_by_id("second").is_some());
        assert!(forms[0].find_by_id("third").is_none());
    }

    #[test]
    fn table_rows_without_explicit_section_receive_implicit_tbody() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<table><tr><td>one<td>two<tr><td>three</table>",
        ));
        let table = root.find_first_element("table").expect("table retained");
        assert_eq!(table.children.len(), 1);
        let tbody = &table.children[0];
        assert_eq!(tbody.tag_name(), Some("tbody"));
        assert_eq!(tbody.children.len(), 2);
        assert_eq!(tbody.children[0].children.len(), 2);
        assert_eq!(tbody.children[0].children[0].text_content(), "one");
        assert_eq!(tbody.children[0].children[1].text_content(), "two");
        assert_eq!(tbody.children[1].children[0].text_content(), "three");
    }

    #[test]
    fn ruby_annotation_peers_close_implicitly() {
        let root = parse(&HtmlTokenizer::tokenize("<ruby><rt>one<rp>(<rt>two"));
        let ruby = root.find_first_element("ruby").expect("ruby retained");
        assert_eq!(ruby.children.len(), 3);
        assert_eq!(ruby.children[0].tag_name(), Some("rt"));
        assert_eq!(ruby.children[0].text_content(), "one");
        assert_eq!(ruby.children[1].tag_name(), Some("rp"));
        assert_eq!(ruby.children[2].tag_name(), Some("rt"));
        assert_eq!(ruby.children[2].text_content(), "two");
    }

    #[test]
    fn comments_are_kept_in_the_dom() {
        let tokens = HtmlTokenizer::tokenize("<div><!-- note --></div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let div = &body.children[0];
        assert!(matches!(
            div.children[0].kind,
            super::super::dom::NodeKind::Comment(_)
        ));
    }

    #[test]
    fn plain_content_gets_implicit_html_head_and_body() {
        let tokens = HtmlTokenizer::tokenize("Hello <p>world</p>");
        let root = parse(&tokens);
        let html = &root.children[0];
        assert_eq!(html.tag_name(), Some("html"));
        assert_eq!(html.children[0].tag_name(), Some("head"));
        assert_eq!(html.children[1].tag_name(), Some("body"));
        assert_eq!(html.children[1].text_content(), "Hello world");
    }

    #[test]
    fn content_outside_explicit_html_is_moved_into_body_in_order() {
        let tokens = HtmlTokenizer::tokenize(
            "before<html><head><title>title</title></head><body>inside</body></html>after",
        );
        let root = parse(&tokens);

        assert_eq!(root.children.len(), 1);
        let html = &root.children[0];
        let body = &html.children[1];
        assert_eq!(body.text_content(), "beforeinsideafter");
    }

    #[test]
    fn normalization_preserves_document_order_and_builds_one_html_head_body() {
        let tokens = HtmlTokenizer::tokenize(
            "before<html><head><title>T</title></head><main>inside</main></html>after",
        );
        let root = parse(&tokens);
        assert_eq!(root.child_count(), 1);
        let html = root.child(0).expect("normalized html");
        assert_eq!(html.tag_name(), Some("html"));
        assert_eq!(html.child_count(), 2);
        assert_eq!(html.child(0).and_then(Node::tag_name), Some("head"));
        assert_eq!(html.child(1).and_then(Node::tag_name), Some("body"));
        assert_eq!(html.text_content(), "Tbeforeinsideafter");
        let body = html.child(1).expect("normalized body");
        assert_eq!(
            body.children()
                .iter()
                .map(Node::tag_name)
                .collect::<Vec<_>>(),
            vec![None, Some("main"), None]
        );
    }

    #[test]
    fn normalization_keeps_duplicate_sections_in_body_without_losing_content() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<html><head>first</head><head>second</head><body>body</body><body>extra</body></html>",
        ));
        let html = root.child(0).expect("normalized html");
        assert_eq!(html.child_count(), 2);
        assert_eq!(html.child(0).and_then(Node::tag_name), Some("head"));
        assert_eq!(html.child(1).and_then(Node::tag_name), Some("body"));
        assert_eq!(html.text_content(), "firstbodysecondextra");
        assert_eq!(
            html.child(1).expect("normalized body").text_content(),
            "bodysecondextra"
        );
    }

    #[test]
    fn explicit_html_gets_an_implicit_head() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        let html = &root.children[0];
        assert_eq!(html.children[0].tag_name(), Some("head"));
        assert_eq!(html.children[1].tag_name(), Some("body"));
    }
}
