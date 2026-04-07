//! Output formatting helpers (table + JSON).

use clap::ValueEnum;
use comfy_table::{Cell, ContentArrangement, Table, presets::UTF8_FULL};
use serde::Serialize;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    Table,
    Json,
}

impl Default for OutputFormat {
    fn default() -> Self {
        OutputFormat::Table
    }
}

/// Render a list of rows as a table or JSON depending on `format`.
pub fn render_list<T: Serialize>(
    format: OutputFormat,
    headers: &[&str],
    rows: &[Vec<String>],
    items: &[T],
) -> String {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(items).unwrap_or_else(|e| format!("error: {e}"))
        }
        OutputFormat::Table => render_table(headers, rows),
    }
}

/// Render a single object as a key-value table or JSON.
pub fn render_object<T: Serialize>(
    format: OutputFormat,
    fields: &[(&str, String)],
    item: &T,
) -> String {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(item).unwrap_or_else(|e| format!("error: {e}"))
        }
        OutputFormat::Table => render_kv(fields),
    }
}

/// Build a comfy-table from headers + rows.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(
            headers
                .iter()
                .map(|h| Cell::new(h).fg(comfy_table::Color::Cyan)),
        );
    for row in rows {
        table.add_row(row.iter().map(Cell::new).collect::<Vec<_>>());
    }
    table.to_string()
}

/// Build a key-value layout (one row per field).
pub fn render_kv(fields: &[(&str, String)]) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    for (key, value) in fields {
        table.add_row(vec![
            Cell::new(key).fg(comfy_table::Color::Cyan),
            Cell::new(value),
        ]);
    }
    table.to_string()
}

/// Format a chrono UTC timestamp as a short ISO 8601 string.
pub fn fmt_time(ts: &chrono::DateTime<chrono::Utc>) -> String {
    ts.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Format a prost Timestamp.
pub fn fmt_proto_time(ts: &prost_types::Timestamp) -> String {
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32);
    match dt {
        Some(dt) => fmt_time(&dt),
        None => "<invalid>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn output_format_default_is_table() {
        assert_eq!(OutputFormat::default(), OutputFormat::Table);
    }

    #[test]
    fn render_table_includes_headers() {
        let out = render_table(&["a", "b"], &[vec!["1".into(), "2".into()]]);
        assert!(out.contains("a"));
        assert!(out.contains("b"));
        assert!(out.contains("1"));
        assert!(out.contains("2"));
    }

    #[test]
    fn render_table_empty_rows() {
        let out = render_table(&["a", "b"], &[]);
        assert!(out.contains("a"));
    }

    #[test]
    fn render_kv_layout() {
        let out = render_kv(&[("Name", "Foo".into()), ("Status", "OK".into())]);
        assert!(out.contains("Name"));
        assert!(out.contains("Foo"));
        assert!(out.contains("Status"));
        assert!(out.contains("OK"));
    }

    #[test]
    fn render_list_json_serializes() {
        let items = vec![json!({"a": 1}), json!({"a": 2})];
        let out = render_list(OutputFormat::Json, &[], &[], &items);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["a"], 1);
    }

    #[test]
    fn render_list_table_uses_rows() {
        let items: Vec<serde_json::Value> = vec![];
        let out = render_list(OutputFormat::Table, &["x"], &[vec!["row1".into()]], &items);
        assert!(out.contains("row1"));
    }

    #[test]
    fn render_object_json() {
        let item = json!({"k": "v"});
        let out = render_object(OutputFormat::Json, &[], &item);
        assert!(out.contains("\"k\""));
        assert!(out.contains("\"v\""));
    }

    #[test]
    fn fmt_time_iso_format() {
        let ts: chrono::DateTime<chrono::Utc> = "2026-04-07T12:34:56Z".parse().unwrap();
        assert_eq!(fmt_time(&ts), "2026-04-07 12:34:56");
    }

    #[test]
    fn fmt_proto_time_valid() {
        let ts = prost_types::Timestamp {
            seconds: 1_700_000_000,
            nanos: 0,
        };
        let out = fmt_proto_time(&ts);
        assert!(out.starts_with("2023"));
    }

    #[test]
    fn fmt_proto_time_invalid() {
        let ts = prost_types::Timestamp {
            seconds: i64::MAX,
            nanos: 0,
        };
        let out = fmt_proto_time(&ts);
        assert_eq!(out, "<invalid>");
    }
}
