use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use openhp1_package::{PACKAGE_MAGIC, Package};
use openhp1_script::{CallTarget, ScriptExport, ScriptMetadata, token_name};

const FUNCTION_EVENT: u32 = 0x0000_0800;

fn main() -> Result<(), Box<dyn Error>> {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res"));
    let mut paths = Vec::new();
    collect_package_paths(&root, &mut paths)?;
    paths.sort();

    let mut exports = 0;
    let mut functions = 0;
    let mut events = 0;
    let mut raw_bytes = 0;
    let mut decoded_bytes = 0;
    let mut tokens = 0;
    let mut opcodes = BTreeMap::<u8, usize>::new();
    let mut native_calls = BTreeMap::<u16, usize>::new();
    let mut named_calls = BTreeMap::<String, usize>::new();
    let mut native_names = BTreeMap::<u16, BTreeSet<String>>::new();

    for path in &paths {
        let package = Package::open(path)?;
        for (export_index, export) in package.summary().exports.iter().enumerate() {
            if export.class != openhp1_package::ObjectReference::None
                && !matches!(
                    package.summary().class_name(export),
                    Some("Struct" | "Function" | "State" | "Class")
                )
            {
                continue;
            }
            let script = ScriptExport::decode(&package, export_index).map_err(|error| {
                format!(
                    "{}:{}: {error}",
                    path.display(),
                    package.summary().name(export.object_name)
                )
            })?;
            exports += 1;
            raw_bytes += script.bytecode.raw_len;
            decoded_bytes += script.bytecode.bytes.len();
            tokens += script.bytecode.tokens.len();
            if let ScriptMetadata::Function(metadata) = &script.metadata {
                functions += 1;
                events += usize::from(metadata.flags & FUNCTION_EVENT != 0);
                if metadata.native_index != 0 {
                    native_names
                        .entry(metadata.native_index)
                        .or_default()
                        .insert(package.summary().name(export.object_name).to_owned());
                }
            }
            for token in &script.bytecode.tokens {
                *opcodes.entry(token.opcode).or_default() += 1;
                match token.call {
                    Some(CallTarget::Native(index)) => {
                        *native_calls.entry(index).or_default() += 1;
                    }
                    Some(CallTarget::Virtual(name) | CallTarget::Global(name)) => {
                        *named_calls
                            .entry(package.summary().name(name).to_owned())
                            .or_default() += 1;
                    }
                    Some(CallTarget::Final(object)) => {
                        let name = package
                            .summary()
                            .object_name(object)
                            .unwrap_or("<unresolved>")
                            .to_owned();
                        *named_calls.entry(name).or_default() += 1;
                    }
                    None => {}
                }
            }
        }
    }

    println!(
        "decoded {exports} script exports ({functions} functions, {events} events) from {} packages",
        paths.len()
    );
    println!("bytecode: {raw_bytes} raw bytes, {decoded_bytes} execution bytes, {tokens} tokens");
    print_top(
        "opcodes",
        opcodes
            .into_iter()
            .map(|(opcode, count)| (format!("{opcode:#04x} {}", token_name(opcode)), count)),
    );
    print_top(
        "native calls",
        native_calls.into_iter().map(|(index, count)| {
            let names = native_names
                .get(&index)
                .map(|names| names.iter().cloned().collect::<Vec<_>>().join("/"))
                .unwrap_or_else(|| "<unnamed>".to_owned());
            (format!("{index:#05x} {names}"), count)
        }),
    );
    print_top("named calls", named_calls);
    Ok(())
}

fn print_top(label: &str, values: impl IntoIterator<Item = (String, usize)>) {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    println!("{label}:");
    for (name, count) in values.into_iter().take(30) {
        println!("  {count:7} {name}");
    }
}

fn collect_package_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_package_paths(&path, paths)?;
            continue;
        }

        let mut magic = [0; 4];
        if File::open(&path)?.read_exact(&mut magic).is_ok() && magic == PACKAGE_MAGIC.to_le_bytes()
        {
            paths.push(path);
        }
    }
    Ok(())
}
