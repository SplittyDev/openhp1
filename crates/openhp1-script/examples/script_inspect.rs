use std::{env, error::Error, path::PathBuf};

use openhp1_package::Package;
use openhp1_script::{ScriptExport, token_name};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: script_inspect <package> <export-index>")?;
    let export_index = arguments
        .next()
        .ok_or("usage: script_inspect <package> <export-index>")?
        .to_string_lossy()
        .parse::<usize>()?;
    let package = Package::open(&path)?;
    let script = ScriptExport::decode(&package, export_index)?;
    let name = package
        .summary()
        .name(package.summary().exports[export_index].object_name);
    println!(
        "{}:{name} ({}, {} raw bytes, {} execution bytes)",
        path.display(),
        script.class_name,
        script.bytecode.raw_len,
        script.bytecode.bytes.len()
    );
    for token in script.bytecode.tokens {
        println!(
            "  {:#06x} {:indent$}{}{call}",
            token.offset,
            "",
            token_name(token.opcode),
            indent = usize::from(token.depth) * 2,
            call = token
                .call
                .map(|call| format!(" {call:?}"))
                .unwrap_or_default()
        );
    }
    Ok(())
}
