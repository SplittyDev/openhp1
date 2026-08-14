use std::{
    collections::HashMap,
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{Export, ObjectReference, PACKAGE_MAGIC, Package, PackageSummary};

pub type ResolveResult<T> = std::result::Result<T, ResolveError>;

pub struct ResolvedObject {
    pub package: Arc<Package>,
    pub export_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigEntry {
    pub section: String,
    pub key: String,
    pub values: Vec<String>,
}

const OPENHP1_CONFIG: &str = "OpenHP1.ini";
const GAME_DATA_SECTION: &str = "OpenHP1.GameData";
const GAME_ROOT_KEY: &str = "Root";
const GAME_LANGUAGE_KEY: &str = "Language";

/// A validated original-game installation and its authored startup map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameInstallation {
    root: PathBuf,
    startup_map: PathBuf,
    language: String,
    available_languages: Vec<String>,
}

impl GameInstallation {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn startup_map(&self) -> &Path {
        &self.startup_map
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn available_languages(&self) -> &[String] {
        &self.available_languages
    }
}

/// Resolves the configured game root, or infers and remembers a conventional one.
pub fn resolve_game_installation() -> Result<GameInstallation, GameInstallationError> {
    let settings_dir = settings_dir();
    resolve_game_installation_with(&settings_dir, inferred_game_roots()?)
}

/// Validates and stores the selected game root in `OpenHP1.ini`.
pub fn configure_game_installation(
    root: impl AsRef<Path>,
    language: Option<&str>,
) -> Result<GameInstallation, GameInstallationError> {
    configure_game_installation_with_settings_dir(root.as_ref(), language, &settings_dir())
}

fn configure_game_installation_with_settings_dir(
    root: &Path,
    language: Option<&str>,
    settings_dir: &Path,
) -> Result<GameInstallation, GameInstallationError> {
    let installation = validate_game_root(root, settings_dir, language)?;
    write_game_installation(settings_dir, &installation)?;
    Ok(installation)
}

fn resolve_game_installation_with(
    settings_dir: &Path,
    inferred_roots: impl IntoIterator<Item = PathBuf>,
) -> Result<GameInstallation, GameInstallationError> {
    let (configured_root, configured_language) = configured_game_data(settings_dir)?;
    if let Some(root) = configured_root {
        let installation = validate_game_root(&root, settings_dir, configured_language.as_deref())?;
        write_game_installation(settings_dir, &installation)?;
        return Ok(installation);
    }
    for root in inferred_roots {
        if let Ok(installation) = validate_game_root(&root, settings_dir, None) {
            write_game_installation(settings_dir, &installation)?;
            return Ok(installation);
        }
    }
    Err(GameInstallationError::NotConfigured)
}

fn configured_game_data(
    settings_dir: &Path,
) -> Result<(Option<PathBuf>, Option<String>), GameInstallationError> {
    let path = settings_dir.join(OPENHP1_CONFIG);
    let Some(contents) = read_openhp1_ini(settings_dir)
        .map_err(|source| GameInstallationError::Io { path, source })?
    else {
        return Ok((None, None));
    };
    let root = ini_values(&contents, GAME_DATA_SECTION, GAME_ROOT_KEY)
        .into_iter()
        .find(|value| !value.is_empty())
        .map(PathBuf::from);
    let language = ini_values(&contents, GAME_DATA_SECTION, GAME_LANGUAGE_KEY)
        .into_iter()
        .find(|value| !value.is_empty());
    Ok((root, language))
}

fn validate_game_root(
    root: &Path,
    settings_dir: &Path,
    language: Option<&str>,
) -> Result<GameInstallation, GameInstallationError> {
    if !root.is_absolute() {
        return Err(GameInstallationError::InvalidRoot {
            root: root.to_path_buf(),
            reason: "OpenHP1.ini requires an absolute path".to_owned(),
        });
    }
    let root = root.to_path_buf();
    let packages = PackageStore::scan_game_root_with_language(&root, settings_dir, language)
        .map_err(|source| match source {
            ResolveError::UnavailableLanguage {
                language,
                available,
            } => GameInstallationError::InvalidLanguage {
                root: root.clone(),
                language,
                available,
            },
            source => GameInstallationError::InvalidRoot {
                root: root.clone(),
                reason: source.to_string(),
            },
        })?;
    let local_map = packages.config_value("URL", "LocalMap").ok_or_else(|| {
        GameInstallationError::InvalidRoot {
            root: root.clone(),
            reason: "Default.ini has no [URL] LocalMap entry".to_owned(),
        }
    })?;
    let package = Path::new(&local_map)
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| GameInstallationError::InvalidRoot {
            root: root.clone(),
            reason: format!("LocalMap `{local_map}` is not a valid package name"),
        })?;
    let startup_map =
        packages
            .package_path(package)
            .ok_or_else(|| GameInstallationError::InvalidRoot {
                root: root.clone(),
                reason: format!("could not find configured startup map `{local_map}`"),
            })?;
    Ok(GameInstallation {
        root,
        startup_map: startup_map.to_path_buf(),
        language: packages.language.clone(),
        available_languages: packages.available_languages.clone(),
    })
}

fn write_game_installation(
    settings_dir: &Path,
    installation: &GameInstallation,
) -> Result<(), GameInstallationError> {
    let root = installation
        .root()
        .to_str()
        .filter(|value| !value.contains(['\n', '\r']))
        .ok_or_else(|| GameInstallationError::InvalidRoot {
            root: installation.root().to_path_buf(),
            reason: "the path cannot be represented in OpenHP1.ini".to_owned(),
        })?;
    let path = settings_dir.join(OPENHP1_CONFIG);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(GameInstallationError::Io { path, source }),
    };
    write_ini_atomically(
        &path,
        update_ini(
            &contents,
            &[
                ConfigEntry {
                    section: GAME_DATA_SECTION.to_owned(),
                    key: GAME_ROOT_KEY.to_owned(),
                    values: vec![root.to_owned()],
                },
                ConfigEntry {
                    section: GAME_DATA_SECTION.to_owned(),
                    key: GAME_LANGUAGE_KEY.to_owned(),
                    values: vec![installation.language().to_owned()],
                },
            ],
        ),
    )
    .map_err(GameInstallationError::Settings)
}

