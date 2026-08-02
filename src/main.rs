// SPDX-License-Identifier: MPL-2.0

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use oon::{
    Source, compile_schema, evaluate, evaluate_with_initial, parse_json_value, parse_overlay,
};

fn usage() -> ! {
    eprintln!(
        "usage: oon --schema CONFIG.sch.oon [--initial-value VALUE.json] [OVERLAY.oon ...]\n       oon --schema CONFIG.sch.oon [--initial-value VALUE.json] --overlays-dir DIRECTORY"
    );
    std::process::exit(2);
}

fn read(path: &Path) -> Result<Source, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: I/O: {error}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{}: I/O: file is not valid UTF-8", path.display()))?;
    Ok(Source {
        name: path.display().to_string(),
        text,
    })
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--schema")) {
        usage();
    }
    let schema = args.next().map(PathBuf::from).unwrap_or_else(|| usage());
    let rest: Vec<_> = args.collect();
    let mut initial_path = None;
    let mut overlays_directory = None;
    let mut explicit_paths = Vec::new();
    let mut index = 0;
    while index < rest.len() {
        if rest[index] == "--initial-value" {
            if initial_path.is_some()
                || index + 1 == rest.len()
                || matches!(
                    rest[index + 1].to_str(),
                    Some("--initial-value" | "--overlays-dir" | "--schema")
                )
            {
                usage();
            }
            initial_path = Some(PathBuf::from(&rest[index + 1]));
            index += 2;
        } else if rest[index] == "--overlays-dir" {
            if overlays_directory.is_some()
                || index + 1 == rest.len()
                || matches!(
                    rest[index + 1].to_str(),
                    Some("--initial-value" | "--overlays-dir" | "--schema")
                )
            {
                usage();
            }
            overlays_directory = Some(PathBuf::from(&rest[index + 1]));
            index += 2;
        } else if rest[index] == "--schema" {
            usage();
        } else {
            explicit_paths.push(PathBuf::from(&rest[index]));
            index += 1;
        }
    }
    if overlays_directory.is_some() && !explicit_paths.is_empty() {
        usage();
    }
    let overlay_paths = if let Some(directory) = overlays_directory {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("{}: I/O: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("{}: I/O: {error}", directory.display()))?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry
                .file_type()
                .map_err(|error| format!("{}: I/O: {error}", path.display()))?
                .is_file()
                && name.ends_with(".oon")
                && !name.ends_with(".sch.oon")
            {
                paths.push(path);
            }
        }
        paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
        paths
    } else {
        explicit_paths
    };
    let schema = compile_schema(read(&schema)?).map_err(|report| report.to_string())?;
    let overlays = overlay_paths
        .iter()
        .map(|path| read(path))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(parse_overlay)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|report| report.to_string())?;
    let value = if let Some(path) = initial_path {
        let initial =
            parse_json_value(&schema, read(&path)?).map_err(|report| report.to_string())?;
        evaluate_with_initial(&schema, &initial, &overlays)
    } else {
        evaluate(&schema, &overlays)
    }
    .map_err(|report| report.to_string())?;
    let json = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}
