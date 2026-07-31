use mythicraft_compat::{
    import_luckperms_config, import_mythicmobs, import_server_properties, import_vault_config,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, fs, path::PathBuf, process};

#[derive(Debug, Serialize)]
struct CliReport {
    kind: String,
    file: String,
    dry_run: bool,
    report: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BatchEntry {
    source_version: String,
    status: String,
    covered_fields: Vec<String>,
    unsupported: Vec<String>,
    diagnostics: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BatchReport {
    entries: std::collections::BTreeMap<String, BatchEntry>,
}

fn adapter_value<T: serde::Serialize>(result: Result<T, mythicraft_compat::ImportError>) -> Value {
    match result {
        Ok(report) => serde_json::to_value(report).unwrap(),
        Err(error) => {
            eprintln!("import failed: {error}");
            process::exit(1);
        }
    }
}
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 && args[1] == "batch" {
        run_batch(&args[2..]);
        return;
    }
    if args.len() < 3 || args.iter().any(|arg| arg == "--help") {
        eprintln!("usage: compat-report <paper|vault|luckperms|mythicmobs> <file> [--dry-run]\n       compat-report batch <fixtures-dir> <golden.json> [--check]");
        process::exit(if args.iter().any(|arg| arg == "--help") {
            0
        } else {
            2
        });
    }
    let kind = &args[1];
    let file = &args[2];
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let source = read(file);
    let report = import_kind(kind, file, &source, dry_run);
    println!(
        "{}",
        serde_json::to_string_pretty(&CliReport {
            kind: kind.clone(),
            file: file.clone(),
            dry_run,
            report
        })
        .unwrap()
    );
}
fn read(file: &str) -> String {
    match fs::read_to_string(file) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("cannot read {file}: {error}");
            process::exit(1)
        }
    }
}
fn import_kind(kind: &str, file: &str, source: &str, dry_run: bool) -> Value {
    match kind {
        "paper" => serde_json::to_value(import_server_properties(file, source, dry_run)).unwrap(),
        "vault" => adapter_value(import_vault_config(file, source, dry_run)),
        "luckperms" => adapter_value(import_luckperms_config(file, source, dry_run)),
        "mythicmobs" => match import_mythicmobs(file, source, dry_run) {
            Ok(report) => serde_json::to_value(report).unwrap(),
            Err(error) => {
                eprintln!("import failed: {error}");
                process::exit(1)
            }
        },
        _ => {
            eprintln!("unsupported kind: {kind}");
            process::exit(2)
        }
    }
}
fn run_batch(args: &[String]) {
    if args.len() < 2 {
        eprintln!("usage: compat-report batch <fixtures-dir> <golden.json> [--check]");
        process::exit(2);
    }
    let root = PathBuf::from(&args[0]);
    let golden = PathBuf::from(&args[1]);
    let check = args.iter().any(|arg| arg == "--check");
    let mut entries = std::collections::BTreeMap::new();
    for (kind, relative) in [
        ("paper", "paper/server.properties"),
        ("vault", "vault/config.yml"),
        ("luckperms", "luckperms/config.yml"),
        ("mythicmobs", "mythicmobs/basic.yml"),
        ("mythicmobs-minimal", "mythicmobs/minimal.yml"),
    ] {
        let path = root.join(relative);
        if !path.exists() {
            continue;
        }
        let file = path.to_string_lossy().to_string();
        let value = import_kind(kind, &file, &read(&file), true);
        entries.insert(kind.into(), summary(kind, &value));
    }
    let actual = BatchReport { entries };
    if check {
        let expected: BatchReport = match serde_json::from_str(
            fs::read_to_string(&golden)
                .unwrap_or_default()
                .trim_start_matches(char::from_u32(0xfeff).unwrap()),
        ) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("invalid golden report: {error}");
                process::exit(1)
            }
        };
        if actual != expected {
            eprintln!(
                "golden mismatch\nexpected:\n{}\nactual:\n{}",
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual).unwrap()
            );
            process::exit(1);
        }
        eprintln!("golden check passed: {}", golden.display());
    }
    println!("{}", serde_json::to_string_pretty(&actual).unwrap());
}
fn summary(kind: &str, value: &Value) -> BatchEntry {
    let report = &value;
    let source_version = report
        .get("source_version")
        .and_then(Value::as_str)
        .unwrap_or(kind)
        .into();
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("Invalid")
        .into();
    let covered_fields = report
        .get("covered_fields")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let unsupported = if let Some(values) = report.get("unsupported").and_then(Value::as_array) {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect()
    } else {
        report
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(|d| d.get("code").and_then(Value::as_str))
                    .filter(|code| code.contains("UNKNOWN") || code.contains("UNSUPPORTED"))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    };
    let diagnostics = report
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|diagnostic| diagnostic.get("code").and_then(Value::as_str))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    BatchEntry {
        source_version,
        status,
        covered_fields,
        unsupported,
        diagnostics,
    }
}
