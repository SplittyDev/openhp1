use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use thiserror::Error;

use crate::{Export, ObjectReference, PACKAGE_MAGIC, Package, PackageSummary};

pub type ResolveResult<T> = std::result::Result<T, ResolveError>;

pub struct ResolvedObject {
    pub package: Arc<Package>,
    pub export_index: usize,
}

/// Discovers packages through `[Core.System] Paths` and caches them by their
/// case-insensitive Unreal package name.
pub struct PackageStore {
    paths: HashMap<String, PathBuf>,
    loaded: HashMap<String, Arc<Package>>,
}

impl PackageStore {
    pub fn scan_game_root(root: impl AsRef<Path>) -> ResolveResult<Self> {
        let root = root.as_ref();
        let system_dir = find_child_directory(root, "System").ok_or_else(|| {
            ResolveError::MissingSystemDirectory {
                root: root.to_path_buf(),
            }
        })?;
        let ini_path =
            find_default_ini(&system_dir).ok_or_else(|| ResolveError::MissingDefaultIni {
                system: system_dir.clone(),
            })?;
        let ini = fs::read_to_string(&ini_path).map_err(|source| ResolveError::Io {
            path: ini_path,
            source,
        })?;
        let patterns = core_system_paths(&ini);
        if patterns.is_empty() {
            return Err(ResolveError::MissingPackagePaths);
        }

        let mut paths = HashMap::new();
        for pattern in patterns {
            scan_pattern(&system_dir, &pattern, &mut paths)?;
        }
        Ok(Self {
            paths,
            loaded: HashMap::new(),
        })
    }

    pub fn package_path(&self, name: &str) -> Option<&Path> {
        self.paths
            .get(&name.to_ascii_lowercase())
            .map(PathBuf::as_path)
    }

    pub fn package_paths(&self) -> impl Iterator<Item = &Path> {
        self.paths.values().map(PathBuf::as_path)
    }

    pub fn load(&mut self, name: &str) -> ResolveResult<Arc<Package>> {
        let key = name.to_ascii_lowercase();
        if let Some(package) = self.loaded.get(&key) {
            return Ok(Arc::clone(package));
        }
        let path = self
            .paths
            .get(&key)
            .ok_or_else(|| ResolveError::MissingPackage {
                name: name.to_owned(),
            })?;
        let package = Arc::new(Package::open(path)?);
        self.loaded.insert(key, Arc::clone(&package));
        Ok(package)
    }

    pub fn load_path(&mut self, path: impl AsRef<Path>) -> ResolveResult<Arc<Package>> {
        let path = path.as_ref();
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ResolveError::InvalidPackagePath {
                path: path.to_path_buf(),
            })?;
        let key = name.to_ascii_lowercase();
        if let Some(package) = self.loaded.get(&key) {
            return Ok(Arc::clone(package));
        }
        let package = Arc::new(Package::open(path)?);
        self.paths
            .entry(key.clone())
            .or_insert_with(|| path.to_path_buf());
        self.loaded.insert(key, Arc::clone(&package));
        Ok(package)
    }

    pub fn resolve(
        &mut self,
        source: &Arc<Package>,
        reference: ObjectReference,
    ) -> ResolveResult<Option<ResolvedObject>> {
        match reference {
            ObjectReference::None => Ok(None),
            ObjectReference::Export(export_index) => Ok(Some(ResolvedObject {
                package: Arc::clone(source),
                export_index,
            })),
            ObjectReference::Import(import_index) => {
                let target = import_target(source.summary(), import_index)?;
                let package = self.load(&target.package)?;
                let export_index = find_export(
                    package.summary(),
                    &target.class,
                    &target.object,
                    &target.groups,
                )
                .ok_or_else(|| ResolveError::MissingObject {
                    package: target.package,
                    class: target.class,
                    path: target
                        .groups
                        .iter()
                        .chain(std::iter::once(&target.object))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("."),
                })?;
                Ok(Some(ResolvedObject {
                    package,
                    export_index,
                }))
            }
        }
    }
}

struct ImportTarget {
    package: String,
    groups: Vec<String>,
    object: String,
    class: String,
}

