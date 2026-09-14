//! Build tasks. Run as `cargo xtask <command>`.

mod mbim;
mod qmi;
mod qmi_wwan;
mod rust;
mod services;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use ureq::tls::{TlsConfig, TlsProvider};

use qmi_wwan::QmiTable;

/// Upstream file the vendor table is generated from.
const SOURCE_URL: &str =
    "https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/usb/qmi_wwan.c";

/// Vendor table compiled into rm-qmi, relative to the workspace root.
const QMI_DEVICE: &str = "rm-qmi/src/qmi_device.rs";

/// libqmi service database, relative to the workspace root.
const QMI_DATA: &str = "ref/libqmi/data";

/// Directory holding the generated QMI `types.rs` and `types/` modules.
const QMI_TYPES: &str = "rm-qmi/src";

/// libmbim service database, relative to the workspace root.
const MBIM_DATA: &str = "ref/libmbim/data";

/// Directory holding the generated MBIM `types.rs` and `types/` modules.
const MBIM_TYPES: &str = "rm-mbim/src";

#[derive(Parser)]
#[command(name = "xtask", about = "Build tasks for the rust_modem workspace")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Generate `rm-qmi/src/qmi_device.rs` from the kernel's QMI WWAN driver.
    QmiTable {
        /// `qmi_wwan.c` to parse.
        #[arg(long, default_value = SOURCE_URL)]
        url: String,

        /// Also write the parsed table as JSON, for diffing.
        #[arg(long, value_name = "PATH")]
        json: Option<PathBuf>,

        /// File to write.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },

    /// Generate the QMI service types of `rm-qmi` from the libqmi database.
    QmiCodegen {
        /// Directory of `qmi-service-*.json` files.
        #[arg(long, value_name = "DIR")]
        data: Option<PathBuf>,

        /// Crate `src` directory to generate into.
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
    },

    /// Generate the MBIM service types of `rm-mbim` from the libmbim database.
    MbimCodegen {
        /// Directory of `mbim-service-*.json` files.
        #[arg(long, value_name = "DIR")]
        data: Option<PathBuf>,

        /// Crate `src` directory to generate into.
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().task {
        Task::QmiTable { url, json, output } => qmi_table(&url, json.as_deref(), output),
        Task::QmiCodegen { data, output } => {
            let root = workspace_root();
            qmi::generate(
                &data.unwrap_or_else(|| root.join(QMI_DATA)),
                &output.unwrap_or_else(|| root.join(QMI_TYPES)),
            )
        }
        Task::MbimCodegen { data, output } => {
            let root = workspace_root();
            mbim::generate(
                &data.unwrap_or_else(|| root.join(MBIM_DATA)),
                &output.unwrap_or_else(|| root.join(MBIM_TYPES)),
            )
        }
    }
}

/// Fetch `qmi_wwan.c` and write its `products[]` table as Rust.
fn qmi_table(
    url: &str,
    json: Option<&Path>,
    output: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let output = output.unwrap_or_else(|| workspace_root().join(QMI_DEVICE));

    // The default provider is rustls, but only native-tls is enabled.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into();

    eprintln!("fetching {url}");
    let source = agent.get(url).call()?.body_mut().read_to_string()?;

    let table = QmiTable::parse(url, &source)?;
    eprintln!("parsed {} entries", table.entries.len());

    if let Some(json) = json {
        write_json(json, &table)?;
    }

    rust::write(&output, table.to_rust())
}

/// Write the parsed table as pretty printed JSON.
fn write_json(path: &Path, table: &QmiTable) -> Result<(), Box<dyn Error>> {
    let mut json = serde_json::to_string_pretty(table)?;
    json.push('\n');

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json)?;
    eprintln!("wrote {}", path.display());

    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the workspace")
        .to_path_buf()
}
