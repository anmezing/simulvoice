use std::sync::OnceLock;

static AGENT: OnceLock<ureq::Agent> = OnceLock::new();

/// Shared HTTP agent so TLS connections are reused (keep-alive).
pub fn agent() -> &'static ureq::Agent {
    AGENT.get_or_init(|| ureq::AgentBuilder::new().build())
}


