use std::{env, error::Error, path::PathBuf};

use openhp1_package::Package;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: cargo run -p openhp1-package --example package_inspect -- <package>")?;
    let package = Package::open(&path)?;
    let summary = package.summary();

    println!(
        "{}: version {} (licensee {}), {} names, {} imports, {} exports",
        path.display(),
        summary.header.version,
        summary.header.licensee_version,
        summary.names.len(),
        summary.imports.len(),
        summary.exports.len()
    );

    for (index, import) in summary.imports.iter().enumerate() {
        println!(
            "  import {index:>5}: {:<28} {:<20} outer={}",
            summary.name(import.object_name),
            summary.name(import.class_name),
            summary.object_name(import.outer).unwrap_or("-")
        );
    }
    for (index, export) in summary.exports.iter().enumerate() {
        println!(
            "  export {index:>5}: {:<28} {:<20} outer={:<20} size={:#x} offset={}",
            summary.name(export.object_name),
            summary.class_name(export).unwrap_or("<class>"),
            summary.object_name(export.outer).unwrap_or("-"),
            export.serial_size,
            export
                .serial_offset
                .map(|offset| format!("{offset:#x}"))
                .unwrap_or_else(|| "-".into())
        );
    }
    Ok(())
}
