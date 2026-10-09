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
        !self.pattern.is_empty() && url.contains(&self.pattern)
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

fn party_context(url: &str, first_party: Option<&crate::net::Url>) -> Option<PartyContext> {
    let first_party = first_party?;
    let requested = crate::net::Url::parse(url).ok()?;
    let same_origin = requested.scheme() == first_party.scheme()
        && requested.host().eq_ignore_ascii_case(first_party.host())
        && requested.effective_port() == first_party.effective_port();
    Some(if same_origin {
        PartyContext::FirstParty
    } else {
        PartyContext::ThirdParty
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