fn read_openhp1_ini(settings_dir: &Path) -> std::io::Result<Option<String>> {
    match fs::read_to_string(settings_dir.join(OPENHP1_CONFIG)) {
        Ok(contents) => Ok(Some(contents)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(source),
    }
}

fn inferred_game_roots() -> Result<Vec<PathBuf>, GameInstallationError> {
    let current_directory = env::current_dir().map_err(|source| GameInstallationError::Io {
        path: PathBuf::from("current directory"),
        source,
    })?;
    let mut roots = vec![current_directory.join("res")];
    let executable = env::current_exe().map_err(|source| GameInstallationError::Io {
        path: PathBuf::from("current executable"),
        source,
    })?;
    if let Some(directory) = executable.parent() {
        roots.push(directory.join("res"));
    }
    #[cfg(target_os = "windows")]
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(directory) = env::var_os(variable) {
            roots.push(
                PathBuf::from(directory)
                    .join("EA Games")
                    .join("Harry Potter TM"),
            );
        }
    }
    Ok(roots)
}

/// Discovers packages through `[Core.System] Paths` and caches them by their
/// case-insensitive Unreal package name.
pub struct PackageStore {
    paths: HashMap<String, PathBuf>,
    localized_paths: HashMap<String, PathBuf>,
    loaded: HashMap<String, Arc<Package>>,
    localized_loaded: HashMap<String, Arc<Package>>,
    system_dir: PathBuf,
    settings_dir: PathBuf,
    default_ini: PathBuf,
    language: String,
    available_languages: Vec<String>,
}

impl PackageStore {
    pub fn scan_game_root(root: impl AsRef<Path>) -> ResolveResult<Self> {
        Self::scan_game_root_with_settings_dir(root, settings_dir())
    }

    pub fn scan_game_root_with_settings_dir(
        root: impl AsRef<Path>,
        settings_dir: impl AsRef<Path>,
    ) -> ResolveResult<Self> {
        let settings_dir = settings_dir.as_ref();
        let path = settings_dir.join(OPENHP1_CONFIG);
        let language = read_openhp1_ini(settings_dir)
            .map_err(|source| ResolveError::Io { path, source })?
            .and_then(|contents| {
                ini_values(&contents, GAME_DATA_SECTION, GAME_LANGUAGE_KEY)
                    .into_iter()
                    .find(|value| !value.is_empty())
            });
        Self::scan_game_root_with_language(root.as_ref(), settings_dir, language.as_deref())
    }