fn import_target(summary: &PackageSummary, import_index: usize) -> ResolveResult<ImportTarget> {
    let import = summary
        .imports
        .get(import_index)
        .ok_or(ResolveError::InvalidImportIndex {
            index: import_index,
            import_count: summary.imports.len(),
        })?;
    let object = summary.name(import.object_name).to_owned();
    let class = summary.name(import.class_name).to_owned();
    let mut groups = Vec::new();
    let mut outer = import.outer;

    for _ in 0..=summary.imports.len() {
        match outer {
            ObjectReference::Import(index) => {
                let entry = summary
                    .imports
                    .get(index)
                    .ok_or(ResolveError::InvalidImportIndex {
                        index,
                        import_count: summary.imports.len(),
                    })?;
                let name = summary.name(entry.object_name).to_owned();
                if entry.outer == ObjectReference::None {
                    return Ok(ImportTarget {
                        package: name,
                        groups,
                        object,
                        class,
                    });
                }
                groups.push(name);
                outer = entry.outer;
            }
            ObjectReference::None | ObjectReference::Export(_) => {
                return Err(ResolveError::ImportWithoutPackage { import_index });
            }
        }
    }
    Err(ResolveError::OuterCycle { import_index })
}

fn find_export(
    summary: &PackageSummary,
    class: &str,
    object: &str,
    groups: &[String],
) -> Option<usize> {
    summary.exports.iter().position(|export| {
        summary
            .name(export.object_name)
            .eq_ignore_ascii_case(object)
            && (summary
                .class_name(export)
                .is_some_and(|actual| actual.eq_ignore_ascii_case(class))
                || (class.eq_ignore_ascii_case("Class") && export.class == ObjectReference::None))
            && export_groups(summary, export).is_some_and(|actual| equal_names(&actual, groups))
    })
}

fn export_groups(summary: &PackageSummary, export: &Export) -> Option<Vec<String>> {
    let mut groups = Vec::new();
    let mut outer = export.outer;
    for _ in 0..=summary.imports.len() + summary.exports.len() {
        match outer {
            ObjectReference::None => return Some(groups),
            ObjectReference::Export(index) => {
                let entry = summary.exports.get(index)?;
                groups.push(summary.name(entry.object_name).to_owned());
                outer = entry.outer;
            }
            ObjectReference::Import(index) => {
                let entry = summary.imports.get(index)?;
                groups.push(summary.name(entry.object_name).to_owned());
                outer = entry.outer;
            }
        }
    }
    None
}

fn equal_names(left: &[String], right: &[String]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn find_child_directory(root: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
        })
        .map(|entry| entry.path())
}

fn find_default_ini(system_dir: &Path) -> Option<PathBuf> {
    find_file(system_dir, "Default.ini").or_else(|| {
        let mut directories: Vec<_> = fs::read_dir(system_dir)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        directories.sort();
        directories
            .into_iter()
            .find_map(|directory| find_file(&directory, "Default.ini"))
    })
}

fn find_file(directory: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
        })
        .map(|entry| entry.path())
}

fn core_system_paths(ini: &str) -> Vec<String> {
    let mut in_core_system = false;
    let mut paths = Vec::new();
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_core_system = line[1..line.len() - 1].eq_ignore_ascii_case("Core.System");
            continue;
        }
        if !in_core_system || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Paths") {
            paths.push(value.trim().replace('\\', "/"));
        }
    }
    paths
}

fn scan_pattern(
    system_dir: &Path,
    pattern: &str,
    paths: &mut HashMap<String, PathBuf>,
) -> ResolveResult<()> {
    let pattern_path = Path::new(pattern);
    let directory = pattern_path
        .parent()
        .map_or_else(|| system_dir.to_path_buf(), |path| system_dir.join(path));
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ResolveError::Io {
                path: directory,
                source,
            });
        }
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| has_package_magic(&entry.path()))
        .map(|entry| entry.path())
        .collect();
    files.sort();
    for path in files {
        if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
            paths.entry(name.to_ascii_lowercase()).or_insert(path);
        }
    }
    Ok(())
}

