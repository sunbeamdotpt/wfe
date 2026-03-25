use std::collections::HashMap;

use regex::Regex;

use crate::error::YamlWorkflowError;

/// Resolve `((var.path))` expressions in a YAML string against a config map.
///
/// Dot-path traversal: `((config.database.host))` resolves by walking
/// `config["config"]["database"]["host"]`.
pub fn interpolate(
    yaml: &str,
    config: &HashMap<String, serde_json::Value>,
) -> Result<String, YamlWorkflowError> {
    let re = Regex::new(r"\(\(([a-zA-Z0-9_.]+)\)\)").expect("valid regex");

    let mut result = String::with_capacity(yaml.len());
    let mut last_end = 0;

    for cap in re.captures_iter(yaml) {
        let m = cap.get(0).unwrap();
        let var_path = &cap[1];

        // Resolve the variable path.
        let value = resolve_path(var_path, config)?;

        result.push_str(&yaml[last_end..m.start()]);
        result.push_str(&value);
        last_end = m.end();
    }

    result.push_str(&yaml[last_end..]);
    Ok(result)
}

fn resolve_path(
    path: &str,
    config: &HashMap<String, serde_json::Value>,
) -> Result<String, YamlWorkflowError> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return Err(YamlWorkflowError::UnresolvedVariable(path.to_string()));
    }

    // The first segment is the top-level key in the config map.
    let root = config
        .get(parts[0])
        .ok_or_else(|| YamlWorkflowError::UnresolvedVariable(path.to_string()))?;

    // Walk remaining segments.
    let mut current = root;
    for &segment in &parts[1..] {
        current = current
            .get(segment)
            .ok_or_else(|| YamlWorkflowError::UnresolvedVariable(path.to_string()))?;
    }

    // Convert the final value to a string.
    match current {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Null => Ok("null".to_string()),
        other => Ok(other.to_string()),
    }
}
