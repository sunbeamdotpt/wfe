use std::collections::HashMap;

/// Parse `##wfe[output key=value]` lines from stdout.
///
/// This is the same output protocol used by wfe-containerd and wfe-yaml shell steps.
pub fn parse_outputs(stdout: &str) -> HashMap<String, String> {
    let mut outputs = HashMap::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(inner) = trimmed
            .strip_prefix("##wfe[output ")
            .and_then(|s| s.strip_suffix(']'))
        {
            if let Some(eq_pos) = inner.find('=') {
                let key = inner[..eq_pos].trim().to_string();
                let value = inner[eq_pos + 1..].to_string();
                if !key.is_empty() {
                    outputs.insert(key, value);
                }
            }
        }
    }
    outputs
}

/// Build the output_data JSON object from step execution results.
///
/// Includes parsed `##wfe[output]` values plus raw stdout/stderr/exit_code
/// prefixed with the step name.
pub fn build_output_data(
    step_name: &str,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    parsed: &HashMap<String, String>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Add parsed outputs.
    for (key, value) in parsed {
        // Try to parse as JSON value, fall back to string.
        let json_val = serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.clone()));
        map.insert(key.clone(), json_val);
    }

    // Add raw stdout/stderr/exit_code with step name prefix.
    map.insert(
        format!("{step_name}.stdout"),
        serde_json::Value::String(stdout.to_string()),
    );
    map.insert(
        format!("{step_name}.stderr"),
        serde_json::Value::String(stderr.to_string()),
    );
    map.insert(
        format!("{step_name}.exit_code"),
        serde_json::Value::Number(exit_code.into()),
    );

    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_empty_stdout() {
        assert!(parse_outputs("").is_empty());
    }

    #[test]
    fn parse_no_output_markers() {
        let stdout = "hello world\nsome other output\n";
        assert!(parse_outputs(stdout).is_empty());
    }

    #[test]
    fn parse_single_output() {
        let stdout = "building...\n##wfe[output version=1.2.3]\ndone\n";
        let outputs = parse_outputs(stdout);
        assert_eq!(outputs.get("version"), Some(&"1.2.3".to_string()));
    }

    #[test]
    fn parse_multiple_outputs() {
        let stdout = "##wfe[output digest=sha256:abc]\n##wfe[output tag=latest]\n";
        let outputs = parse_outputs(stdout);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs.get("digest"), Some(&"sha256:abc".to_string()));
        assert_eq!(outputs.get("tag"), Some(&"latest".to_string()));
    }

    #[test]
    fn parse_value_with_equals() {
        let stdout = "##wfe[output url=https://example.com?a=1&b=2]\n";
        let outputs = parse_outputs(stdout);
        assert_eq!(
            outputs.get("url"),
            Some(&"https://example.com?a=1&b=2".to_string())
        );
    }

    #[test]
    fn parse_duplicate_key_last_wins() {
        let stdout = "##wfe[output key=first]\n##wfe[output key=second]\n";
        let outputs = parse_outputs(stdout);
        assert_eq!(outputs.get("key"), Some(&"second".to_string()));
    }

    #[test]
    fn parse_whitespace_in_key() {
        let stdout = "##wfe[output  key = value]\n";
        let outputs = parse_outputs(stdout);
        assert_eq!(outputs.get("key"), Some(&" value".to_string()));
    }

    #[test]
    fn parse_empty_value() {
        let stdout = "##wfe[output key=]\n";
        let outputs = parse_outputs(stdout);
        assert_eq!(outputs.get("key"), Some(&"".to_string()));
    }

    #[test]
    fn parse_ignores_malformed() {
        let stdout = "##wfe[output ]\n##wfe[output no_equals]\n##wfe[output =no_key]\n";
        let outputs = parse_outputs(stdout);
        // "=no_key" has empty key, ignored. "no_equals" has no =, ignored. " " has no =, ignored.
        assert!(outputs.is_empty());
    }

    #[test]
    fn build_output_data_with_parsed() {
        let parsed: HashMap<String, String> =
            [("version".into(), "1.0.0".into())].into_iter().collect();
        let data = build_output_data("test", "hello\n", "warn\n", 0, &parsed);
        assert_eq!(data["version"], "1.0.0");
        assert_eq!(data["test.stdout"], "hello\n");
        assert_eq!(data["test.stderr"], "warn\n");
        assert_eq!(data["test.exit_code"], 0);
    }

    #[test]
    fn build_output_data_empty() {
        let data = build_output_data("step", "", "", 0, &HashMap::new());
        assert_eq!(data["step.stdout"], "");
        assert_eq!(data["step.exit_code"], 0);
    }

    #[test]
    fn build_output_data_json_value() {
        let parsed: HashMap<String, String> = [
            ("count".into(), "42".into()),
            ("flag".into(), "true".into()),
        ]
        .into_iter()
        .collect();
        let data = build_output_data("s", "", "", 0, &parsed);
        // Numbers and booleans should be parsed as JSON, not strings.
        assert_eq!(data["count"], 42);
        assert_eq!(data["flag"], true);
    }
}