fn has_package_magic(path: &Path) -> bool {
    let mut magic = [0; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && magic == PACKAGE_MAGIC.to_le_bytes()
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error(transparent)]
    Package(#[from] crate::Error),

    #[error("failed to read `{}`", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("game root `{}` has no System directory", root.display())]
    MissingSystemDirectory { root: PathBuf },

    #[error("System directory `{}` has no Default.ini", system.display())]
    MissingDefaultIni { system: PathBuf },

    #[error("Default.ini has no [Core.System] Paths entries")]
    MissingPackagePaths,

    #[error("`{}` is not a valid package path", path.display())]
    InvalidPackagePath { path: PathBuf },

    #[error("could not find Unreal package `{name}` in the configured paths")]
    MissingPackage { name: String },

    #[error("import index {index} is outside the {import_count} imports")]
    InvalidImportIndex { index: usize, import_count: usize },

    #[error("import {import_index} has no package at the root of its outer chain")]
    ImportWithoutPackage { import_index: usize },

    #[error("import {import_index} has a cyclic outer chain")]
    OuterCycle { import_index: usize },

    #[error("package `{package}` has no {class} export named `{path}`")]
    MissingObject {
        package: String,
        class: String,
        path: String,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        Export, Import, NameEntry, ObjectReference, PackageHeader, PackageSummary,
        resolver::{core_system_paths, find_export},
    };

    #[test]
    fn reads_only_core_system_package_paths() {
        let ini = "\
            [Other]\n\
            Paths=wrong\n\
            [Core.System]\n\
            Paths=..\\System\\*.u\n\
            Paths=../Textures/*.utx\n";
        assert_eq!(
            core_system_paths(ini),
            ["../System/*.u", "../Textures/*.utx"]
        );
    }

    #[test]
    fn matches_export_class_name_and_outer_chain_case_insensitively() {
        let names = ["None", "Texture", "Walls", "Stone"]
            .into_iter()
            .map(|value| NameEntry {
                value: value.into(),
                flags: 0,
            })
            .collect();
        let summary = PackageSummary {
            source: Arc::from("test"),
            header: PackageHeader {
                version: 76,
                licensee_version: 0,
                package_flags: 0,
                name_count: 4,
                name_offset: 0,
                export_count: 2,
                export_offset: 0,
                import_count: 1,
                import_offset: 0,
                history: crate::HeaderHistory::Generations {
                    guid: [0; 16],
                    generations: Vec::new(),
                },
            },
            names,
            imports: vec![Import {
                class_package: 0,
                class_name: 0,
                outer: ObjectReference::None,
                object_name: 1,
            }],
            exports: vec![
                Export {
                    class: ObjectReference::None,
                    super_class: ObjectReference::None,
                    outer: ObjectReference::None,
                    object_name: 2,
                    object_flags: 0,
                    serial_size: 0,
                    serial_offset: None,
                },
                Export {
                    class: ObjectReference::Import(0),
                    super_class: ObjectReference::None,
                    outer: ObjectReference::Export(0),
                    object_name: 3,
                    object_flags: 0,
                    serial_size: 0,
                    serial_offset: None,
                },
            ],
        };
        assert_eq!(
            find_export(&summary, "texture", "stone", &["walls".into()]),
            Some(1)
        );
    }

    #[test]
    fn resolves_class_exports_with_no_serialized_meta_class() {
        let names = ["None", "LampPost"]
            .into_iter()
            .map(|value| NameEntry {
                value: value.into(),
                flags: 0,
            })
            .collect();
        let summary = PackageSummary {
            source: Arc::from("test"),
            header: PackageHeader {
                version: 76,
                licensee_version: 0,
                package_flags: 0,
                name_count: 2,
                name_offset: 0,
                export_count: 1,
                export_offset: 0,
                import_count: 0,
                import_offset: 0,
                history: crate::HeaderHistory::Generations {
                    guid: [0; 16],
                    generations: Vec::new(),
                },
            },
            names,
            imports: vec![],
            exports: vec![Export {
                class: ObjectReference::None,
                super_class: ObjectReference::None,
                outer: ObjectReference::None,
                object_name: 1,
                object_flags: 0,
                serial_size: 0,
                serial_offset: None,
            }],
        };
        assert_eq!(find_export(&summary, "Class", "lamppost", &[]), Some(0));
    }

    #[test]
    fn discovers_packages_by_magic_despite_localized_suffix() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openhp1-resolver-{unique}"));
        let system = root.join("System");
        let textures = root.join("Textures");
        fs::create_dir_all(&system).unwrap();
        fs::create_dir_all(&textures).unwrap();
        fs::write(
            system.join("Default.ini"),
            "[Core.System]\nPaths=../Textures/*.utx\n",
        )
        .unwrap();
        fs::write(
            textures.join("Localized.hun_utx"),
            crate::PACKAGE_MAGIC.to_le_bytes(),
        )
        .unwrap();

        let store = super::PackageStore::scan_game_root(&root).unwrap();
        assert_eq!(
            store
                .package_path("localized")
                .and_then(std::path::Path::file_name)
                .and_then(|name| name.to_str()),
            Some("Localized.hun_utx")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
