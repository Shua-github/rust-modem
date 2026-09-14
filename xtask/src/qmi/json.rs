//! Loader for the libqmi JSON service database.
//!
//! The files are JSON with `//` line comments, which `serde_json` does not
//! accept, so comments are stripped before parsing. A few objects repeat a key
//! (`format`, `personal-info`); parsing through [`serde_json::Value`] keeps the
//! last value like libqmi's own Python loader does. Every entry of every file
//! is parsed into the same [`Raw`] shape; the `type` field tells them apart.

use std::path::Path;

use serde_json::Value;

/// One entry of a `qmi-*.json` file.
#[derive(Debug, Clone)]
pub struct Raw {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub id: Option<String>,
    pub format: Option<String>,
    pub contents: Option<Vec<Raw>>,
    pub input: Option<Vec<Raw>>,
    pub output: Option<Vec<Raw>>,
    pub common_ref: Option<String>,
    pub array_element: Option<Box<Raw>>,
    pub size_prefix_format: Option<String>,
    pub sequence_prefix_format: Option<String>,
    pub fixed_size: Option<String>,
    pub max_size: Option<String>,
    pub public_format: Option<String>,
    pub guint_size: Option<String>,
    pub endian: Option<String>,
    pub mandatory: Option<String>,
    pub prerequisites: Option<Prerequisites>,
    // Prerequisite-only keys.
    pub field: Option<String>,
    pub operation: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Prerequisites {
    List(Vec<Prerequisite>),
    /// Input-side ordering hints such as `"0x01-first"` are never evaluated.
    #[allow(dead_code)]
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Prerequisite {
    pub field: Option<String>,
    pub operation: Option<String>,
    pub value: Option<String>,
    pub common_ref: Option<String>,
}

impl Raw {
    fn from_value(value: &Value) -> Self {
        let string = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);

        let list = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|items| items.iter().map(Raw::from_value).collect())
        };

        let prerequisites = value.get("prerequisites").map(|prereq| match prereq {
            Value::Array(items) => Prerequisites::List(
                items
                    .iter()
                    .map(|item| Prerequisite {
                        field: item
                            .get("field")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        operation: item
                            .get("operation")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        value: item
                            .get("value")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        common_ref: item
                            .get("common-ref")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                    .collect(),
            ),
            Value::String(text) => Prerequisites::Text(text.clone()),
            other => panic!("unexpected prerequisites {other}"),
        });

        Self {
            kind: string("type"),
            name: string("name"),
            id: string("id"),
            format: string("format"),
            contents: list("contents"),
            input: list("input"),
            output: list("output"),
            common_ref: string("common-ref"),
            array_element: value
                .get("array-element")
                .map(|item| Box::new(Raw::from_value(item))),
            size_prefix_format: string("size-prefix-format"),
            sequence_prefix_format: string("sequence-prefix-format"),
            fixed_size: string("fixed-size"),
            max_size: string("max-size"),
            public_format: string("public-format"),
            guint_size: string("guint-size"),
            endian: string("endian"),
            mandatory: string("mandatory"),
            prerequisites,
            field: string("field"),
            operation: string("operation"),
            value: string("value"),
        }
    }

    /// The prerequisites that take part in decoding, if any.
    pub fn prereq_list(&self) -> &[Prerequisite] {
        match &self.prerequisites {
            Some(Prerequisites::List(list)) => list,
            _ => &[],
        }
    }

    pub fn id_value(&self) -> Option<u32> {
        self.id.as_deref().map(parse_int)
    }

    pub fn fixed_size_value(&self) -> Option<usize> {
        self.fixed_size
            .as_deref()
            .map(parse_int)
            .map(|v| v as usize)
    }

    pub fn max_size_value(&self) -> usize {
        self.max_size.as_deref().map(parse_int).unwrap_or(0) as usize
    }
}

/// Parse `"0x01"` or `"15"` into an integer.
pub fn parse_int(text: &str) -> u32 {
    let text = text.trim();

    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => text.parse(),
    }
    .unwrap_or_else(|_| panic!("cannot parse integer {text:?}"))
}

/// Load one JSON file, stripping `//` comment lines.
pub fn load(path: &Path) -> Vec<Raw> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let stripped: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .map(|line| format!("{line}\n"))
        .collect();

    let value: Value = serde_json::from_str(&stripped)
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));

    value
        .as_array()
        .unwrap_or_else(|| panic!("{} is not a JSON array", path.display()))
        .iter()
        .map(Raw::from_value)
        .collect()
}
