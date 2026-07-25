use std::{ops::Range, sync::Arc};

use crate::{
    archive::Archive,
    error::{Error, Result},
    package::{ObjectReference, PackageSummary, object_reference},
};

/// The type nibble stored in an Unreal serialized property tag.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PropertyKind {
    Byte = 1,
    Int = 2,
    Bool = 3,
    Float = 4,
    Object = 5,
    Name = 6,
    String = 7,
    Class = 8,
    Array = 9,
    Struct = 10,
    Vector = 11,
    Rotator = 12,
    Str = 13,
    Map = 14,
    FixedArray = 15,
}

impl PropertyKind {
    fn from_tag(value: u8, package: Arc<str>, offset: usize) -> Result<Self> {
        Ok(match value {
            1 => Self::Byte,
            2 => Self::Int,
            3 => Self::Bool,
            4 => Self::Float,
            5 => Self::Object,
            6 => Self::Name,
            7 => Self::String,
            8 => Self::Class,
            9 => Self::Array,
            10 => Self::Struct,
            11 => Self::Vector,
            12 => Self::Rotator,
            13 => Self::Str,
            14 => Self::Map,
            15 => Self::FixedArray,
            kind => {
                return Err(Error::InvalidPropertyType {
                    package,
                    offset,
                    kind,
                });
            }
        })
    }
}

/// Metadata for one tagged property. `value` is relative to the export payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyTag {
    pub name: usize,
    pub kind: PropertyKind,
    pub array_index: Option<usize>,
    pub struct_name: Option<usize>,
    pub bool_value: Option<bool>,
    pub value: Range<usize>,
}

/// A checked cursor over the serialized payload of one export.
///
/// The cursor retains the package summary so typed decoders can resolve names
/// while keeping all byte-range checks in the package crate.
pub struct ObjectReader<'a> {
    archive: Archive<'a>,
    summary: &'a PackageSummary,
}

impl<'a> ObjectReader<'a> {
    pub(crate) fn new(data: &'a [u8], summary: &'a PackageSummary, base_offset: usize) -> Self {
        Self {
            archive: Archive::with_base(data, Arc::clone(&summary.source), base_offset),
            summary,
        }
    }

    pub fn summary(&self) -> &'a PackageSummary {
        self.summary
    }

    pub fn position(&self) -> usize {
        self.archive.position()
    }

    pub fn absolute_position(&self) -> usize {
        self.archive.absolute_position()
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        self.archive.read_u8()
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        self.archive.read_u16()
    }

    pub fn read_i16(&mut self) -> Result<i16> {
        self.archive.read_i16()
    }

    pub fn read_u32(&mut self) -> Result<u32> {
        self.archive.read_u32()
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        self.archive.read_i32()
    }

    pub fn read_u64(&mut self) -> Result<u64> {
        self.archive.read_u64()
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        self.archive.read_f32()
    }

    pub fn read_compact_index(&mut self) -> Result<i32> {
        self.archive.read_compact_index()
    }

    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        self.archive.take(length)
    }

    pub fn read_string(&mut self) -> Result<String> {
        if self.summary.header.version < 64 {
            self.archive.read_c_string()
        } else {
            self.archive.read_unreal_string()
        }
    }

    pub fn remaining(&self) -> usize {
        self.archive.remaining()
    }

    pub fn read_object_reference(&mut self) -> Result<ObjectReference> {
        let offset = self.absolute_position();
        let index = self.read_compact_index()?;
        object_reference(index, self.archive.source(), offset)
    }

    /// Reads the next tagged property and advances past its value.
    ///
    /// `None` is both an engine name and the property-list terminator.
    pub fn next_property(&mut self) -> Result<Option<PropertyTag>> {
        let name = self.read_name_index("property name")?;
        if self.summary.name(name).eq_ignore_ascii_case("None") {
            return Ok(None);
        }

        let info_offset = self.absolute_position();
        let info = self.read_u8()?;
        let kind = PropertyKind::from_tag(info & 0x0f, self.archive.source(), info_offset)?;
        let mut size = match (info >> 4) & 0x07 {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 12,
            4 => 16,
            5 => usize::from(self.read_u8()?),
            6 => usize::from(self.read_u16()?),
            7 => usize::try_from(self.read_u32()?).map_err(|_| Error::InvalidCount {
                package: self.archive.source(),
                field: "property payload",
                count: i32::MAX,
                offset: info_offset,
            })?,
            _ => unreachable!(),
        };

        // Bool values live in the array flag and have no payload bytes.
        let bool_value = (kind == PropertyKind::Bool).then_some(info & 0x80 != 0);
        if kind == PropertyKind::Bool {
            size = 0;
        }

        let struct_name = (kind == PropertyKind::Struct)
            .then(|| self.read_name_index("property struct"))
            .transpose()?;
        let array_index = (info & 0x80 != 0 && kind != PropertyKind::Bool)
            .then(|| self.read_nonnegative_compact("property array index"))
            .transpose()?;
        let start = self.position();
        self.read_bytes(size)?;

        Ok(Some(PropertyTag {
            name,
            kind,
            array_index,
            struct_name,
            bool_value,
            value: start..start + size,
        }))
    }

    /// Creates a checked reader for a previously returned property value.
    pub fn property_reader(&self, property: &PropertyTag) -> ObjectReader<'a> {
        let data = &self.archive_bytes()[property.value.clone()];
        ObjectReader::new(
            data,
            self.summary,
            self.base_offset() + property.value.start,
        )
    }

    fn read_name_index(&mut self, field: &'static str) -> Result<usize> {
        let offset = self.absolute_position();
        let index = self.read_compact_index()?;
        usize::try_from(index)
            .ok()
            .filter(|index| *index < self.summary.names.len())
            .ok_or_else(|| Error::InvalidNameIndex {
                package: self.archive.source(),
                field,
                index,
                name_count: self.summary.names.len(),
                offset,
            })
    }

    fn read_nonnegative_compact(&mut self, field: &'static str) -> Result<usize> {
        let offset = self.absolute_position();
        let value = self.read_compact_index()?;
        usize::try_from(value).map_err(|_| Error::InvalidCount {
            package: self.archive.source(),
            field,
            count: value,
            offset,
        })
    }

    fn archive_bytes(&self) -> &'a [u8] {
        // ObjectReader is the only public gateway to Archive; exposing this
        // internally keeps sub-readers on the same checked byte slice.
        self.archive.bytes()
    }

    fn base_offset(&self) -> usize {
        self.archive.base_offset()
    }
}
