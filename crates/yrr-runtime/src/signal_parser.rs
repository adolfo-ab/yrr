use yrr_core::message::{EmittedQuery, EmittedSignal};

/// Parse `<<SIGNAL:name>> payload` markers from agent output.
pub fn parse_signals(output: &str) -> Vec<EmittedSignal> {
    parse_markers(output, "<<SIGNAL:", |name, payload| EmittedSignal {
        signal: name,
        payload,
    })
}

/// Parse `<<QUERY:key>> payload` markers from agent output.
pub fn parse_queries(output: &str) -> Vec<EmittedQuery> {
    parse_markers(output, "<<QUERY:", |key, payload| EmittedQuery {
        key,
        payload,
    })
}

/// Generic marker parser. Finds `{prefix}name>> payload` patterns in output.
fn parse_markers<T>(
    output: &str,
    prefix: &str,
    build: fn(String, String) -> T,
) -> Vec<T> {
    let mut results = Vec::new();
    let marker_end = ">>";

    let mut remaining = output;

    while let Some(start) = remaining.find(prefix) {
        let after_marker = &remaining[start + prefix.len()..];

        // Find the closing >> but ensure it comes before the next marker of the same type.
        let next_marker_pos = after_marker.find(prefix);
        let close_pos = after_marker.find(marker_end);

        let Some(end) = close_pos else {
            // No closing >> at all. Skip rest.
            break;
        };

        // If the >> comes after the next marker, this marker is malformed.
        if let Some(next) = next_marker_pos {
            if end > next {
                // Malformed — skip past this marker and try the next one.
                remaining = &after_marker[next..];
                continue;
            }
        }

        let name = after_marker[..end].trim();
        let after_close = &after_marker[end + marker_end.len()..];

        // Payload is everything after >> until the next marker of any type or end.
        let next_signal = after_close.find("<<SIGNAL:");
        let next_query = after_close.find("<<QUERY:");
        let payload_end = match (next_signal, next_query) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => after_close.len(),
        };
        let payload = after_close[..payload_end].trim();

        if !name.is_empty() {
            results.push(build(name.to_string(), payload.to_string()));
        }

        remaining = &after_close[payload_end..];
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Signal tests ───────────────────────────────────────────────────────

    #[test]
    fn parse_single_signal() {
        let output = "Some text\n<<SIGNAL:plan_ready>> Plan written to PLAN.md";
        let signals = parse_signals(output);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal, "plan_ready");
        assert_eq!(signals[0].payload, "Plan written to PLAN.md");
    }

    #[test]
    fn parse_multiple_signals() {
        let output = r#"Working on it...
<<SIGNAL:progress>> Finished auth module
Still working...
<<SIGNAL:code_ready>> Done. See commit abc123, files: src/auth.rs"#;

        let signals = parse_signals(output);
        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0].signal, "progress");
        assert_eq!(signals[0].payload, "Finished auth module\nStill working...");
        assert_eq!(signals[1].signal, "code_ready");
        assert!(signals[1].payload.contains("abc123"));
    }

    #[test]
    fn parse_no_signals() {
        let output = "Just some regular output with no markers.";
        let signals = parse_signals(output);
        assert!(signals.is_empty());
    }

    #[test]
    fn parse_signal_with_empty_payload() {
        let output = "<<SIGNAL:done>>";
        let signals = parse_signals(output);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal, "done");
        assert_eq!(signals[0].payload, "");
    }

    #[test]
    fn parse_signal_at_start() {
        let output = "<<SIGNAL:plan_ready>> Here is the plan";
        let signals = parse_signals(output);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal, "plan_ready");
    }

    #[test]
    fn malformed_marker_is_skipped() {
        let output = "<<SIGNAL:oops no closing bracket\n<<SIGNAL:ok>> payload";
        let signals = parse_signals(output);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal, "ok");
    }

    // ─── Query tests ────────────────────────────────────────────────────────

    #[test]
    fn parse_single_query() {
        let output = "I need a review.\n<<QUERY:review>> Please review src/auth.rs";
        let queries = parse_queries(output);
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].key, "review");
        assert_eq!(queries[0].payload, "Please review src/auth.rs");
    }

    #[test]
    fn parse_multiple_queries() {
        let output = "<<QUERY:review>> Review auth.rs\n<<QUERY:lint>> Check style";
        let queries = parse_queries(output);
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0].key, "review");
        assert_eq!(queries[1].key, "lint");
    }

    #[test]
    fn parse_no_queries() {
        let output = "Just a signal: <<SIGNAL:done>> finished";
        let queries = parse_queries(output);
        assert!(queries.is_empty());
    }

    #[test]
    fn parse_query_with_empty_payload() {
        let output = "<<QUERY:status>>";
        let queries = parse_queries(output);
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].key, "status");
        assert_eq!(queries[0].payload, "");
    }

    // ─── Mixed signals and queries ──────────────────────────────────────────

    #[test]
    fn mixed_signals_and_queries() {
        let output = "Working...\n<<QUERY:review>> Check my code\nMore text\n<<SIGNAL:progress>> halfway done";
        let signals = parse_signals(output);
        let queries = parse_queries(output);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal, "progress");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].key, "review");
        // The query payload ends at the next marker (<<SIGNAL:)
        assert_eq!(queries[0].payload, "Check my code\nMore text");
    }
}
