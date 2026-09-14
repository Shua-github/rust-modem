//! Turning generated code into Rust source files.
//!
//! The generators build a [`TokenStream`] rather than a string, so what they
//! emit is guaranteed to tokenize; [`write`] additionally parses the result
//! and runs it through `rustfmt`, which keeps the checked in files readable.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use proc_macro2::TokenStream;

/// Concatenate a `//!` header with the generated code.
pub fn render(header: &str, tokens: &TokenStream) -> String {
    let mut source = String::with_capacity(header.len() + 4096);
    source.push_str(header);
    source.push_str(&tokens.to_string());
    source
}

/// Parse and format `source`, then write it to `path`.
pub fn write(path: &Path, source: String) -> Result<(), Box<dyn Error>> {
    let source = finalize(source)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &source)?;
    eprintln!("wrote {} ({} bytes)", path.display(), source.len());

    Ok(())
}

/// Reject `source` if it does not parse, then format it with `rustfmt`.
pub fn finalize(source: String) -> Result<String, Box<dyn Error>> {
    syn::parse_file(&source).map_err(|error| format!("generated code does not parse: {error}"))?;

    match rustfmt(&source) {
        Some(formatted) => Ok(formatted),
        // The unformatted source is still valid, so a missing or unhappy
        // rustfmt is only a cosmetic problem.
        None => {
            eprintln!("warning: rustfmt failed, writing unformatted code");
            Ok(source)
        }
    }
}

/// Run `rustfmt` over `source`, editing it through stdin and stdout.
fn rustfmt(source: &str) -> Option<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;

    // Dropping the handle closes stdin, which is what ends rustfmt's read.
    child.stdin.take()?.write_all(source.as_bytes()).ok()?;

    let output = child.wait_with_output().ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}
