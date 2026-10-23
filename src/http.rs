use std::sync::OnceLock;

static AGENT: OnceLock<ureq::Agent> = OnceLock::new();

/// Shared HTTP agent so TLS connections are reused (keep-alive).
pub fn agent() -> &'static ureq::Agent {
    AGENT.get_or_init(|| ureq::AgentBuilder::new().build())
}



/// Removes credential-bearing query values from error/log text.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        let key = ["api_access_key=", "api_key=", "api_app_key=", "token=", "key="]
            .iter()
            .find(|k| rest.starts_with(**k));
        if let Some(k) = key {
            out.push_str(&text[copied..i]);
            out.push_str(k);
            out.push_str("***");
            let mut j = i + k.len();
            while j < bytes.len() {
                let c = bytes[j] as char;
                if c == '&' || c == ' ' || c == '"' || c == '\'' || c == ')' {
                    break;
                }
                j += 1;
            }
            i = j;
            copied = j;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