    fn scan_game_root_with_language(
        root: &Path,
        settings_dir: &Path,
        language: Option<&str>,
    ) -> ResolveResult<Self> {
        let system_dir = find_child_directory(root, "System").ok_or_else(|| {
            ResolveError::MissingSystemDirectory {
                root: root.to_path_buf(),
            }
        })?;
        let languages = discover_game_languages(&system_dir)?;
        let selected = if let Some(language) = language {
            languages
                .iter()
                .find(|candidate| candidate.code.eq_ignore_ascii_case(language))
                .ok_or_else(|| ResolveError::UnavailableLanguage {
                    language: language.to_owned(),
                    available: languages
                        .iter()
                        .map(|candidate| candidate.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                })?
        } else {
            languages
                .first()
                .ok_or_else(|| ResolveError::MissingDefaultIni {
                    system: system_dir.clone(),
                })?
        };
        let ini_path = selected.default_ini.clone();
        let ini = fs::read_to_string(&ini_path).map_err(|source| ResolveError::Io {
            path: ini_path.clone(),
            source,
        })?;
        let patterns = core_system_paths(&ini);
        if patterns.is_empty() {
            return Err(ResolveError::MissingPackagePaths);
        }

        let mut paths = HashMap::new();
        let mut localized_paths = HashMap::new();
        let language = selected.code.clone();
        let available_languages = languages
            .into_iter()
            .map(|language| language.code)
            .collect();
        let language_directory = ini_path
            .parent()
            .filter(|parent| *parent != system_dir)
            .and_then(Path::file_name);
        for pattern in patterns {
            scan_pattern(
                &system_dir,
                &pattern,
                language_directory,
                &mut paths,
                &mut localized_paths,
            )?;
        }
        Ok(Self {
            paths,
            localized_paths,
            loaded: HashMap::new(),
            localized_loaded: HashMap::new(),
            system_dir,
            settings_dir: settings_dir.to_path_buf(),
            default_ini: ini_path,
            language,
            available_languages,
        })
    }

    pub fn config_value(&self, section: &str, key: &str) -> Option<String> {
        self.config_values("System", section, key)
            .into_iter()
            .next()
    }

    pub fn config_values(&self, config_name: &str, section: &str, key: &str) -> Vec<String> {
        let Ok(files) = self.config_files(config_name) else {
            return Vec::new();
        };
        for path in std::iter::once(&files.destination).chain(files.fallbacks.iter()) {
            let Ok(contents) = fs::read_to_string(path) else {
                continue;
            };
            let values = ini_values(&contents, section, key);
            if !values.is_empty() {
                return values;
            }
        }
        Vec::new()
    }

    /// Writes only derived user configuration files. Package files and INI
    /// templates remain read-only.
    pub fn save_config(&self, config_name: &str, entries: &[ConfigEntry]) -> ResolveResult<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let files = self.config_files(config_name)?;
        let mut contents = None;
        for path in std::iter::once(&files.destination).chain(files.fallbacks.iter()) {
            match fs::read_to_string(path) {
                Ok(value) => {
                    contents = Some(value);
                    break;
                }
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(ResolveError::Io {
                        path: path.to_path_buf(),
                        source,
                    });
                }
            }
        }
        write_ini_atomically(
            &files.destination,
            update_ini(&contents.unwrap_or_default(), entries),
        )
    }

    pub fn remove_config_section(&self, config_name: &str, section: &str) -> ResolveResult<()> {
        let destination = self.config_files(config_name)?.destination;
        let contents = match fs::read_to_string(&destination) {
            Ok(contents) => contents,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ResolveError::Io {
                    path: destination,
                    source,
                });
            }
        };
        let Some(contents) = remove_ini_section(&contents, section) else {
            return Ok(());
        };
        write_ini_atomically(&destination, contents)
    }

    pub fn package_path(&self, name: &str) -> Option<&Path> {
        self.paths
            .get(&name.to_ascii_lowercase())
            .map(PathBuf::as_path)
    }

    pub fn package_paths(&self) -> impl Iterator<Item = &Path> {
        self.paths.values().map(PathBuf::as_path)
    }

    /// Directory owned by OpenHP1 for derived user data. Installed packages and
    /// INI templates are never written through this path.
    pub fn settings_dir(&self) -> &Path {
        &self.settings_dir
    }

    pub fn localize(&self, package: &str, section: &str, key: &str) -> String {
        let language_file = format!("{package}.{}", self.language);
        let selected_directory = self
            .default_ini
            .parent()
            .filter(|directory| *directory != self.system_dir);
        selected_directory
            .into_iter()
            .chain(std::iter::once(self.system_dir.as_path()))
            .find_map(|directory| find_file(directory, &language_file))
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| localization_value(&String::from_utf8_lossy(&bytes), section, key))
            .unwrap_or_default()
    }

    pub fn find_object(
        &mut self,
        qualified_name: &str,
        class: &str,
    ) -> ResolveResult<Option<ResolvedObject>> {
        self.find_object_matching(qualified_name, Some(class))
    }

    /// Resolves an object from the selected-language overlay, falling back to
    /// the base package when the overlay does not replace that object.
    pub fn find_localized_object(
        &mut self,
        qualified_name: &str,
        class: &str,
    ) -> ResolveResult<Option<ResolvedObject>> {
        if let Some(object) = self.find_object_matching_source(qualified_name, Some(class), true)? {
            return Ok(Some(object));
        }
        self.find_object_matching(qualified_name, Some(class))
    }

    /// Resolves a package object by its case-insensitive qualified path.
    /// Callers that require a particular object class should use [`Self::find_object`].
    pub fn find_object_any(
        &mut self,
        qualified_name: &str,
    ) -> ResolveResult<Option<ResolvedObject>> {
        self.find_object_matching(qualified_name, None)
    }

    /// Returns a qualified object path accepted by [`Self::find_object_any`].
    pub fn qualified_object_name(object: &ResolvedObject) -> ResolveResult<String> {
        let summary = object.package.summary();
        let package = Path::new(summary.source.as_ref())
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ResolveError::InvalidPackagePath {
                path: PathBuf::from(summary.source.as_ref()),
            })?;
        let export = summary.exports.get(object.export_index).ok_or_else(|| {
            crate::Error::InvalidExportIndex {
                package: summary.source.clone(),
                index: object.export_index,
                export_count: summary.exports.len(),
            }
        })?;
        let mut path = export_groups(summary, export).ok_or(ResolveError::InvalidObjectPath {
            package: package.to_owned(),
            export_index: object.export_index,
        })?;
        path.reverse();
        path.push(summary.name(export.object_name).to_owned());
        Ok(std::iter::once(package.to_owned())
            .chain(path)
            .collect::<Vec<_>>()
            .join("."))
    }

    fn find_object_matching(
        &mut self,
        qualified_name: &str,
        class: Option<&str>,
    ) -> ResolveResult<Option<ResolvedObject>> {
        self.find_object_matching_source(qualified_name, class, false)
    }

    fn find_object_matching_source(
        &mut self,
        qualified_name: &str,
        class: Option<&str>,
        localized: bool,
    ) -> ResolveResult<Option<ResolvedObject>> {
        let mut parts = qualified_name.split('.');
        let Some(package_name) = parts.next().filter(|part| !part.is_empty()) else {
            return Ok(None);
        };
        let mut path = parts.map(str::to_owned).collect::<Vec<_>>();
        let Some(object) = path.pop() else {
            return Ok(None);
        };
        path.reverse();
        let package = if localized {
            let Some(package) = self.load_localized(package_name)? else {
                return Ok(None);
            };
            package
        } else {
            self.load(package_name)?
        };
        Ok((match class {
            Some(class) => find_export(package.summary(), class, &object, &path),
            None => find_export_any(package.summary(), &object, &path),
        })
        .map(|export_index| ResolvedObject {
            package,
            export_index,
        }))
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

    fn load_localized(&mut self, name: &str) -> ResolveResult<Option<Arc<Package>>> {
        let key = name.to_ascii_lowercase();
        if let Some(package) = self.localized_loaded.get(&key) {
            return Ok(Some(Arc::clone(package)));
        }
        let Some(path) = self.localized_paths.get(&key) else {
            return Ok(None);
        };
        let package = Arc::new(Package::open(path)?);
        self.localized_loaded.insert(key, Arc::clone(&package));
        Ok(Some(package))
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
                let export_index =
                    find_import_export(package.summary(), &target).ok_or_else(|| {
                        ResolveError::MissingObject {
                            package: target.package,
                            class: target.class,
                            path: target
                                .groups
                                .iter()
                                .map(|group| group.name.clone())
                                .chain(std::iter::once(target.object.clone()))
                                .collect::<Vec<_>>()
                                .join("."),
                        }
                    })?;
                Ok(Some(ResolvedObject {
                    package,
                    export_index,
                }))
            }
        }
    }

    fn config_files(&self, config_name: &str) -> ResolveResult<ConfigFiles> {
        if config_name.is_empty() || config_name.eq_ignore_ascii_case("System") {
            let stem = system_ini_stem(&self.system_dir).unwrap_or_else(|| "OpenHP1".to_owned());
            return Ok(ConfigFiles {
                destination: self.settings_dir.join(format!("{stem}.ini")),
                fallbacks: vec![
                    self.system_dir.join(format!("{stem}.ini")),
                    self.default_ini.clone(),
                ],
            });
        }
        if config_name.eq_ignore_ascii_case("User") {
            let mut fallbacks = vec![self.system_dir.join("User.ini")];
            if let Some(template) = find_file(&self.system_dir, "DefUser.ini") {
                fallbacks.push(template);
            }
            return Ok(ConfigFiles {
                destination: self.settings_dir.join("User.ini"),
                fallbacks,
            });
        }
        if !config_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(ResolveError::InvalidConfigName {
                name: config_name.to_owned(),
            });
        }
        Ok(ConfigFiles {
            destination: self.settings_dir.join(format!("{config_name}.ini")),
            fallbacks: vec![self.system_dir.join(format!("{config_name}.ini"))],
        })
    }
}

