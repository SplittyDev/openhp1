use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use openhp1_package::{PACKAGE_MAGIC, Package};

fn main() -> Result<(), Box<dyn Error>> {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("res"));
    let mut paths = Vec::new();
    collect_package_paths(&root, &mut paths)?;
    paths.sort();

    let mut versions = BTreeMap::<u16, usize>::new();
    let mut names = 0;
    let mut imports = 0;
    let mut exports = 0;
    for path in &paths {
        let package = Package::open(path)?;
        let summary = package.summary();
        *versions.entry(summary.header.version).or_default() += 1;
        names += summary.names.len();
        imports += summary.imports.len();
        exports += summary.exports.len();
    }

    println!(
        "parsed {} packages: {} names, {} imports, {} exports",
        paths.len(),
        names,
        imports,
        exports
    );
    for (version, count) in versions {
        println!("  version {version}: {count}");
    }
    Ok(())
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
