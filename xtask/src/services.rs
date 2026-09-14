//! The service database a generator reads: one JSON file per service,
//! `qmi-service-*.json` for QMI and `mbim-service-*.json` for MBIM.

use std::error::Error;
use std::path::{Path, PathBuf};

/// The `<prefix>*.json` files of `data`, in file name order.
pub fn files(data: &Path, prefix: &str) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = std::fs::read_dir(data)
        .map_err(|error| format!("cannot list {}: {error}", data.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = file_name(path);
            name.starts_with(prefix) && name.ends_with(".json")
        })
        .collect::<Vec<_>>();
    files.sort();

    Ok(files)
}

/// Module name of the service in `file`, e.g. `ctl` for
/// `qmi-service-ctl.json`.
pub fn module_name(file: &Path, prefix: &str) -> String {
    file_name(file)
        .trim_start_matches(prefix)
        .trim_end_matches(".json")
        .replace('-', "_")
}

/// File name of `path`, or the empty string if it is not representable.
pub fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}
