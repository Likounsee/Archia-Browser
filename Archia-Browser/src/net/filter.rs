#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDecision {
    Allow,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRule {
    pattern: String,
    decision: FilterDecision,
}

impl FilterRule {
    pub fn block(pattern: impl Into<String>) -> Self {
        Self { pattern: pattern.into(), decision: FilterDecision::Block }
    }

    pub fn allow(pattern: impl Into<String>) -> Self {
        Self { pattern: pattern.into(), decision: FilterDecision::Allow }
    }

    pub fn matches(&self, url: &str) -> bool {
        url.contains(&self.pattern)
    }

    pub fn decision(&self) -> FilterDecision {
        self.decision
    }
}

#[derive(Default)]
pub struct RequestFilter {
    rules: Vec<FilterRule>,
}

impl RequestFilter {
    pub fn add_rule(&mut self, rule: FilterRule) {
        self.rules.push(rule);
    }

    pub fn decide(&self, url: &str) -> FilterDecision {
        self.rules
            .iter()
            .find(|rule| rule.matches(url))
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
        assert_eq!(filter.decide("https://ads.example/banner.js"), FilterDecision::Block);
        assert_eq!(filter.decide("https://example.org/app.js"), FilterDecision::Allow);
    }
}
