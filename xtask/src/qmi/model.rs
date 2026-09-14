//! Resolves the JSON database into a typed model ready for emission.

use std::collections::HashMap;
use std::path::Path;

use crate::qmi::json::{self, Raw};
use crate::qmi::naming;

/// Service name in the database and QMI service id.
const SERVICES: &[(&str, u16)] = &[
    ("CTL", 0x00),
    ("WDS", 0x01),
    ("DMS", 0x02),
    ("NAS", 0x03),
    ("QOS", 0x04),
    ("WMS", 0x05),
    ("PDS", 0x06),
    ("VOICE", 0x09),
    ("UIM", 0x0b),
    ("PBM", 0x0c),
    ("LOC", 0x10),
    ("SAR", 0x11),
    ("IMS", 0x12),
    ("WDA", 0x1a),
    ("IMSP", 0x1f),
    ("IMSA", 0x21),
    ("PDC", 0x24),
    ("DSD", 0x2a),
    ("DPM", 0x2f),
    ("OMA", 0xe2),
    ("FOX", 0xe3),
    ("GMS", 0xe7),
    ("GAS", 0xe8),
    ("ATR", 0xed),
    ("SSC", 0x190),
    ("IMSDCM", 0x302),
];

/// Byte order of a field, defaulting to little-endian like libqmi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E {
    Little,
    Big,
}

/// Database name of the mandatory `Result` TLV of every response.
const RESULT_REF: &str = "Operation Result";

/// Rust type of a generated field.
#[derive(Debug, Clone)]
pub enum RType {
    U8,
    I8,
    U16(E),
    I16(E),
    U32(E),
    I32(E),
    U64(E),
    I64(E),
    F32(E),
    F64(E),
    Bool {
        signed: bool,
    },
    SizedUint {
        bytes: usize,
        e: E,
    },
    Str {
        prefix: u8,
        max: usize,
        fixed: Option<usize>,
    },
    Composite(String),
    Array {
        elem: Box<RType>,
        prefix: Option<u8>,
        fixed: Option<usize>,
    },
}

#[derive(Debug, Clone)]
pub struct RMember {
    pub name: String,
    pub rust_name: String,
    pub ty: RType,
}

#[derive(Debug, Clone)]
pub enum CompositeKind {
    Struct(Vec<RMember>),
    /// An array carrying both a size prefix and a sequence prefix, as used by
    /// the DMS PRL segment.
    SeqArray {
        count_prefix: u8,
        elem: Box<RType>,
    },
}

#[derive(Debug, Clone)]
pub struct Composite {
    pub name: String,
    pub kind: CompositeKind,
}

/// A top-level TLV of a request or response container.
#[derive(Debug, Clone)]
pub struct RField {
    pub name: String,
    pub rust_name: String,
    pub tlv_id: u8,
    pub const_name: String,
    /// Whether the field is optional, prerequisites included.
    pub optional: bool,
    /// Whether the database itself marks the TLV mandatory, ignoring the
    /// prerequisites.
    pub mandatory: bool,
    /// Whether the field is the `Result` TLV every response carries.
    pub is_result: bool,
    pub ty: RType,
    pub guards: Vec<Guard>,
}

/// A prerequisite condition that gates decoding of a field.
#[derive(Debug, Clone)]
pub struct Guard {
    pub access: Access,
    pub op: &'static str,
    pub value: u64,
}

#[derive(Debug, Clone)]
pub struct Access {
    pub levels: Vec<AccessLevel>,
}

#[derive(Debug, Clone)]
pub struct AccessLevel {
    pub rust_name: String,
    pub optional: bool,
}

pub struct Service {
    pub id: u16,
    pub messages: Vec<MessageDef>,
    pub indications: Vec<MessageDef>,
}

pub struct MessageDef {
    pub name: String,
    pub module: String,
    pub id: u16,
    pub input: Vec<RField>,
    pub output: Vec<RField>,
    pub types: Vec<Composite>,
}

pub struct Database {
    common_tlvs: HashMap<String, Raw>,
    common_prereqs: HashMap<String, json::Prerequisite>,
}

