use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageHeader {
    pub version: u16,
    pub licensee_version: u16,
    pub package_flags: u32,
    pub name_count: usize,
    pub name_offset: usize,
    pub export_count: usize,
    pub export_offset: usize,
    pub import_count: usize,
    pub import_offset: usize,
    pub history: HeaderHistory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeaderHistory {
    /// Package versions before 68 store a list of heritage GUIDs.
    Heritage { count: usize, offset: usize },
    /// Version 68 introduced a package GUID and generation summaries.
    Generations {
        guid: [u8; 16],
        generations: Vec<Generation>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generation {
    pub export_count: usize,
    pub name_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameEntry {
    pub value: String,
    pub flags: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ObjectReference {
    None,
    Export(usize),
    Import(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Import {
    pub class_package: usize,
    pub class_name: usize,
    pub outer: ObjectReference,
    pub object_name: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Export {
    pub class: ObjectReference,
    pub super_class: ObjectReference,
    pub outer: ObjectReference,
    pub object_name: usize,
    pub object_flags: u32,
    pub serial_size: usize,
    pub serial_offset: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageSummary {
    pub source: Arc<str>,
    pub header: PackageHeader,
    pub names: Vec<NameEntry>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
}

impl PackageSummary {
    pub fn name(&self, index: usize) -> &str {
        &self.names[index].value
    }

    pub fn object_name(&self, reference: ObjectReference) -> Option<&str> {
        match reference {
            ObjectReference::None => None,
            ObjectReference::Export(index) => self
                .exports
                .get(index)
                .map(|export| self.name(export.object_name)),
            ObjectReference::Import(index) => self
                .imports
                .get(index)
                .map(|import| self.name(import.object_name)),
        }
    }

    pub fn class_name(&self, export: &Export) -> Option<&str> {
        self.object_name(export.class)
    }
}