struct ConfigFiles {
    destination: PathBuf,
    fallbacks: Vec<PathBuf>,
}

/// Returns the platform-specific directory for OpenHP1 settings and user data.
pub fn settings_dir() -> PathBuf {
    if let Some(path) = env::var_os("OPENHP1_SETTINGS_DIR") {
        return PathBuf::from(path);
    }

    #[cfg(target_os = "windows")]
    if let Some(path) = env::var_os("APPDATA") {
        return PathBuf::from(path).join("OpenHP1");
    }

    let Some(home) = env::var_os("HOME") else {
        return env::temp_dir().join("OpenHP1");
    };

    #[cfg(target_os = "macos")]
    {
        PathBuf::from(home).join("Library/Application Support/OpenHP1")
    }
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(home).join("AppData/Roaming/OpenHP1")
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(path).join("openhp1")
    } else {
        PathBuf::from(home).join(".config/openhp1")
    }
}

struct ImportTarget {
    package: String,
    groups: Vec<ImportGroup>,
    object: String,
    class: String,
}

struct ImportGroup {
    name: String,
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
                let group = ImportGroup {
                    name: summary.name(entry.object_name).to_owned(),
                    class: summary.name(entry.class_name).to_owned(),
                };
                if entry.outer == ObjectReference::None {
                    return Ok(ImportTarget {
                        package: group.name,
                        groups,
                        object,
                        class,
                    });
                }
                groups.push(group);
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
    summary
        .exports
        .iter()
        .position(|export| export_matches(summary, export, class, object, groups))
}

