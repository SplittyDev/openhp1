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

pub use error::{Error, Result};
pub use object::{ObjectReader, PropertyKind, PropertyTag};
pub use package::{
    Export, Generation, HeaderHistory, Import, NameEntry, ObjectReference, PACKAGE_MAGIC, Package,
    PackageHeader, PackageSummary,
};