impl Database {
    /// Collect the common TLVs and prerequisites of every file in `dir`; a
    /// service may reference a common object defined by any other service.
    pub fn load(dir: &Path) -> Self {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".json"))
            })
            .collect();
        files.sort();

        let mut common_tlvs = HashMap::new();
        let mut common_prereqs = HashMap::new();

        for file in files {
            for entry in json::load(&file) {
                let Some(reference) = entry.common_ref.clone() else {
                    continue;
                };

                match entry.kind.as_deref() {
                    Some("TLV") => {
                        common_tlvs.insert(reference, entry.clone());
                    }
                    Some("prerequisite") => {
                        common_prereqs.insert(
                            reference,
                            json::Prerequisite {
                                field: entry.field.clone(),
                                operation: entry.operation.clone(),
                                value: entry.value.clone(),
                                common_ref: None,
                            },
                        );
                    }
                    _ => {}
                }
            }
        }

        Self {
            common_tlvs,
            common_prereqs,
        }
    }

    /// Build one service from its JSON file.
    pub fn service(&self, file: &Path) -> Service {
        let entries = json::load(file);

        let Some((_, id)) = entries
            .iter()
            .find(|e| e.kind.as_deref() == Some("Service"))
            .and_then(|e| {
                SERVICES
                    .iter()
                    .find(|(name, _)| Some(*name) == e.name.as_deref())
            })
        else {
            panic!("unknown service in {}", file.display());
        };

        let mut messages = Vec::new();
        let mut indications = Vec::new();

        for entry in entries.iter() {
            match entry.kind.as_deref() {
                Some("Message") => messages.push(self.message(entry, false)),
                Some("Indication") => indications.push(self.message(entry, true)),
                _ => {}
            }
        }

        Service {
            id: *id,
            messages,
            indications,
        }
    }

    fn message(&self, entry: &Raw, is_indication: bool) -> MessageDef {
        let name = entry.name.clone().expect("message without a name");
        let mut builder = Builder {
            common_tlvs: &self.common_tlvs,
            common_prereqs: &self.common_prereqs,
            types: Vec::new(),
            used: HashMap::new(),
        };

        let input = if is_indication {
            Vec::new()
        } else {
            builder.container(entry.input.as_deref().unwrap_or(&[]), false)
        };
        let output = builder.container(entry.output.as_deref().unwrap_or(&[]), true);

        MessageDef {
            name,
            module: naming::snake(entry.name.as_deref().unwrap()),
            id: entry.id_value().expect("message without an id") as u16,
            input,
            output,
            types: builder.types,
        }
    }
}

struct Builder<'a> {
    common_tlvs: &'a HashMap<String, Raw>,
    common_prereqs: &'a HashMap<String, json::Prerequisite>,
    types: Vec<Composite>,
    used: HashMap<String, u32>,
}

