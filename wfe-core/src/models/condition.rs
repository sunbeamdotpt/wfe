use serde::{Deserialize, Serialize};

/// A condition that determines whether a workflow step should execute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StepCondition {
    /// All sub-conditions must be true (AND).
    All(Vec<StepCondition>),
    /// At least one sub-condition must be true (OR).
    Any(Vec<StepCondition>),
    /// No sub-conditions may be true (NOR).
    None(Vec<StepCondition>),
    /// Exactly one sub-condition must be true (XOR).
    OneOf(Vec<StepCondition>),
    /// Negation of a single condition (NOT).
    Not(Box<StepCondition>),
    /// A leaf comparison against a field in workflow data.
    Comparison(FieldComparison),
}

/// A comparison of a workflow data field against an expected value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldComparison {
    /// Dot-separated field path, e.g. ".outputs.docker_started".
    pub field: String,
    /// The comparison operator.
    pub operator: ComparisonOp,
    /// The value to compare against. Required for all operators except IsNull/IsNotNull.
    pub value: Option<serde_json::Value>,
}

/// Comparison operators for field conditions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComparisonOp {
    Equals,
    NotEquals,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    IsNull,
    IsNotNull,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn comparison_op_serde_round_trip() {
        for op in [
            ComparisonOp::Equals,
            ComparisonOp::NotEquals,
            ComparisonOp::Gt,
            ComparisonOp::Gte,
            ComparisonOp::Lt,
            ComparisonOp::Lte,
            ComparisonOp::Contains,
            ComparisonOp::IsNull,
            ComparisonOp::IsNotNull,
        ] {
            let json_str = serde_json::to_string(&op).unwrap();
            let deserialized: ComparisonOp = serde_json::from_str(&json_str).unwrap();
            assert_eq!(op, deserialized);
        }
    }

    #[test]
    fn field_comparison_serde_round_trip() {
        let comp = FieldComparison {
            field: ".outputs.status".to_string(),
            operator: ComparisonOp::Equals,
            value: Some(json!("success")),
        };
        let json_str = serde_json::to_string(&comp).unwrap();
        let deserialized: FieldComparison = serde_json::from_str(&json_str).unwrap();
        assert_eq!(comp, deserialized);
    }

    #[test]
    fn field_comparison_without_value_serde_round_trip() {
        let comp = FieldComparison {
            field: ".outputs.result".to_string(),
            operator: ComparisonOp::IsNull,
            value: None,
        };
        let json_str = serde_json::to_string(&comp).unwrap();
        let deserialized: FieldComparison = serde_json::from_str(&json_str).unwrap();
        assert_eq!(comp, deserialized);
    }

    #[test]
    fn step_condition_comparison_serde_round_trip() {
        let condition = StepCondition::Comparison(FieldComparison {
            field: ".count".to_string(),
            operator: ComparisonOp::Gt,
            value: Some(json!(5)),
        });
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn step_condition_not_serde_round_trip() {
        let condition = StepCondition::Not(Box::new(StepCondition::Comparison(FieldComparison {
            field: ".active".to_string(),
            operator: ComparisonOp::Equals,
            value: Some(json!(false)),
        })));
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn step_condition_all_serde_round_trip() {
        let condition = StepCondition::All(vec![
            StepCondition::Comparison(FieldComparison {
                field: ".a".to_string(),
                operator: ComparisonOp::Equals,
                value: Some(json!(1)),
            }),
            StepCondition::Comparison(FieldComparison {
                field: ".b".to_string(),
                operator: ComparisonOp::Equals,
                value: Some(json!(2)),
            }),
        ]);
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn step_condition_any_serde_round_trip() {
        let condition = StepCondition::Any(vec![StepCondition::Comparison(FieldComparison {
            field: ".x".to_string(),
            operator: ComparisonOp::IsNull,
            value: None,
        })]);
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn step_condition_none_serde_round_trip() {
        let condition = StepCondition::None(vec![StepCondition::Comparison(FieldComparison {
            field: ".err".to_string(),
            operator: ComparisonOp::IsNotNull,
            value: None,
        })]);
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn step_condition_one_of_serde_round_trip() {
        let condition = StepCondition::OneOf(vec![
            StepCondition::Comparison(FieldComparison {
                field: ".mode".to_string(),
                operator: ComparisonOp::Equals,
                value: Some(json!("fast")),
            }),
            StepCondition::Comparison(FieldComparison {
                field: ".mode".to_string(),
                operator: ComparisonOp::Equals,
                value: Some(json!("slow")),
            }),
        ]);
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }

    #[test]
    fn nested_combinator_serde_round_trip() {
        let condition = StepCondition::All(vec![
            StepCondition::Any(vec![
                StepCondition::Comparison(FieldComparison {
                    field: ".a".to_string(),
                    operator: ComparisonOp::Equals,
                    value: Some(json!(1)),
                }),
                StepCondition::Comparison(FieldComparison {
                    field: ".b".to_string(),
                    operator: ComparisonOp::Equals,
                    value: Some(json!(2)),
                }),
            ]),
            StepCondition::Not(Box::new(StepCondition::Comparison(FieldComparison {
                field: ".c".to_string(),
                operator: ComparisonOp::IsNull,
                value: None,
            }))),
        ]);
        let json_str = serde_json::to_string(&condition).unwrap();
        let deserialized: StepCondition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(condition, deserialized);
    }
}