fn find_export_any(summary: &PackageSummary, object: &str, groups: &[String]) -> Option<usize> {
    let mut matches = summary
        .exports
        .iter()
        .enumerate()
        .filter_map(|(index, export)| {
            (summary
                .name(export.object_name)
                .eq_ignore_ascii_case(object)
                && export_groups(summary, export)
                    .is_some_and(|actual| equal_names(&actual, groups)))
            .then_some(index)
        });
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn find_import_export(summary: &PackageSummary, target: &ImportTarget) -> Option<usize> {
    let groups = target
        .groups
        .iter()
        .map(|group| group.name.clone())
        .collect::<Vec<_>>();
    find_export(summary, &target.class, &target.object, &groups).or_else(|| {
        let mut matches = summary
            .exports
            .iter()
            .enumerate()
            .filter_map(|(index, export)| {
                let actual = export_groups(summary, export)?;
                // HP1 imports can include parent Package objects the target package omits.
                (actual.len() < groups.len()
                    && equal_names(&actual, &groups[..actual.len()])
                    && target.groups[actual.len()..]
                        .iter()
                        .all(|group| group.class.eq_ignore_ascii_case("Package"))
                    && export_matches(summary, export, &target.class, &target.object, &actual))
                .then_some(index)
            });
        let export_index = matches.next()?;
        matches.next().is_none().then_some(export_index)
    })
}

fn export_matches(
    summary: &PackageSummary,
    export: &Export,
    class: &str,
    object: &str,
    groups: &[String],
) -> bool {
    summary
        .name(export.object_name)
        .eq_ignore_ascii_case(object)
        && (summary
            .class_name(export)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(class))
            || (class.eq_ignore_ascii_case("Class") && export.class == ObjectReference::None))
        && export_groups(summary, export).is_some_and(|actual| equal_names(&actual, groups))
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

struct GameLanguageConfig {
    code: String,
    default_ini: PathBuf,
}

fn discover_game_languages(system_dir: &Path) -> ResolveResult<Vec<GameLanguageConfig>> {
    let mut default_inis = Vec::new();
    if let Some(default_ini) = find_file(system_dir, "Default.ini") {
        default_inis.push(default_ini);
    }
    let mut directories: Vec<_> = fs::read_dir(system_dir)
        .map_err(|source| ResolveError::Io {
            path: system_dir.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    directories.sort();
    default_inis.extend(
        directories
            .into_iter()
            .filter_map(|directory| find_file(&directory, "Default.ini")),
    );
    if default_inis.is_empty() {
        return Err(ResolveError::MissingDefaultIni {
            system: system_dir.to_path_buf(),
        });
    }

    let mut languages = Vec::new();
    for default_ini in default_inis {
        let contents = fs::read_to_string(&default_ini).map_err(|source| ResolveError::Io {
            path: default_ini.clone(),
            source,
        })?;
        let code = localization_value(&contents, "Engine.Engine", "Language")
            .unwrap_or_else(|| "int".to_owned())
            .to_ascii_lowercase();
        if code.is_empty()
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(ResolveError::InvalidLanguageCode {
                path: default_ini,
                language: code,
            });
        }
        if languages
            .iter()
            .any(|language: &GameLanguageConfig| language.code.eq_ignore_ascii_case(&code))
        {
            continue;
        }
        languages.push(GameLanguageConfig { code, default_ini });
    }
    Ok(languages)
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

fn localization_value(contents: &str, section: &str, key: &str) -> Option<String> {
    ini_values(contents, section, key).into_iter().next()
}

fn ini_values(contents: &str, section: &str, key: &str) -> Vec<String> {
    let mut in_section = false;
    let mut values = Vec::new();
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line[1..line.len() - 1].eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section || line.starts_with(';') {
            continue;
        }
        let Some((candidate, value)) = line.split_once('=') else {
            continue;
        };
        if !candidate.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let value = value.trim();
        values.push(
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value)
                .to_owned(),
        );
    }
    values
}

fn system_ini_stem(system_dir: &Path) -> Option<String> {
    let mut executables = fs::read_dir(system_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                .then(|| {
                    entry
                        .path()
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_owned)
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    executables.sort_by_key(|name| name.to_ascii_lowercase());
    executables.into_iter().next()
}

fn update_ini(contents: &str, entries: &[ConfigEntry]) -> String {
    let mut current_section = "";
    let mut written = vec![false; entries.len()];
    let mut lines = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            append_missing_ini_entries(&mut lines, current_section, entries, &mut written);
            current_section = &trimmed[1..trimmed.len() - 1];
            lines.push(line.to_owned());
            continue;
        }
        let Some((key, _)) = trimmed.split_once('=') else {
            lines.push(line.to_owned());
            continue;
        };
        let Some(index) = entries.iter().position(|entry| {
            entry.section.eq_ignore_ascii_case(current_section)
                && entry.key.eq_ignore_ascii_case(key.trim())
        }) else {
            lines.push(line.to_owned());
            continue;
        };
        if !written[index] {
            lines.extend(
                entries[index]
                    .values
                    .iter()
                    .map(|value| format!("{}={value}", entries[index].key)),
            );
            written[index] = true;
        }
    }
    append_missing_ini_entries(&mut lines, current_section, entries, &mut written);

    let mut appended_sections = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if written[index] {
            continue;
        }
        written[index] = true;
        if entry.values.is_empty() {
            continue;
        }
        if !appended_sections
            .iter()
            .any(|section: &String| section.eq_ignore_ascii_case(&entry.section))
        {
            if !lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{}]", entry.section));
            appended_sections.push(entry.section.clone());
        }
        lines.extend(
            entry
                .values
                .iter()
                .map(|value| format!("{}={value}", entry.key)),
        );
    }
    let mut updated = lines.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    updated
}

