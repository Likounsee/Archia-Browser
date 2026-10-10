#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDecision {
    Allow,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyContext {
    FirstParty,
    ThirdParty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    Script,
    Image,
    Style,
    Font,
    Media,
    Xhr,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRule {
    pattern: String,
    decision: FilterDecision,
    resource: Option<ResourceType>,
    party: Option<PartyContext>,
}

impl FilterRule {
    pub fn block(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            decision: FilterDecision::Block,
            resource: None,
            party: None,
        }
    }

    pub fn allow(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            decision: FilterDecision::Allow,
            resource: None,
            party: None,
        }
    }

    pub fn for_resource(mut self, resource: ResourceType) -> Self {
        self.resource = Some(resource);
        self
    }

    pub fn for_party(mut self, party: PartyContext) -> Self {
        self.party = Some(party);
        self
    }

    pub fn matches(&self, url: &str, resource: Option<ResourceType>) -> bool {
        self.matches_with_party(url, resource, None)
    }

    pub fn matches_with_party(
        &self,
        url: &str,
        resource: Option<ResourceType>,
        first_party: Option<&crate::net::Url>,
    ) -> bool {
        self.pattern_matches(url)
            && self
                .resource
                .is_none_or(|expected| Some(expected) == resource)
            && self
                .party
                .is_none_or(|expected| Some(expected) == party_context(url, first_party))
    }

    fn pattern_matches(&self, url: &str) -> bool {
        // An empty substring matches every URL; treating an accidental empty
        // rule as valid could silently disable every request when blocking.
        if self.pattern.is_empty() {
            return false;
        }

        // Servers commonly treat percent-encoded unreserved ASCII bytes as
        // their literal characters. Match that equivalent spelling as well,
        // or a rule for "/blocked" could be bypassed with "/%62locked".
        let normalized_url = decode_unreserved_percent_escapes(url);
        let normalized_pattern = decode_unreserved_percent_escapes(&self.pattern);
        url.contains(&self.pattern)
            || url.contains(&normalized_pattern)
            || normalized_url.contains(&self.pattern)
            || normalized_url.contains(&normalized_pattern)
    }
    pub fn decision(&self) -> FilterDecision {
        self.decision
    }
}

#[derive(Debug, Default)]
pub struct RequestFilter {
    rules: Vec<FilterRule>,
}

impl RequestFilter {
    pub fn add_rule(&mut self, rule: FilterRule) {
        self.rules.push(rule);
    }
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn decide(&self, url: &str, resource: Option<ResourceType>) -> FilterDecision {
        self.decide_with_party(url, resource, None)
    }

    pub fn decide_with_party(
        &self,
        url: &str,
        resource: Option<ResourceType>,
        first_party: Option<&crate::net::Url>,
    ) -> FilterDecision {
        let mut blocked = false;

        for rule in &self.rules {
            if !rule.matches_with_party(url, resource, first_party) {
                continue;
            }
            match rule.decision() {
                FilterDecision::Allow => return FilterDecision::Allow,
                FilterDecision::Block => blocked = true,
            }
        }

        if blocked {
            FilterDecision::Block
        } else {
            FilterDecision::Allow
        }
    }
}

fn decode_unreserved_percent_escapes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let hex = |byte: u8| -> u8 {
                match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => 0,
                }
            };
            let decoded = (hex(bytes[index + 1]) << 4) | hex(bytes[index + 2]);
            if decoded.is_ascii_alphanumeric() || matches!(decoded, b'-' | b'.' | b'_' | b'~') {
                output.push(decoded as char);
                index += 3;
                continue;
            }
        }
        // Percent escapes and UTF-8 bytes are ASCII-preserved here; append
        // complete UTF-8 characters for non-ASCII input.
        let character = input[index..]
            .chars()
            .next()
            .expect("index stays on a character boundary");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn party_context(url: &str, first_party: Option<&crate::net::Url>) -> Option<PartyContext> {
    let first_party = first_party?;
    let requested = crate::net::Url::parse(url).ok()?;
    // Url values do not carry document opaque-origin identity. Conservatively
    // classify file URLs as cross-origin instead of collapsing all local files
    // into the same empty-host tuple.
    let same_origin = requested.scheme() != "file"
        && first_party.scheme() != "file"
        && requested.scheme() == first_party.scheme()
        && requested.host().eq_ignore_ascii_case(first_party.host())
        && origin_port(&requested) == origin_port(first_party);
    Some(if same_origin {
        PartyContext::FirstParty
    } else {
        PartyContext::ThirdParty
    })
}

