/// Maps signal names to Zenoh key expressions.
///
/// Internal to the bus — users never see these keys.
pub struct SignalMapper {
    namespace: String,
}

impl SignalMapper {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
        }
    }

    /// Returns the namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Convert a signal name to a Zenoh key expression.
    /// e.g., signal "plan_ready" in namespace "dev-pipeline"
    ///       → "yrr/dev-pipeline/plan_ready"
    pub fn signal_to_key(&self, signal: &str) -> String {
        format!("yrr/{}/{}", self.namespace, signal)
    }

    /// Extract the signal name from a Zenoh key expression.
    /// e.g., "yrr/dev-pipeline/plan_ready" → Some("plan_ready")
    pub fn key_to_signal<'a>(&self, key: &'a str) -> Option<&'a str> {
        let prefix = format!("yrr/{}/", self.namespace);
        key.strip_prefix(&prefix)
    }

    /// Convert a queryable name to a Zenoh key expression.
    /// e.g., queryable "review" in namespace "dev-pipeline"
    ///       → "yrr/dev-pipeline/q/review"
    pub fn queryable_to_key(&self, name: &str) -> String {
        format!("yrr/{}/q/{}", self.namespace, name)
    }

    /// Extract the queryable name from a Zenoh key expression.
    /// e.g., "yrr/dev-pipeline/q/review" → Some("review")
    pub fn key_to_queryable<'a>(&self, key: &'a str) -> Option<&'a str> {
        let prefix = format!("yrr/{}/q/", self.namespace);
        key.strip_prefix(&prefix)
    }

    /// Convert an agent ID to its status key expression.
    /// e.g., agent_id "coder-abc123" in namespace "dev-pipeline"
    ///       → "yrr/dev-pipeline/_agent/coder-abc123/status"
    pub fn status_key(&self, agent_id: &str) -> String {
        format!("yrr/{}/_agent/{}/status", self.namespace, agent_id)
    }

    /// Wildcard key expression matching all agent status updates in this namespace.
    /// e.g., "yrr/dev-pipeline/_agent/*/status"
    pub fn status_wildcard(&self) -> String {
        format!("yrr/{}/_agent/*/status", self.namespace)
    }

    /// Extract the agent ID from a status key expression.
    /// e.g., "yrr/dev-pipeline/_agent/coder-abc123/status" → Some("coder-abc123")
    pub fn key_to_agent_id<'a>(&self, key: &'a str) -> Option<&'a str> {
        let prefix = format!("yrr/{}/_agent/", self.namespace);
        key.strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix("/status"))
    }

    /// Convert an agent ID to its dispatch key expression.
    /// e.g., agent_id "coder-abc123" in namespace "dev-pipeline"
    ///       → "yrr/dev-pipeline/_dispatch/coder-abc123"
    pub fn dispatch_key(&self, agent_id: &str) -> String {
        format!("yrr/{}/_dispatch/{}", self.namespace, agent_id)
    }

    /// Convert an agent name to its steer key expression.
    /// e.g., agent_name "planner" in namespace "dev-pipeline"
    ///       → "yrr/dev-pipeline/_steer/planner"
    pub fn steer_key(&self, agent_name: &str) -> String {
        format!("yrr/{}/_steer/{}", self.namespace, agent_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_to_key_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.signal_to_key("plan_ready"),
            "yrr/dev-pipeline/plan_ready"
        );
        assert_eq!(
            mapper.signal_to_key("review_passed"),
            "yrr/dev-pipeline/review_passed"
        );
    }

    #[test]
    fn key_to_signal_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.key_to_signal("yrr/dev-pipeline/plan_ready"),
            Some("plan_ready")
        );
        assert_eq!(mapper.key_to_signal("yrr/other-ns/plan_ready"), None);
    }

    #[test]
    fn queryable_to_key_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.queryable_to_key("review"),
            "yrr/dev-pipeline/q/review"
        );
    }

    #[test]
    fn key_to_queryable_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.key_to_queryable("yrr/dev-pipeline/q/review"),
            Some("review")
        );
        assert_eq!(mapper.key_to_queryable("yrr/dev-pipeline/review"), None);
        assert_eq!(mapper.key_to_queryable("yrr/other-ns/q/review"), None);
    }

    #[test]
    fn status_key_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.status_key("coder-abc123"),
            "yrr/dev-pipeline/_agent/coder-abc123/status"
        );
    }

    #[test]
    fn status_wildcard_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(mapper.status_wildcard(), "yrr/dev-pipeline/_agent/*/status");
    }

    #[test]
    fn key_to_agent_id_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.key_to_agent_id("yrr/dev-pipeline/_agent/coder-abc123/status"),
            Some("coder-abc123")
        );
        assert_eq!(
            mapper.key_to_agent_id("yrr/other-ns/_agent/coder-abc123/status"),
            None
        );
        assert_eq!(
            mapper.key_to_agent_id("yrr/dev-pipeline/_agent/coder-abc123/other"),
            None
        );
    }

    #[test]
    fn dispatch_key_mapping() {
        let mapper = SignalMapper::new("dev-pipeline");
        assert_eq!(
            mapper.dispatch_key("coder-abc123"),
            "yrr/dev-pipeline/_dispatch/coder-abc123"
        );
    }
}
