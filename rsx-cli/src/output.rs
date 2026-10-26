use crate::record;
use crate::record::DisplayScale;
use rsx_cast::wal::RawWalRecord;
use serde_json::json;
use std::fmt::Display;
use std::process::ExitCode;

#[cfg(test)]
#[path = "output_test.rs"]
mod output_test;

/// Why a command failed; `Runtime` exits 1, `Misuse` exits 2.
#[derive(Debug)]
pub enum Failure {
    Runtime(String),
    Misuse(String),
}

pub type Outcome = Result<(), Failure>;

impl Failure {
    pub fn runtime(message: impl Display) -> Self {
        Failure::Runtime(message.to_string())
    }

    pub fn misuse(message: impl Display) -> Self {
        Failure::Misuse(message.to_string())
    }

    pub fn message(&self) -> &str {
        match self {
            Failure::Runtime(message) | Failure::Misuse(message) => message,
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Failure::Runtime(_) => 1,
            Failure::Misuse(_) => 2,
        }
    }
}

/// How a record line is rendered; text divides by the scale, JSON stays raw.
pub enum Format {
    Text(DisplayScale),
    Json,
}

/// A number for a text table cell, `-` when absent.
pub fn show(value: Option<u64>) -> String {
    value.map_or("-".to_string(), |value| value.to_string())
}

/// Prints the failure on stderr: one `{"error": ...}` line under `--json`,
/// `Error: ...` otherwise. Returns the exit code.
pub fn report(failure: &Failure, is_json: bool) -> ExitCode {
    if is_json {
        eprintln!("{}", json!({ "error": failure.message() }));
    } else {
        eprintln!("Error: {}", failure.message());
    }
    ExitCode::from(failure.exit_code())
}

pub fn print_record(format: &Format, raw: &RawWalRecord) {
    match format {
        Format::Json => println!("{}", record::to_json(raw)),
        Format::Text(scale) => println!("{}", record::to_text(raw, scale)),
    }
}
