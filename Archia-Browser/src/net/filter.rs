#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDecision {
    Allow,
    Block,
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
}

impl FilterRule {
    pub fn block(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            decision: FilterDecision::Block,
            resource: None,
        }
    }

    pub fn allow(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            decision: FilterDecision::Allow,
            resource: None,
        }
    }

    pub fn for_resource(mut self, resource: ResourceType) -> Self {
        self.resource = Some(resource);
        self
    }

    pub fn matches(&self, url: &str, resource: Option<ResourceType>) -> bool {
        self.pattern_matches(url)
            && self
                .resource
                .is_none_or(|expected| Some(expected) == resource)
    }

    fn pattern_matches(&self, url: &str) -> bool {
        url.contains(&self.pattern)
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
        self.rules
            .iter()
            .find(|rule| rule.matches(url, resource))
            .map_or(FilterDecision::Allow, FilterRule::decision)
    }
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
