use crate::models::condition::{ComparisonOp, FieldComparison, StepCondition};
use crate::WfeError;

/// Evaluate a step condition against workflow data.
///
/// Returns `Ok(true)` if the step should run, `Ok(false)` if it should be skipped.
/// Missing field paths return `Ok(false)` (cascade skip behavior).
pub fn evaluate(
    condition: &StepCondition,
    workflow_data: &serde_json::Value,
) -> Result<bool, WfeError> {
    match evaluate_inner(condition, workflow_data) {
        Ok(result) => Ok(result),
        Err(EvalError::FieldNotPresent) => Ok(false), // cascade skip
        Err(EvalError::Wfe(e)) => Err(e),
    }
}

/// Internal error type that distinguishes missing-field from real errors.
#[derive(Debug)]
enum EvalError {
    FieldNotPresent,
    Wfe(WfeError),
}

impl From<WfeError> for EvalError {
    fn from(e: WfeError) -> Self {
        EvalError::Wfe(e)
    }
}

fn evaluate_inner(
    condition: &StepCondition,
    data: &serde_json::Value,
) -> Result<bool, EvalError> {
    match condition {
        StepCondition::All(conditions) => {
            for c in conditions {
                if !evaluate_inner(c, data)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        StepCondition::Any(conditions) => {
            for c in conditions {
                if evaluate_inner(c, data)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        StepCondition::None(conditions) => {
            for c in conditions {
                if evaluate_inner(c, data)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        StepCondition::OneOf(conditions) => {
            let mut count = 0;
            for c in conditions {
                if evaluate_inner(c, data)? {
                    count += 1;
                    if count > 1 {
                        return Ok(false);
                    }
                }
            }
            Ok(count == 1)
        }
        StepCondition::Not(inner) => {
            let result = evaluate_inner(inner, data)?;
            Ok(!result)
        }
        StepCondition::Comparison(comp) => evaluate_comparison(comp, data),
    }
}

/// Resolve a dot-separated field path against JSON data.
///
/// Path starts with `.` which is stripped, then split by `.`.
/// Segments that parse as `usize` are treated as array indices.
fn resolve_field_path<'a>(
    path: &str,
    data: &'a serde_json::Value,
) -> Result<&'a serde_json::Value, EvalError> {
    let path = path.strip_prefix('.').unwrap_or(path);
    if path.is_empty() {
        return Ok(data);
    }

    let segments: Vec<&str> = path.split('.').collect();

    // Try resolving the full path first (for nested data like {"outputs": {"x": 1}}).
    // If the first segment is "outputs"/"inputs" and doesn't exist as a key,
    // strip it and resolve flat (for workflow data where outputs merge flat).
    if segments.len() >= 2
        && (segments[0] == "outputs" || segments[0] == "inputs")
        && data.get(segments[0]).is_none()
    {
        return walk_segments(&segments[1..], data);
    }

    walk_segments(&segments, data)
}

fn walk_segments<'a>(
    segments: &[&str],
    data: &'a serde_json::Value,
) -> Result<&'a serde_json::Value, EvalError> {
    let mut current = data;

    for segment in segments {
        if let Ok(idx) = segment.parse::<usize>() {
            match current.as_array() {
                Some(arr) => {
                    current = arr.get(idx).ok_or(EvalError::FieldNotPresent)?;
                }
                None => {
                    return Err(EvalError::FieldNotPresent);
                }
            }
        } else {
            match current.as_object() {
                Some(obj) => {
                    current = obj.get(*segment).ok_or(EvalError::FieldNotPresent)?;
                }
                None => {
                    return Err(EvalError::FieldNotPresent);
                }
            }
        }
    }

    Ok(current)
}

fn evaluate_comparison(
    comp: &FieldComparison,
    data: &serde_json::Value,
) -> Result<bool, EvalError> {
    let resolved = resolve_field_path(&comp.field, data)?;

    match &comp.operator {
        ComparisonOp::IsNull => Ok(resolved.is_null()),
        ComparisonOp::IsNotNull => Ok(!resolved.is_null()),
        ComparisonOp::Equals => {
            let expected = comp.value.as_ref().ok_or_else(|| {
                EvalError::Wfe(WfeError::StepExecution(
                    "Equals operator requires a value".into(),
                ))
            })?;
            Ok(resolved == expected)
        }
        ComparisonOp::NotEquals => {
            let expected = comp.value.as_ref().ok_or_else(|| {
                EvalError::Wfe(WfeError::StepExecution(
                    "NotEquals operator requires a value".into(),
                ))
            })?;
            Ok(resolved != expected)
        }
        ComparisonOp::Gt => compare_numeric(resolved, comp, |a, b| a > b),
        ComparisonOp::Gte => compare_numeric(resolved, comp, |a, b| a >= b),
        ComparisonOp::Lt => compare_numeric(resolved, comp, |a, b| a < b),
        ComparisonOp::Lte => compare_numeric(resolved, comp, |a, b| a <= b),
        ComparisonOp::Contains => evaluate_contains(resolved, comp),
    }
}

fn compare_numeric(
    resolved: &serde_json::Value,
    comp: &FieldComparison,
    cmp_fn: fn(f64, f64) -> bool,
) -> Result<bool, EvalError> {
    let expected = comp.value.as_ref().ok_or_else(|| {
        EvalError::Wfe(WfeError::StepExecution(format!(
            "{:?} operator requires a value",
            comp.operator
        )))
    })?;

    let a = resolved.as_f64().ok_or_else(|| {
        EvalError::Wfe(WfeError::StepExecution(format!(
            "cannot compare non-numeric field value: {}",
            resolved
        )))
    })?;

    let b = expected.as_f64().ok_or_else(|| {
        EvalError::Wfe(WfeError::StepExecution(format!(
            "cannot compare with non-numeric value: {}",
            expected
        )))
    })?;

    Ok(cmp_fn(a, b))
}

fn evaluate_contains(
    resolved: &serde_json::Value,
    comp: &FieldComparison,
) -> Result<bool, EvalError> {
    let expected = comp.value.as_ref().ok_or_else(|| {
        EvalError::Wfe(WfeError::StepExecution(
            "Contains operator requires a value".into(),
        ))
    })?;

    // String contains substring.
    if let Some(s) = resolved.as_str()
        && let Some(substr) = expected.as_str()
    {
        return Ok(s.contains(substr));
    }

    // Array contains element.
    if let Some(arr) = resolved.as_array() {
        return Ok(arr.contains(expected));
    }

    Err(EvalError::Wfe(WfeError::StepExecution(format!(
        "Contains requires a string or array field, got {}",
        value_type_name(resolved)
    ))))
}

fn value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::condition::{ComparisonOp, FieldComparison, StepCondition};
    use serde_json::json;

    // -- resolve_field_path tests --

    #[test]
    fn resolve_simple_field() {
        let data = json!({"name": "alice"});
        let result = resolve_field_path(".name", &data).unwrap();
        assert_eq!(result, &json!("alice"));
    }

    #[test]
    fn resolve_nested_field() {
        let data = json!({"outputs": {"status": "success"}});
        let result = resolve_field_path(".outputs.status", &data).unwrap();
        assert_eq!(result, &json!("success"));
    }

    #[test]
    fn resolve_missing_field() {
        let data = json!({"name": "alice"});
        let result = resolve_field_path(".missing", &data);
        assert!(matches!(result, Err(EvalError::FieldNotPresent)));
    }

    #[test]
    fn resolve_array_index() {
        let data = json!({"items": [10, 20, 30]});
        let result = resolve_field_path(".items.1", &data).unwrap();
        assert_eq!(result, &json!(20));
    }

    #[test]
    fn resolve_array_index_out_of_bounds() {
        let data = json!({"items": [10, 20]});
        let result = resolve_field_path(".items.5", &data);
        assert!(matches!(result, Err(EvalError::FieldNotPresent)));
    }

    #[test]
    fn resolve_deeply_nested() {
        let data = json!({"a": {"b": {"c": {"d": 42}}}});
        let result = resolve_field_path(".a.b.c.d", &data).unwrap();
        assert_eq!(result, &json!(42));
    }

    #[test]
    fn resolve_empty_path_returns_root() {
        let data = json!({"x": 1});
        let result = resolve_field_path(".", &data).unwrap();
        assert_eq!(result, &data);
    }

    #[test]
    fn resolve_field_on_non_object() {
        let data = json!({"x": 42});
        let result = resolve_field_path(".x.y", &data);
        assert!(matches!(result, Err(EvalError::FieldNotPresent)));
    }

    // -- Comparison operator tests --

    fn comp(field: &str, op: ComparisonOp, value: Option<serde_json::Value>) -> StepCondition {
        StepCondition::Comparison(FieldComparison {
            field: field.to_string(),
            operator: op,
            value,
        })
    }

    #[test]
    fn equals_match() {
        let data = json!({"status": "ok"});
        let cond = comp(".status", ComparisonOp::Equals, Some(json!("ok")));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn equals_mismatch() {
        let data = json!({"status": "fail"});
        let cond = comp(".status", ComparisonOp::Equals, Some(json!("ok")));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn equals_numeric() {
        let data = json!({"count": 5});
        let cond = comp(".count", ComparisonOp::Equals, Some(json!(5)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn not_equals_match() {
        let data = json!({"status": "fail"});
        let cond = comp(".status", ComparisonOp::NotEquals, Some(json!("ok")));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn not_equals_mismatch() {
        let data = json!({"status": "ok"});
        let cond = comp(".status", ComparisonOp::NotEquals, Some(json!("ok")));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn gt_match() {
        let data = json!({"count": 10});
        let cond = comp(".count", ComparisonOp::Gt, Some(json!(5)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn gt_mismatch() {
        let data = json!({"count": 3});
        let cond = comp(".count", ComparisonOp::Gt, Some(json!(5)));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn gt_equal_is_false() {
        let data = json!({"count": 5});
        let cond = comp(".count", ComparisonOp::Gt, Some(json!(5)));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn gte_match() {
        let data = json!({"count": 5});
        let cond = comp(".count", ComparisonOp::Gte, Some(json!(5)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn gte_mismatch() {
        let data = json!({"count": 4});
        let cond = comp(".count", ComparisonOp::Gte, Some(json!(5)));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn lt_match() {
        let data = json!({"count": 3});
        let cond = comp(".count", ComparisonOp::Lt, Some(json!(5)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn lt_mismatch() {
        let data = json!({"count": 7});
        let cond = comp(".count", ComparisonOp::Lt, Some(json!(5)));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn lte_match() {
        let data = json!({"count": 5});
        let cond = comp(".count", ComparisonOp::Lte, Some(json!(5)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn lte_mismatch() {
        let data = json!({"count": 6});
        let cond = comp(".count", ComparisonOp::Lte, Some(json!(5)));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn contains_string_match() {
        let data = json!({"msg": "hello world"});
        let cond = comp(".msg", ComparisonOp::Contains, Some(json!("world")));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn contains_string_mismatch() {
        let data = json!({"msg": "hello world"});
        let cond = comp(".msg", ComparisonOp::Contains, Some(json!("xyz")));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn contains_array_match() {
        let data = json!({"tags": ["a", "b", "c"]});
        let cond = comp(".tags", ComparisonOp::Contains, Some(json!("b")));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn contains_array_mismatch() {
        let data = json!({"tags": ["a", "b", "c"]});
        let cond = comp(".tags", ComparisonOp::Contains, Some(json!("z")));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn is_null_true() {
        let data = json!({"val": null});
        let cond = comp(".val", ComparisonOp::IsNull, None);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn is_null_false() {
        let data = json!({"val": 42});
        let cond = comp(".val", ComparisonOp::IsNull, None);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn is_not_null_true() {
        let data = json!({"val": 42});
        let cond = comp(".val", ComparisonOp::IsNotNull, None);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn is_not_null_false() {
        let data = json!({"val": null});
        let cond = comp(".val", ComparisonOp::IsNotNull, None);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    // -- Combinator tests --

    #[test]
    fn all_both_true() {
        let data = json!({"a": 1, "b": 2});
        let cond = StepCondition::All(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn all_one_false() {
        let data = json!({"a": 1, "b": 99});
        let cond = StepCondition::All(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn all_empty_is_true() {
        let data = json!({});
        let cond = StepCondition::All(vec![]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn any_one_true() {
        let data = json!({"a": 1, "b": 99});
        let cond = StepCondition::Any(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn any_none_true() {
        let data = json!({"a": 99, "b": 99});
        let cond = StepCondition::Any(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn any_empty_is_false() {
        let data = json!({});
        let cond = StepCondition::Any(vec![]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn none_all_false() {
        let data = json!({"a": 99, "b": 99});
        let cond = StepCondition::None(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn none_one_true() {
        let data = json!({"a": 1, "b": 99});
        let cond = StepCondition::None(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn none_empty_is_true() {
        let data = json!({});
        let cond = StepCondition::None(vec![]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn one_of_exactly_one_true() {
        let data = json!({"a": 1, "b": 99});
        let cond = StepCondition::OneOf(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn one_of_both_true() {
        let data = json!({"a": 1, "b": 2});
        let cond = StepCondition::OneOf(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn one_of_none_true() {
        let data = json!({"a": 99, "b": 99});
        let cond = StepCondition::OneOf(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".b", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn not_true_becomes_false() {
        let data = json!({"a": 1});
        let cond = StepCondition::Not(Box::new(comp(
            ".a",
            ComparisonOp::Equals,
            Some(json!(1)),
        )));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn not_false_becomes_true() {
        let data = json!({"a": 99});
        let cond = StepCondition::Not(Box::new(comp(
            ".a",
            ComparisonOp::Equals,
            Some(json!(1)),
        )));
        assert!(evaluate(&cond, &data).unwrap());
    }

    // -- Cascade skip tests --

    #[test]
    fn missing_field_returns_false_cascade_skip() {
        let data = json!({"other": 1});
        let cond = comp(".missing", ComparisonOp::Equals, Some(json!(1)));
        // Missing field -> cascade skip -> Ok(false)
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn missing_nested_field_returns_false() {
        let data = json!({"a": {"b": 1}});
        let cond = comp(".a.c", ComparisonOp::Equals, Some(json!(1)));
        assert!(!evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn missing_field_in_all_returns_false() {
        let data = json!({"a": 1});
        let cond = StepCondition::All(vec![
            comp(".a", ComparisonOp::Equals, Some(json!(1))),
            comp(".missing", ComparisonOp::Equals, Some(json!(2))),
        ]);
        assert!(!evaluate(&cond, &data).unwrap());
    }

    // -- Nested combinator tests --

    #[test]
    fn nested_all_any_not() {
        let data = json!({"a": 1, "b": 2, "c": 3});
        // All(Any(a==1, a==99), Not(c==99))
        let cond = StepCondition::All(vec![
            StepCondition::Any(vec![
                comp(".a", ComparisonOp::Equals, Some(json!(1))),
                comp(".a", ComparisonOp::Equals, Some(json!(99))),
            ]),
            StepCondition::Not(Box::new(comp(
                ".c",
                ComparisonOp::Equals,
                Some(json!(99)),
            ))),
        ]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn nested_any_of_alls() {
        let data = json!({"x": 10, "y": 20});
        // Any(All(x>5, y>25), All(x>5, y>15))
        let cond = StepCondition::Any(vec![
            StepCondition::All(vec![
                comp(".x", ComparisonOp::Gt, Some(json!(5))),
                comp(".y", ComparisonOp::Gt, Some(json!(25))),
            ]),
            StepCondition::All(vec![
                comp(".x", ComparisonOp::Gt, Some(json!(5))),
                comp(".y", ComparisonOp::Gt, Some(json!(15))),
            ]),
        ]);
        assert!(evaluate(&cond, &data).unwrap());
    }

    // -- Edge cases / error cases --

    #[test]
    fn gt_on_string_errors() {
        let data = json!({"name": "alice"});
        let cond = comp(".name", ComparisonOp::Gt, Some(json!(5)));
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn gt_with_string_value_errors() {
        let data = json!({"count": 5});
        let cond = comp(".count", ComparisonOp::Gt, Some(json!("not a number")));
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn contains_on_number_errors() {
        let data = json!({"count": 42});
        let cond = comp(".count", ComparisonOp::Contains, Some(json!("4")));
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn equals_without_value_errors() {
        let data = json!({"a": 1});
        let cond = comp(".a", ComparisonOp::Equals, None);
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn not_equals_without_value_errors() {
        let data = json!({"a": 1});
        let cond = comp(".a", ComparisonOp::NotEquals, None);
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn gt_without_value_errors() {
        let data = json!({"a": 1});
        let cond = comp(".a", ComparisonOp::Gt, None);
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn contains_without_value_errors() {
        let data = json!({"msg": "hello"});
        let cond = comp(".msg", ComparisonOp::Contains, None);
        let result = evaluate(&cond, &data);
        assert!(result.is_err());
    }

    #[test]
    fn equals_bool_values() {
        let data = json!({"active": true});
        let cond = comp(".active", ComparisonOp::Equals, Some(json!(true)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn equals_null_value() {
        let data = json!({"val": null});
        let cond = comp(".val", ComparisonOp::Equals, Some(json!(null)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn float_comparison() {
        let data = json!({"score": 3.14});
        assert!(evaluate(&comp(".score", ComparisonOp::Gt, Some(json!(3.0))), &data).unwrap());
        assert!(evaluate(&comp(".score", ComparisonOp::Lt, Some(json!(4.0))), &data).unwrap());
        assert!(!evaluate(&comp(".score", ComparisonOp::Equals, Some(json!(3.0))), &data).unwrap());
    }

    #[test]
    fn contains_array_numeric_element() {
        let data = json!({"nums": [1, 2, 3]});
        let cond = comp(".nums", ComparisonOp::Contains, Some(json!(2)));
        assert!(evaluate(&cond, &data).unwrap());
    }

    #[test]
    fn one_of_empty_is_false() {
        let data = json!({});
        let cond = StepCondition::OneOf(vec![]);
        assert!(!evaluate(&cond, &data).unwrap());
    }
}