fn remove_ini_section(contents: &str, section: &str) -> Option<String> {
    let mut found = false;
    let mut removing = false;
    let mut lines = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            removing = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
            found |= removing;
        }
        if !removing {
            lines.push(line);
        }
    }
    found.then(|| {
        let mut updated = lines.join("\n");
        if !updated.is_empty() {
            updated.push('\n');
        }
        updated
    })
}

fn append_missing_ini_entries(
    lines: &mut Vec<String>,
    section: &str,
    entries: &[ConfigEntry],
    written: &mut [bool],
) {
    for (index, entry) in entries.iter().enumerate() {
        if written[index] || !entry.section.eq_ignore_ascii_case(section) {
            continue;
        }
        written[index] = true;
        lines.extend(
            entry
                .values
                .iter()
                .map(|value| format!("{}={value}", entry.key)),
        );
    }
}

fn write_ini_atomically(path: &Path, contents: String) -> ResolveResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ResolveError::InvalidConfigPath {
            path: path.to_path_buf(),
        })?;
    fs::create_dir_all(parent).map_err(|source| ResolveError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ResolveError::InvalidConfigPath {
            path: path.to_path_buf(),
        })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), nonce));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| ResolveError::Io {
            path: temporary.clone(),
            source,
        })?;
    let write_result = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(source) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(ResolveError::Io {
            path: temporary,
            source,
        });
    }
    if let Err(source) = rename_atomically(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(ResolveError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn rename_atomically(temporary: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(temporary, path)
}

#[cfg(target_os = "windows")]
fn rename_atomically(temporary: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    if unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn scan_pattern(
    system_dir: &Path,
    pattern: &str,
    language_directory: Option<&std::ffi::OsStr>,
    paths: &mut HashMap<String, PathBuf>,
    localized_paths: &mut HashMap<String, PathBuf>,
) -> ResolveResult<()> {
    let pattern_path = Path::new(pattern);
    let directory = pattern_path
        .parent()
        .map_or_else(|| system_dir.to_path_buf(), |path| system_dir.join(path));
    scan_package_directory(&directory, paths)?;
    if let Some(language_directory) = language_directory {
        scan_package_directory(&directory.join(language_directory), localized_paths)?;
    }
    Ok(())
}

fn scan_package_directory(
    directory: &Path,
    paths: &mut HashMap<String, PathBuf>,
) -> ResolveResult<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ResolveError::Io {
                path: directory.to_path_buf(),
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

    #[error("Default.ini `{}` has invalid language code `{language}`", path.display())]
    InvalidLanguageCode { path: PathBuf, language: String },

    #[error("language `{language}` is not available; available languages: {available}")]
    UnavailableLanguage { language: String, available: String },

    #[error("Default.ini has no [Core.System] Paths entries")]
    MissingPackagePaths,

    #[error("`{}` is not a valid package path", path.display())]
    InvalidPackagePath { path: PathBuf },

    #[error("`{name}` is not a valid Unreal configuration name")]
    InvalidConfigName { name: String },

    #[error("`{}` is not a valid configuration path", path.display())]
    InvalidConfigPath { path: PathBuf },

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

    #[error("package `{package}` export {export_index} has an invalid outer path")]
    InvalidObjectPath {
        package: String,
        export_index: usize,
    },
}

#[derive(Debug, Error)]
pub enum GameInstallationError {
    #[error(
        "OpenHP1 could not find the original game files; configure the folder containing `Maps` and `System`"
    )]
    NotConfigured,

    #[error("game root `{}` is invalid: {reason}", root.display())]
    InvalidRoot { root: PathBuf, reason: String },

    #[error(
        "language `{language}` is not available in `{}`; available languages: {available}",
        root.display()
    )]
    InvalidLanguage {
        root: PathBuf,
        language: String,
        available: String,
    },

    #[error("failed to read `{}`: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to update OpenHP1.ini: {0}")]
    Settings(#[source] ResolveError),
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
        resolver::{
            GameInstallationError, configure_game_installation_with_settings_dir,
            core_system_paths, find_export, find_import_export, import_target, localization_value,
            remove_ini_section, resolve_game_installation_with,
        },
    };

    fn game_installation_fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture = std::env::temp_dir().join(format!("openhp1-{name}-{unique}"));
        let game_root = fixture.join("game");
        let settings_dir = fixture.join("settings");
        fs::create_dir_all(game_root.join("System")).unwrap();
        fs::create_dir_all(game_root.join("Maps")).unwrap();
        fs::create_dir_all(&settings_dir).unwrap();
        fs::write(
            game_root.join("System/Default.ini"),
            "[URL]\nLocalMap=Startup.unr\n[Engine.Engine]\nLanguage=int\n[Core.System]\nPaths=../Maps/*.unr\n",
        )
        .unwrap();
        fs::write(
            game_root.join("Maps/STARTUP.UNR"),
            crate::PACKAGE_MAGIC.to_le_bytes(),
        )
        .unwrap();
        (game_root, settings_dir)
    }

    #[test]
    fn configures_and_resolves_the_game_root_through_openhp1_ini() {
        let (game_root, settings_dir) = game_installation_fixture("game-root");
        fs::write(settings_dir.join("OpenHP1.ini"), "[Keep]\nValue=1\n").unwrap();

        let configured =
            configure_game_installation_with_settings_dir(&game_root, None, &settings_dir).unwrap();
        let resolved = resolve_game_installation_with(&settings_dir, []).unwrap();

        assert_eq!(resolved, configured);
        assert_eq!(resolved.root(), game_root);
        assert_eq!(resolved.startup_map().file_name().unwrap(), "STARTUP.UNR");
        assert_eq!(resolved.language(), "int");
        assert_eq!(resolved.available_languages(), ["int"]);
        let ini = fs::read_to_string(settings_dir.join("OpenHP1.ini")).unwrap();
        assert!(ini.contains("[Keep]\nValue=1"));
        assert!(ini.contains("[OpenHP1.GameData]"));
        assert!(ini.contains(&format!("Root={}", resolved.root().display())));
        assert!(ini.contains("Language=int"));
        fs::remove_dir_all(game_root.parent().unwrap()).unwrap();
    }

    #[test]
    fn selects_and_remembers_a_language_shipped_in_a_subdirectory() {
        let (game_root, settings_dir) = game_installation_fixture("game-language");
        fs::remove_file(game_root.join("System/Default.ini")).unwrap();
        for (directory, language) in [("0", "eng"), ("1", "fre")] {
            let directory = game_root.join("System").join(directory);
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("Default.ini"),
                format!(
                    "[URL]\nLocalMap=Startup.unr\n[Engine.Engine]\nLanguage={language}\n[Core.System]\nPaths=../Maps/*.unr\n"
                ),
            )
            .unwrap();
        }

        let configured =
            configure_game_installation_with_settings_dir(&game_root, Some("FRE"), &settings_dir)
                .unwrap();
        let resolved = resolve_game_installation_with(&settings_dir, []).unwrap();

        assert_eq!(configured.language(), "fre");
        assert_eq!(configured.available_languages(), ["eng", "fre"]);
        assert_eq!(resolved, configured);
        assert!(
            fs::read_to_string(settings_dir.join("OpenHP1.ini"))
                .unwrap()
                .contains("Language=fre")
        );
        fs::remove_dir_all(game_root.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_a_language_not_shipped_with_the_game_root() {
        let (game_root, settings_dir) = game_installation_fixture("missing-language");

        let error =
            configure_game_installation_with_settings_dir(&game_root, Some("fre"), &settings_dir)
                .unwrap_err();

        assert!(matches!(
            error,
            GameInstallationError::InvalidLanguage { language, available, .. }
                if language == "fre" && available == "int"
        ));
        fs::remove_dir_all(game_root.parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_configured_root_does_not_fall_back_to_an_inferred_root() {
        let (game_root, settings_dir) = game_installation_fixture("bad-game-root");
        let missing = game_root.parent().unwrap().join("missing");
        fs::write(
            settings_dir.join("OpenHP1.ini"),
            format!("[OpenHP1.GameData]\nRoot={}\n", missing.display()),
        )
        .unwrap();

        let error = resolve_game_installation_with(&settings_dir, [game_root.clone()]).unwrap_err();
        assert!(
            matches!(error, GameInstallationError::InvalidRoot { root, .. } if root == missing)
        );
        fs::remove_dir_all(game_root.parent().unwrap()).unwrap();
    }

    #[test]
    fn infers_and_remembers_a_valid_game_root_when_unconfigured() {
        let (game_root, settings_dir) = game_installation_fixture("inferred-game-root");

        let resolved = resolve_game_installation_with(&settings_dir, [game_root.clone()]).unwrap();

        assert_eq!(resolved.root(), game_root);
        assert!(
            fs::read_to_string(settings_dir.join("OpenHP1.ini"))
                .unwrap()
                .contains(&format!("Root={}", game_root.display()))
        );
        fs::remove_dir_all(game_root.parent().unwrap()).unwrap();
    }

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
    fn reads_case_insensitive_localization_values() {
        let contents = "[all]\nGreeting=\"Welcome\"\n";
        assert_eq!(
            localization_value(contents, "ALL", "greeting"),
            Some("Welcome".to_owned())
        );
        assert_eq!(localization_value(contents, "all", "missing"), None);
    }

    #[test]
    fn removes_one_ini_section_case_insensitively() {
        let contents =
            "[Keep]\nValue=1\n\n[OpenHP1.Graphics]\nRenderer=classic\n\n[Later]\nValue=2\n";
        assert_eq!(
            remove_ini_section(contents, "openhp1.graphics"),
            Some("[Keep]\nValue=1\n\n[Later]\nValue=2\n".to_owned())
        );
        assert_eq!(remove_ini_section(contents, "Missing"), None);
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
    fn resolves_unique_sound_export_beneath_a_missing_parent_package() {
        let names: Vec<_> = [
            "None",
            "Package",
            "Sound",
            "HPSounds",
            "Hub3_sfx",
            "Hub5_sfx",
            "Vold_Pillar_Thump_06",
        ]
        .into_iter()
        .map(|value| NameEntry {
            value: value.into(),
            flags: 0,
        })
        .collect();
        let header = PackageHeader {
            version: 76,
            licensee_version: 0,
            package_flags: 0,
            name_count: 7,
            name_offset: 0,
            export_count: 0,
            export_offset: 0,
            import_count: 4,
            import_offset: 0,
            history: crate::HeaderHistory::Generations {
                guid: [0; 16],
                generations: Vec::new(),
            },
        };
        let source = PackageSummary {
            source: Arc::from("Lev3_Troll"),
            header: header.clone(),
            names: names.clone(),
            imports: vec![
                Import {
                    class_package: 0,
                    class_name: 1,
                    outer: ObjectReference::None,
                    object_name: 3,
                },
                Import {
                    class_package: 0,
                    class_name: 1,
                    outer: ObjectReference::Import(0),
                    object_name: 4,
                },
                Import {
                    class_package: 0,
                    class_name: 1,
                    outer: ObjectReference::Import(1),
                    object_name: 5,
                },
                Import {
                    class_package: 0,
                    class_name: 2,
                    outer: ObjectReference::Import(2),
                    object_name: 6,
                },
            ],
            exports: vec![],
        };
        let target = PackageSummary {
            source: Arc::from("HPSounds"),
            header,
            names,
            imports: vec![Import {
                class_package: 0,
                class_name: 0,
                outer: ObjectReference::None,
                object_name: 2,
            }],
            exports: vec![
                Export {
                    class: ObjectReference::None,
                    super_class: ObjectReference::None,
                    outer: ObjectReference::None,
                    object_name: 5,
                    object_flags: 0,
                    serial_size: 0,
                    serial_offset: None,
                },
                Export {
                    class: ObjectReference::Import(0),
                    super_class: ObjectReference::None,
                    outer: ObjectReference::Export(0),
                    object_name: 6,
                    object_flags: 0,
                    serial_size: 0,
                    serial_offset: None,
                },
            ],
        };

        let import = import_target(&source, 3).unwrap();
        assert_eq!(find_import_export(&target, &import), Some(1));
    }

    #[test]
    fn keeps_base_packages_and_selected_language_overlays() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openhp1-resolver-{unique}"));
        let system = root.join("System");
        let textures = root.join("Textures");
        fs::create_dir_all(system.join("0")).unwrap();
        fs::create_dir_all(&textures).unwrap();
        fs::create_dir_all(textures.join("0")).unwrap();
        fs::write(
            system.join("0/Default.ini"),
            "[Engine.Engine]\nLanguage=int\n[Core.System]\nPaths=../Textures/*.utx\n",
        )
        .unwrap();
        fs::write(
            textures.join("Localized.utx"),
            crate::PACKAGE_MAGIC.to_le_bytes(),
        )
        .unwrap();
        fs::write(
            textures.join("0/Localized.int_utx"),
            crate::PACKAGE_MAGIC.to_le_bytes(),
        )
        .unwrap();
        fs::write(system.join("0/Pickup.int"), "[all]\nGreeting=Welcome\n").unwrap();

        let store = super::PackageStore::scan_game_root(&root).unwrap();
        assert_eq!(
            store
                .package_path("localized")
                .and_then(std::path::Path::file_name)
                .and_then(|name| name.to_str()),
            Some("Localized.utx")
        );
        assert_eq!(
            store
                .localized_paths
                .get("localized")
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("Localized.int_utx")
        );
        assert_eq!(store.localize("Pickup", "all", "Greeting"), "Welcome");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_write_removes_its_temporary_file_when_replacement_fails() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openhp1-config-write-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("User.ini");
        fs::create_dir(&destination).unwrap();

        assert!(super::write_ini_atomically(&destination, "Value=1\n".to_owned()).is_err());
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp")
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
