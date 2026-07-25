use std::sync::Arc;

use crate::error::{Error, Result};

/// A little-endian, bounds-checked cursor over one package file.
///
/// Unreal packages contain absolute table offsets and variable-width integers,
/// so an explicit cursor provides clearer diagnostics than deriving the whole
/// format as one contiguous Rust structure.
pub(crate) struct Archive<'a> {
    bytes: &'a [u8],
    position: usize,
    base_offset: usize,
    source: Arc<str>,
}

impl<'a> Archive<'a> {
    pub(crate) fn new(bytes: &'a [u8], source: Arc<str>) -> Self {
        Self {
            bytes,
            position: 0,
            base_offset: 0,
            source,
        }
    }

    pub(crate) fn with_base(bytes: &'a [u8], source: Arc<str>, base_offset: usize) -> Self {
        Self {
            bytes,
            position: 0,
            base_offset,
            source,
        }
    }

    pub(crate) fn position(&self) -> usize {
        self.position
    }

    pub(crate) fn absolute_position(&self) -> usize {
        self.base_offset + self.position
    }

    pub(crate) fn source(&self) -> Arc<str> {
        Arc::clone(&self.source)
    }

    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub(crate) fn base_offset(&self) -> usize {
        self.base_offset
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub(crate) fn seek(&mut self, position: usize, field: &'static str) -> Result<()> {
        if position > self.bytes.len() {
            return Err(Error::InvalidOffset {
                package: Arc::clone(&self.source),
                field,
                offset: self.base_offset + position,
                file_len: self.base_offset + self.bytes.len(),
            });
        }
        self.position = position;
        Ok(())
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub(crate) fn read_i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn read_i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub(crate) fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn read_guid(&mut self) -> Result<[u8; 16]> {
        Ok(self.take(16)?.try_into().unwrap())
    }

    /// Reads UE1's signed, one-to-five-byte compact index.
    pub(crate) fn read_compact_index(&mut self) -> Result<i32> {
        let start = self.position;
        let first = self.read_u8()?;
        let negative = first & 0x80 != 0;
        let mut value = i64::from(first & 0x3f);
        let mut more = first & 0x40 != 0;
        let mut shift = 6;

        for byte_number in 1..5 {
            if !more {
                break;
            }

            let byte = self.read_u8()?;
            let payload = if byte_number == 4 {
                if byte & 0xf0 != 0 {
                    return Err(Error::InvalidCompactIndex {
                        package: Arc::clone(&self.source),
                        offset: self.base_offset + start,
                    });
                }
                byte & 0x0f
            } else {
                byte & 0x7f
            };
            value |= i64::from(payload) << shift;
            more = byte & 0x80 != 0;
            shift += 7;
        }

        if more || value > i64::from(i32::MAX) + i64::from(negative) {
            return Err(Error::InvalidCompactIndex {
                package: Arc::clone(&self.source),
                offset: self.base_offset + start,
            });
        }

        if negative {
            if value == 0 {
                return Err(Error::InvalidCompactIndex {
                    package: Arc::clone(&self.source),
                    offset: self.base_offset + start,
                });
            }
            Ok(-(value as i32))
        } else {
            Ok(value as i32)
        }
    }

    pub(crate) fn read_c_string(&mut self) -> Result<String> {
        let start = self.position;
        let remaining = &self.bytes[start..];
        let Some(end) = remaining.iter().position(|byte| *byte == 0) else {
            return Err(Error::UnterminatedString {
                package: Arc::clone(&self.source),
                offset: self.base_offset + start,
            });
        };
        self.position += end + 1;
        Ok(String::from_utf8_lossy(&remaining[..end]).into_owned())
    }

    /// Reads the length-prefixed string representation used by package names
    /// from package version 64 onwards.
    pub(crate) fn read_unreal_string(&mut self) -> Result<String> {
        let offset = self.absolute_position();
        let length = self.read_compact_index()?;
        if length == 0 {
            return Ok(String::new());
        }

        if length > 0 {
            let length = checked_length(length, self.bytes.len(), &self.source, offset)?;
            let bytes = self.take(length)?;
            if bytes.last() != Some(&0) {
                return Err(Error::UnterminatedString {
                    package: Arc::clone(&self.source),
                    offset,
                });
            }
            return Ok(String::from_utf8_lossy(&bytes[..length - 1]).into_owned());
        }

        let units = checked_length(
            length
                .checked_abs()
                .ok_or_else(|| Error::InvalidStringLength {
                    package: Arc::clone(&self.source),
                    offset,
                    length,
                })?,
            self.bytes.len() / 2,
            &self.source,
            offset,
        )?;
        let byte_len = units
            .checked_mul(2)
            .ok_or_else(|| Error::InvalidStringLength {
                package: Arc::clone(&self.source),
                offset,
                length,
            })?;
        let bytes = self.take(byte_len)?;
        let utf16: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if utf16.last() != Some(&0) {
            return Err(Error::UnterminatedString {
                package: Arc::clone(&self.source),
                offset,
            });
        }
        String::from_utf16(&utf16[..units - 1]).map_err(|source| Error::InvalidUtf16 {
            package: Arc::clone(&self.source),
            offset,
            source,
        })
    }

    pub(crate) fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| Error::UnexpectedEnd {
                package: Arc::clone(&self.source),
                offset: self.absolute_position(),
                needed: length,
                file_len: self.base_offset + self.bytes.len(),
            })?;
        let bytes = &self.bytes[self.position..end];
        self.position = end;
        Ok(bytes)
    }
}

fn checked_length(length: i32, maximum: usize, source: &Arc<str>, offset: usize) -> Result<usize> {
    let length = usize::try_from(length).map_err(|_| Error::InvalidStringLength {
        package: Arc::clone(source),
        offset,
        length,
    })?;
    if length == 0 || length > maximum {
        return Err(Error::InvalidStringLength {
            package: Arc::clone(source),
            offset,
            length: length as i32,
        });
    }
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::Archive;

    fn compact(bytes: &[u8]) -> i32 {
        Archive::new(bytes, "test".into())
            .read_compact_index()
            .unwrap()
    }

    #[test]
    fn reads_compact_index_boundaries() {
        assert_eq!(compact(&[0x00]), 0);
        assert_eq!(compact(&[0x3f]), 63);
        assert_eq!(compact(&[0x40, 0x01]), 64);
        assert_eq!(compact(&[0x7f, 0x7f]), 8_191);
        assert_eq!(compact(&[0x80 | 42]), -42);
        assert_eq!(compact(&[0x7f, 0xff, 0xff, 0xff, 0x0f]), i32::MAX);
    }

    #[test]
    fn rejects_negative_zero_and_overlong_compact_indices() {
        assert!(
            Archive::new(&[0x80], "test".into())
                .read_compact_index()
                .is_err()
        );
        assert!(
            Archive::new(&[0x40, 0x80, 0x80, 0x80, 0x10], "test".into())
                .read_compact_index()
                .is_err()
        );
    }
}
