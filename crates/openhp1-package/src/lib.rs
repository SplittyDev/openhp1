//! Read-only support for the Unreal Engine 1 package container used by HP1.
//!
//! Maps (`.unr`), textures (`.utx`), sounds (`.uax`), music (`.umx`), and
//! compiled script packages (`.u`) share this container. This crate parses the
//! common header and index tables without interpreting class-specific export
//! payloads.

mod archive;
mod error;
mod object;
mod package;
mod resolver;
mod summary;
mod tables;

pub use error::{Error, Result};
pub use object::{ObjectReader, ObjectStack, PropertyKind, PropertyTag};
pub use package::Package;
pub use resolver::{
    ConfigEntry, GameInstallation, GameInstallationError, PackageStore, ResolveError,
    ResolveResult, ResolvedObject, configure_game_installation, resolve_game_installation,
    settings_dir,
};
pub use summary::{
    Export, Generation, HeaderHistory, Import, NameEntry, ObjectReference, PackageHeader,
    PackageSummary,
};

pub const PACKAGE_MAGIC: u32 = 0x9e2a_83c1;
