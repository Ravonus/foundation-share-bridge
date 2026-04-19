//! Pure data-shape helpers: small functions for peeking into JSON values and
//! for collection-level reductions (dedup / max-by / first-present).

use std::collections::HashSet;

use chrono::{DateTime, Utc};

pub fn json_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|entry| entry.as_str())
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
}

pub fn nested_json_value<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path {
        current = current.as_object()?.get(*segment)?;
    }
    Some(current)
}

pub fn json_display_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
        serde_json::Value::Array(values) => {
            let joined = values
                .iter()
                .filter_map(|entry| json_display_value(Some(entry)))
                .collect::<Vec<_>>();
            (!joined.is_empty()).then(|| joined.join(", "))
        }
        serde_json::Value::Object(record) => {
            serde_json::to_string(record).ok().filter(|value| !value.is_empty())
        }
    }
}

pub fn first_present_string<I>(values: I) -> Option<String>
where
    I: IntoIterator<Item = Option<String>>,
{
    values.into_iter().flatten().find(|value| !value.trim().is_empty())
}

pub fn max_timestamp_by<T, F>(members: &[T], accessor: F) -> Option<DateTime<Utc>>
where
    F: Fn(&T) -> Option<DateTime<Utc>>,
{
    members.iter().filter_map(accessor).max()
}

pub fn first_present_error<T, F>(members: &[T], accessor: F) -> Option<String>
where
    F: Fn(&T) -> Option<&String>,
{
    members.iter().filter_map(accessor).find(|value| !value.trim().is_empty()).cloned()
}

pub fn unique_trimmed_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}
