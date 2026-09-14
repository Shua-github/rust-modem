//! Identifier helpers for the generated Rust types.

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Split on anything that is not alphanumeric, capitalizing each word.
///
/// `"GSM WCDMA Cause Info"` becomes `GsmWcdmaCauseInfo`. Identifiers that
/// would start with a digit are prefixed with `N`, e.g. `"3GPP Time"` becomes
/// `N3gppTime`.
pub fn camel(name: &str) -> String {
    let mut out = String::new();

    for word in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut chars = word.chars();

        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
        }

        for c in chars {
            out.push(c.to_ascii_lowercase());
        }
    }

    if out.is_empty() {
        out.push_str("Unnamed");
    }

    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, 'N');
    }

    out
}

/// Split on anything that is not alphanumeric and lowercase the words.
///
/// `"Message ID"` becomes `message_id`. Identifiers that would start with a
/// digit get an `n_` prefix, and Rust keywords get a trailing underscore.
pub fn snake(name: &str) -> String {
    let mut parts = Vec::new();

    for word in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        if !word.is_empty() {
            parts.push(word.to_ascii_lowercase());
        }
    }

    let mut out = parts.join("_");

    if out.is_empty() {
        out.push_str("unnamed");
    }

    if out.as_bytes()[0].is_ascii_digit() {
        out.insert_str(0, "n_");
    }

    if KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }

    out
}

/// Upper-case snake case, used for TLV constants.
pub fn screaming(name: &str) -> String {
    snake(name).to_ascii_uppercase()
}
