//! `poid` — the POID command-line tool.
//!
//! For developers only; end users never touch a terminal (the "no terminal"
//! rule protects users, not people who live in a shell). Every command
//! supports `--json` for scripting and for the MCP server. Exit code 0 means
//! success; for `validate` it is the conformance verdict itself.

mod commands;
mod output;
mod project;
mod stdlib;
mod templates;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use crate::output::{CmdError, Report};

/// Create, inspect and validate POID containers.
#[derive(Parser)]
#[command(name = "poid", version, about)]
struct Cli {
    /// Machine-readable JSON output on stdout (also for errors).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new POID project.
    Init {
        /// Directory to create the project in.
        dir: PathBuf,
        /// Project template.
        #[arg(long, value_enum, default_value_t = Template::Web)]
        template: Template,
        /// Write into a non-empty directory.
        #[arg(long)]
        force: bool,
    },
    /// Build a project and package it into a .poid container.
    Pack {
        /// Project directory (must contain poid.json).
        dir: PathBuf,
        /// Output file. Defaults to `<dir-name>.poid`.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert a folder, ZIP, HTML file or AI artifact into a .poid.
    Convert {
        /// Input: a project folder, a .zip, a .html document or a .jsx/.tsx artifact.
        input: PathBuf,
        /// Output file. Defaults to <name>.poid.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Conformance-check a container. Exit code 0 = conformant.
    Validate {
        /// The .poid file to check.
        file: PathBuf,
    },
    /// Show manifest, file tree, permissions and sizes.
    Inspect {
        /// The .poid file to inspect.
        file: PathBuf,
    },
    /// Unpack a container into a directory.
    Extract {
        /// The .poid file to unpack.
        file: PathBuf,
        /// Destination directory.
        #[arg(short, long)]
        output: PathBuf,
        /// Write into a non-empty directory.
        #[arg(long)]
        force: bool,
    },
    /// Export the user's data. Your data is always extractable (SECURITY §6).
    Data {
        /// The .poid file to read.
        file: PathBuf,
        /// Where to write the exported data.
        #[arg(long)]
        export: PathBuf,
    },
    /// Generate an Ed25519 signing key.
    Keygen {
        /// Where to write the private key. Keep this file secret.
        #[arg(short, long)]
        output: PathBuf,
        /// Overwrite an existing key file.
        #[arg(long)]
        force: bool,
    },
    /// Sign a container in place (writes signature/signature.json).
    Sign {
        /// The .poid file to sign.
        file: PathBuf,
        /// Path to the private key file from `poid keygen`.
        #[arg(long)]
        key: PathBuf,
    },
    /// Update a POID's program in place, keeping its data (SPEC §12).
    /// Swaps app/, deps/, migrations/ and the program manifest fields for a
    /// newer build with the same app.id; preserves data/, slots/ and identity.
    Update {
        /// The .poid file to update in place.
        file: PathBuf,
        /// The newer .poid whose program replaces `file`'s.
        #[arg(long)]
        from: PathBuf,
    },
    /// Verify integrity and signature.
    Verify {
        /// The .poid file to verify.
        file: PathBuf,
    },
    /// Encrypt a container's embedded data with a passphrase (SPEC §9.2).
    /// AES-256-GCM + Argon2id. Sending a POID with no data is safer still.
    Protect {
        /// The .poid file to protect in place.
        file: PathBuf,
        /// The passphrase. Prefer POID_PASSPHRASE in the environment.
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Decrypt a protected container's data back to plaintext (SPEC §9.2).
    Unprotect {
        /// The .poid file to unprotect in place.
        file: PathBuf,
        /// The passphrase. Prefer POID_PASSPHRASE in the environment.
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Run the conformance suite: any implementation can use this to claim
    /// conformance (SPEC 14, spec/CONFORMANCE.md). Exit 0 = 100% passed.
    Conformance {
        /// Suite directory containing valid/ and invalid/ fixture folders.
        suite: PathBuf,
    },
}

/// Project template for `poid init`.
#[derive(Clone, Copy, ValueEnum)]
enum Template {
    /// Plain HTML/CSS/JS application (works without any build tool).
    Web,
    /// Python application (runtime profile `web+python`, runs via Pyodide).
    Python,
    /// Offline survey / data-collection form.
    Survey,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(report) => {
            if cli.json {
                println!("{}", report.json);
            } else {
                println!("{}", report.human);
            }
            if report.exit_failure {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "error": {
                        "code": e.code,
                        "poid": e.poid_code,
                        "message": e.message,
                    } })
                );
            } else {
                let registry = e
                    .poid_code
                    .as_ref()
                    .map(|c| format!(" / {c}"))
                    .unwrap_or_default();
                eprintln!("error[{}{registry}]: {}", e.code, e.message);
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<Report, CmdError> {
    match &cli.command {
        Command::Init {
            dir,
            template,
            force,
        } => commands::init(dir, *template, *force),
        Command::Pack { dir, output } => commands::pack(dir, output.as_deref()),
        Command::Convert { input, output } => commands::convert(input, output.as_deref()),
        Command::Validate { file } => commands::validate(file),
        Command::Inspect { file } => commands::inspect(file),
        Command::Extract {
            file,
            output,
            force,
        } => commands::extract(file, output, *force),
        Command::Data { file, export } => commands::data(file, export),
        Command::Keygen { output, force } => commands::keygen(output, *force),
        Command::Sign { file, key } => commands::sign(file, key),
        Command::Update { file, from } => commands::update(file, from),
        Command::Verify { file } => commands::verify(file),
        Command::Protect { file, passphrase } => commands::protect(file, passphrase.as_deref()),
        Command::Unprotect { file, passphrase } => commands::unprotect(file, passphrase.as_deref()),
        Command::Conformance { suite } => commands::conformance(suite),
    }
}