fn origin_port(url: &crate::net::Url) -> Option<u16> {
    url.port().or(match url.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn party_context_keeps_opaque_files_and_missing_ports_cross_origin() {
        let first_file = crate::net::Url::parse("file:///private/first.html").unwrap();
        assert_eq!(
            party_context("file:///private/second.html", Some(&first_file)),
            Some(PartyContext::ThirdParty)
        );

        let implicit = crate::net::Url::parse("custom://example.org/resource").unwrap();
        assert_eq!(
            party_context("custom://example.org:0/resource", Some(&implicit)),
            Some(PartyContext::ThirdParty)
        );
        let implicit_https = crate::net::Url::parse("https://example.org/resource").unwrap();
        assert_eq!(
            party_context("https://example.org:443/resource", Some(&implicit_https)),
            Some(PartyContext::FirstParty)
        );
    }

    #[test]
    fn blocks_matching_request() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("ads.example"));
        assert_eq!(
            filter.decide("https://ads.example/banner.js", Some(ResourceType::Script)),
            FilterDecision::Block
        );
        assert_eq!(
            filter.decide("https://example.org/app.js", Some(ResourceType::Script)),
            FilterDecision::Allow
        );
    }

    #[test]
    fn filter_matches_percent_encoded_unreserved_path_characters() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("/blocked"));

        assert_eq!(
            filter.decide(
                "https://example.org/%62locked/resource",
                Some(ResourceType::Document)
            ),
            FilterDecision::Block
        );
        assert_eq!(
            filter.decide("https://example.org/%7Euser", Some(ResourceType::Document)),
            FilterDecision::Allow,
            "reserved policy patterns should not be broadened to unrelated paths"
        );
    }

    #[test]
    fn filter_normalizes_unreserved_escapes_in_patterns_too() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("/%62locked"));

        assert_eq!(
            filter.decide(
                "https://example.org/blocked/resource",
                Some(ResourceType::Document)
            ),
            FilterDecision::Block
        );
    }

    #[test]
    fn empty_pattern_does_not_match_every_request() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block(""));

        assert_eq!(
            filter.decide("https://example.org/", Some(ResourceType::Document)),
            FilterDecision::Allow
        );
    }

    #[test]
    fn allow_rule_overrides_a_matching_block() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("ads.example"));
        filter.add_rule(FilterRule::allow("ads.example/allowed"));
        assert_eq!(
            filter.decide("https://ads.example/allowed.js", Some(ResourceType::Script)),
            FilterDecision::Allow
        );
        assert_eq!(
            filter.decide("https://ads.example/banner.js", Some(ResourceType::Script)),
            FilterDecision::Block
        );
    }

    #[test]
    fn party_specific_rules_distinguish_origins() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("tracker.example").for_party(PartyContext::ThirdParty));

        let first_party = crate::net::Url::parse("https://site.example/").unwrap();
        assert_eq!(
            filter.decide_with_party(
                "https://tracker.example/pixel",
                Some(ResourceType::Image),
                Some(&first_party)
            ),
            FilterDecision::Block
        );
        let tracker = crate::net::Url::parse("https://tracker.example/").unwrap();
        assert_eq!(
            filter.decide_with_party(
                "https://tracker.example/pixel",
                Some(ResourceType::Image),
                Some(&tracker)
            ),
            FilterDecision::Allow
        );
    }

    #[test]
    fn resource_specific_rule_does_not_block_other_resources() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("cdn.example").for_resource(ResourceType::Image));
        assert_eq!(
            filter.decide("https://cdn.example/app.js", Some(ResourceType::Script)),
            FilterDecision::Allow
        );
        assert_eq!(
            filter.decide("https://cdn.example/ad.png", Some(ResourceType::Image)),
            FilterDecision::Block
        );
    }
}