impl Builder<'_> {
    fn container(&mut self, raws: &[Raw], output: bool) -> Vec<RField> {
        let mut expanded: Vec<Raw> = raws.iter().map(|raw| self.expand(raw)).collect();

        // A field carrying prerequisites is ordered after the fields it
        // references; libqmi moves every plain field ahead of the rest.
        if output {
            let (plain, guarded): (Vec<Raw>, Vec<Raw>) = expanded
                .into_iter()
                .partition(|raw| raw.prerequisites.is_none());
            expanded = plain;
            expanded.extend(guarded);
        }

        let mut fields = Vec::new();

        for raw in &expanded {
            let field = self.field(raw, &fields, output);
            fields.push(field);
        }

        fields
    }

    /// Replace a `common-ref` with the referenced TLV definition, letting the
    /// message override the id, name and prerequisites.
    fn expand(&self, raw: &Raw) -> Raw {
        let Some(reference) = &raw.common_ref else {
            return raw.clone();
        };

        let mut copy = self
            .common_tlvs
            .get(reference)
            .unwrap_or_else(|| panic!("unknown common-ref {reference}"))
            .clone();

        if raw.id.is_some() {
            copy.id = raw.id.clone();
        }
        if raw.name.is_some() {
            copy.name = raw.name.clone();
        }
        if raw.prerequisites.is_some() {
            copy.prerequisites = raw.prerequisites.clone();
        }

        copy
    }

    fn field(&mut self, raw: &Raw, prior: &[RField], output: bool) -> RField {
        let name = raw.name.clone().expect("TLV without a name");
        let id = raw.id_value().expect("TLV without an id") as u8;
        let prereqs = raw.prereq_list();
        let mandatory = self.mandatory(raw);
        let optional = !prereqs.is_empty() || !mandatory;

        let ty = self.ty(raw, true);
        // libqmi only evaluates prerequisites on output containers.
        let guards = if output {
            self.guards(prereqs, prior)
        } else {
            Vec::new()
        };

        RField {
            rust_name: naming::snake(&name),
            const_name: format!("TLV_{}", naming::screaming(&name)),
            name,
            tlv_id: id,
            optional,
            mandatory,
            is_result: raw.common_ref.as_deref() == Some(RESULT_REF),
            ty,
            guards,
        }
    }

    fn mandatory(&self, raw: &Raw) -> bool {
        match raw.mandatory.as_deref() {
            Some("yes") => true,
            Some("no") => false,
            // libqmi treats TLVs below 0x10 as mandatory by default.
            _ => raw.id_value().is_none_or(|id| id < 0x10),
        }
    }

    fn ty(&mut self, raw: &Raw, top_level: bool) -> RType {
        let endian = match raw.endian.as_deref() {
            Some("big") | Some("network") => E::Big,
            _ => E::Little,
        };

        match raw.format.as_deref().expect("field without a format") {
            "guint8" => {
                if self.is_bool(raw) {
                    RType::Bool { signed: false }
                } else {
                    RType::U8
                }
            }
            "gint8" => {
                if self.is_bool(raw) {
                    RType::Bool { signed: true }
                } else {
                    RType::I8
                }
            }
            "guint16" => RType::U16(endian),
            "gint16" => RType::I16(endian),
            "guint32" => RType::U32(endian),
            "gint32" => RType::I32(endian),
            "guint64" => RType::U64(endian),
            "gint64" => RType::I64(endian),
            "gfloat" => RType::F32(endian),
            "gdouble" => RType::F64(endian),
            "guint-sized" => RType::SizedUint {
                bytes: raw
                    .guint_size
                    .as_deref()
                    .map(|s| json::parse_int(s) as usize)
                    .expect("guint-sized without guint-size"),
                e: endian,
            },
            "string" => {
                let fixed = raw.fixed_size_value();

                let prefix = if fixed.is_some() {
                    0
                } else if let Some(format) = &raw.size_prefix_format {
                    match format.as_str() {
                        "guint8" => 1,
                        "guint16" => 2,
                        _ => panic!("invalid string size prefix format {format}"),
                    }
                } else if top_level {
                    // A string that is the whole TLV value has no prefix.
                    0
                } else {
                    // Nested strings default to a one byte length prefix.
                    1
                };

                RType::Str {
                    prefix,
                    max: raw.max_size_value(),
                    fixed,
                }
            }
            "sequence" | "struct" => {
                let name = self.composite_name(&raw.name.clone().expect("sequence without a name"));
                let members = self.members(raw.contents.as_deref().unwrap_or(&[]));

                self.types.push(Composite {
                    name: name.clone(),
                    kind: CompositeKind::Struct(members),
                });

                RType::Composite(name)
            }
            "array" => {
                let fixed = raw.fixed_size_value();
                let prefix = if fixed.is_some() {
                    None
                } else {
                    Some(
                        match raw
                            .size_prefix_format
                            .as_deref()
                            .expect("array without size prefix")
                        {
                            "guint8" => 1,
                            "guint16" => 2,
                            "guint32" => 4,
                            other => panic!("invalid array size prefix format {other}"),
                        },
                    )
                };

                let element = raw
                    .array_element
                    .as_deref()
                    .expect("array without an element");
                let elem = self.ty(element, false);

                if let Some(sequence) = &raw.sequence_prefix_format {
                    if sequence != "guint8" {
                        panic!("unsupported sequence prefix format {sequence}");
                    }

                    let name =
                        self.composite_name(&raw.name.clone().expect("array without a name"));
                    self.types.push(Composite {
                        name: name.clone(),
                        kind: CompositeKind::SeqArray {
                            count_prefix: prefix.expect("sequence prefix without size prefix"),
                            elem: Box::new(elem),
                        },
                    });

                    RType::Composite(name)
                } else {
                    RType::Array {
                        elem: Box::new(elem),
                        prefix,
                        fixed,
                    }
                }
            }
            other => panic!("unsupported format {other}"),
        }
    }

    fn members(&mut self, raws: &[Raw]) -> Vec<RMember> {
        raws.iter()
            .map(|raw| {
                let name = raw.name.clone().expect("member without a name");

                RMember {
                    rust_name: naming::snake(&name),
                    ty: self.ty(raw, false),
                    name,
                }
            })
            .collect()
    }

    /// Composite type names are shared by the whole message module, so keep
    /// them unique with a numeric suffix on collisions.
    fn composite_name(&mut self, field_name: &str) -> String {
        let mut base = naming::camel(field_name);

        if base == "Result" {
            base = "OperationResult".into();
        }

        let count = self.used.entry(base.clone()).or_insert(0);
        *count += 1;

        if *count == 1 {
            base
        } else {
            format!("{base}{count}")
        }
    }

    fn is_bool(&self, raw: &Raw) -> bool {
        raw.public_format.as_deref() == Some("gboolean")
    }

    fn guards(&self, prereqs: &[json::Prerequisite], prior: &[RField]) -> Vec<Guard> {
        prereqs
            .iter()
            .map(|prereq| {
                let (path, operation, value) = match &prereq.common_ref {
                    Some(reference) => {
                        let common = self
                            .common_prereqs
                            .get(reference)
                            .unwrap_or_else(|| panic!("unknown common prerequisite {reference}"));

                        (
                            common
                                .field
                                .clone()
                                .expect("common prerequisite without a field"),
                            common
                                .operation
                                .clone()
                                .expect("common prerequisite without an operation"),
                            common
                                .value
                                .clone()
                                .expect("common prerequisite without a value"),
                        )
                    }
                    None => (
                        prereq.field.clone().expect("prerequisite without a field"),
                        prereq
                            .operation
                            .clone()
                            .expect("prerequisite without an operation"),
                        prereq.value.clone().expect("prerequisite without a value"),
                    ),
                };

                Guard {
                    access: self.resolve_access(&path, prior),
                    op: match operation.as_str() {
                        "==" => "==",
                        "!=" => "!=",
                        other => panic!("unsupported prerequisite operation {other}"),
                    },
                    value: resolve_value(&value),
                }
            })
            .collect()
    }

    /// Resolve a prerequisite field path such as `"Result Error Status"` into
    /// the access chain `result.error_status`.
    fn resolve_access(&self, path: &str, fields: &[RField]) -> Access {
        for field in fields {
            if path == field.name {
                return Access {
                    levels: vec![AccessLevel {
                        rust_name: field.rust_name.clone(),
                        optional: field.optional,
                    }],
                };
            }

            if let Some(rest) = path
                .strip_prefix(field.name.as_str())
                .and_then(|r| r.strip_prefix(' '))
            {
                let members = self.members_of(&field.ty);
                let mut levels = vec![AccessLevel {
                    rust_name: field.rust_name.clone(),
                    optional: field.optional,
                }];

                levels.extend(self.resolve_members(rest, &members));

                return Access { levels };
            }
        }

        panic!(
            "cannot resolve prerequisite path {path:?} against {:?}",
            fields.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    fn resolve_members(&self, path: &str, members: &[RMember]) -> Vec<AccessLevel> {
        for member in members {
            if path == member.name {
                return vec![AccessLevel {
                    rust_name: member.rust_name.clone(),
                    optional: false,
                }];
            }

            if let Some(rest) = path
                .strip_prefix(member.name.as_str())
                .and_then(|r| r.strip_prefix(' '))
            {
                let nested = self.members_of(&member.ty);
                let mut levels = vec![AccessLevel {
                    rust_name: member.rust_name.clone(),
                    optional: false,
                }];

                levels.extend(self.resolve_members(rest, &nested));

                return levels;
            }
        }

        panic!("cannot resolve prerequisite member path {path:?}");
    }

    fn members_of(&self, ty: &RType) -> Vec<RMember> {
        match ty {
            RType::Composite(name) => {
                match self.types.iter().find(|c| &c.name == name).map(|c| &c.kind) {
                    Some(CompositeKind::Struct(members)) => members.clone(),
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }
}

/// Map the C enumeration values used in prerequisites to their numbers.
fn resolve_value(symbol: &str) -> u64 {
    match symbol {
        "QMI_STATUS_SUCCESS" => 0,
        "QMI_PROTOCOL_ERROR_CALL_FAILED" => 14,
        "QMI_PROTOCOL_ERROR_OUT_OF_CALL" => 15,
        "QMI_PROTOCOL_ERROR_WMS_CAUSE_CODE" => 54,
        "QMI_PROTOCOL_ERROR_EXTENDED_INTERNAL" => 81,
        "QMI_PROTOCOL_ERROR_ACK_NOT_SENT" => 84,
        "QMI_DMS_OPERATING_MODE_OFFLINE" => 3,
        "QMI_OMA_SESSION_STATE_FAILED" => 2,
        other => panic!("unknown prerequisite value {other}"),
    }
}
