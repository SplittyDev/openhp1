use openhp1_package::{ObjectReader, ObjectReference};

use crate::{Error, Result};

const MAX_DEPTH: u8 = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallTarget {
    Native(u16),
    Virtual(usize),
    Final(ObjectReference),
    Global(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    pub offset: usize,
    pub depth: u8,
    pub opcode: u8,
    pub call: Option<CallTarget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bytecode {
    pub version: u16,
    /// UE1's execution representation, with compact indices expanded to i32.
    pub bytes: Vec<u8>,
    pub raw_len: usize,
    pub tokens: Vec<Token>,
}

impl Bytecode {
    pub(crate) fn decode(reader: &mut ObjectReader<'_>, decoded_size: u32) -> Result<Self> {
        let expected = usize::try_from(decoded_size).map_err(|_| Error::BytecodeSize {
            package: reader.summary().source.clone(),
            raw_offset: reader.absolute_position(),
            expected: usize::MAX,
            actual: 0,
        })?;
        let maximum = reader.remaining().saturating_mul(4);
        if expected > maximum {
            return Err(Error::BytecodeSize {
                package: reader.summary().source.clone(),
                raw_offset: reader.absolute_position(),
                expected,
                actual: maximum,
            });
        }

        let start = reader.position();
        let version = reader.summary().header.version;
        let mut parser = Parser {
            reader,
            expected,
            version,
            bytes: Vec::with_capacity(expected),
            tokens: Vec::new(),
        };
        while parser.bytes.len() < expected {
            parser.read_token(0)?;
        }
        if parser.bytes.len() != expected {
            return Err(parser.size_error());
        }
        Ok(Self {
            version,
            bytes: parser.bytes,
            raw_len: parser.reader.position() - start,
            tokens: parser.tokens,
        })
    }
}

struct Parser<'reader, 'data> {
    reader: &'reader mut ObjectReader<'data>,
    expected: usize,
    version: u16,
    bytes: Vec<u8>,
    tokens: Vec<Token>,
}

impl Parser<'_, '_> {
    fn read_token(&mut self, depth: u8) -> Result<u8> {
        if depth >= MAX_DEPTH {
            return Err(Error::RecursionLimit {
                package: self.reader.summary().source.clone(),
                raw_offset: self.reader.absolute_position(),
                decoded_offset: self.bytes.len(),
            });
        }

        let raw_offset = self.reader.absolute_position();
        let offset = self.bytes.len();
        let opcode = self.reader.read_u8()?;
        self.push(&[opcode])?;
        let token_index = self.tokens.len();
        self.tokens.push(Token {
            offset,
            depth,
            opcode,
            call: None,
        });
        let child_depth = depth + 1;

        if (0x39..=0x60).contains(&opcode) {
            self.read_token(child_depth)?;
        } else if opcode >= 0x70 {
            self.tokens[token_index].call = Some(CallTarget::Native(u16::from(opcode)));
            self.read_parameters(child_depth)?;
        } else if (0x61..=0x6f).contains(&opcode) {
            let low = self.read_u8()?;
            let native = (u16::from(opcode - 0x60) << 8) | u16::from(low);
            self.tokens[token_index].call = Some(CallTarget::Native(native));
            self.read_parameters(child_depth)?;
        } else {
            match opcode {
                0x00..=0x02 => {
                    self.read_index()?;
                }
                0x04 => {
                    if self.version > 61 {
                        self.read_token(child_depth)?;
                    }
                }
                0x05 => {
                    self.read_u8()?;
                    self.read_token(child_depth)?;
                }
                0x06 => {
                    self.read_u16()?;
                }
                0x07 => {
                    self.read_u16()?;
                    self.read_token(child_depth)?;
                }
                0x08 | 0x0b | 0x15 | 0x16 | 0x17 | 0x25..=0x28 | 0x2a | 0x30 | 0x31 => {}
                0x09 => {
                    self.read_u16()?;
                    self.read_token(child_depth)?;
                }
                0x0a => {
                    let next = self.read_u16()?;
                    if next != 0xffff {
                        self.read_token(child_depth)?;
                    }
                }
                0x0c => loop {
                    let name = self.read_name("label")?;
                    self.read_u32()?;
                    if self
                        .reader
                        .summary()
                        .name(name)
                        .eq_ignore_ascii_case("None")
                    {
                        break;
                    }
                },
                0x0d | 0x0e | 0x2d | 0x37 => {
                    self.read_token(child_depth)?;
                }
                0x0f | 0x10 | 0x1a => {
                    self.read_token(child_depth)?;
                    self.read_token(child_depth)?;
                }
                0x11 => {
                    for _ in 0..4 {
                        self.read_token(child_depth)?;
                    }
                }
                0x12 | 0x19 => {
                    self.read_token(child_depth)?;
                    self.read_u16()?;
                    self.read_u8()?;
                    self.read_token(child_depth)?;
                }
                0x13 | 0x2e => {
                    self.read_index()?;
                    self.read_token(child_depth)?;
                }
                0x14 if self.version <= 63 => loop {
                    let size = self.read_u8()?;
                    if size == 0 {
                        break;
                    }
                    self.read_u8()?;
                },
                0x14 => {
                    self.read_token(child_depth)?;
                    self.read_token(child_depth)?;
                }
                0x18 => {
                    self.read_u16()?;
                    self.read_token(child_depth)?;
                }
                0x1b => {
                    let name = self.read_name("virtual function")?;
                    self.tokens[token_index].call = Some(CallTarget::Virtual(name));
                    self.read_parameters(child_depth)?;
                }
                0x1c => {
                    let object = self.read_object()?;
                    self.tokens[token_index].call = Some(CallTarget::Final(object));
                    self.read_parameters(child_depth)?;
                }
                0x1d => {
                    self.read_u32()?;
                }
                0x1e => {
                    self.read_u32()?;
                }
                0x1f => self.read_ascii_z()?,
                0x20 => {
                    self.read_index()?;
                }
                0x21 => {
                    self.read_name("name constant")?;
                }
                0x22 | 0x23 => {
                    for _ in 0..3 {
                        self.read_u32()?;
                    }
                }
                0x24 | 0x2c => {
                    self.read_u8()?;
                }
                0x29 => {
                    self.read_index()?;
                }
                0x2b => {
                    self.read_u8()?;
                    self.read_token(child_depth)?;
                }
                0x2f => {
                    self.read_token(child_depth)?;
                    self.read_u16()?;
                }
                0x32 | 0x33 => {
                    self.read_index()?;
                    self.read_token(child_depth)?;
                    self.read_token(child_depth)?;
                }
                0x34 => self.read_unicode_z()?,
                0x36 => {
                    self.read_index()?;
                    self.read_token(child_depth)?;
                }
                0x38 => {
                    let name = self.read_name("global function")?;
                    self.tokens[token_index].call = Some(CallTarget::Global(name));
                    self.read_parameters(child_depth)?;
                }
                _ => {
                    return Err(Error::UnknownToken {
                        package: self.reader.summary().source.clone(),
                        raw_offset,
                        decoded_offset: offset,
                        token: opcode,
                    });
                }
            }
        }
        Ok(opcode)
    }

    fn read_parameters(&mut self, depth: u8) -> Result<()> {
        while self.read_token(depth)? != 0x16 {}
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8> {
        let value = self.reader.read_u8()?;
        self.push(&[value])?;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let value = self.reader.read_u16()?;
        self.push(&value.to_le_bytes())?;
        Ok(value)
    }

    fn read_u32(&mut self) -> Result<u32> {
        let value = self.reader.read_u32()?;
        self.push(&value.to_le_bytes())?;
        Ok(value)
    }

    fn read_index(&mut self) -> Result<i32> {
        let value = self.reader.read_compact_index()?;
        self.push(&value.to_le_bytes())?;
        Ok(value)
    }

    fn read_name(&mut self, field: &'static str) -> Result<usize> {
        let value = self.reader.read_name_index(field)?;
        let encoded = i32::try_from(value).map_err(|_| Error::InvalidCount {
            package: self.reader.summary().source.clone(),
            field,
            count: i32::MAX,
            offset: self.reader.absolute_position(),
        })?;
        self.push(&encoded.to_le_bytes())?;
        Ok(value)
    }

    fn read_object(&mut self) -> Result<ObjectReference> {
        let object = self.reader.read_object_reference()?;
        let index = match object {
            ObjectReference::None => 0,
            ObjectReference::Export(index) => i32::try_from(index + 1).unwrap(),
            ObjectReference::Import(index) => -i32::try_from(index + 1).unwrap(),
        };
        self.push(&index.to_le_bytes())?;
        Ok(object)
    }

    fn read_ascii_z(&mut self) -> Result<()> {
        loop {
            if self.read_u8()? == 0 {
                return Ok(());
            }
        }
    }

    fn read_unicode_z(&mut self) -> Result<()> {
        loop {
            if self.read_u16()? == 0 {
                return Ok(());
            }
        }
    }

    fn push(&mut self, value: &[u8]) -> Result<()> {
        let Some(actual) = self.bytes.len().checked_add(value.len()) else {
            return Err(self.size_error());
        };
        if actual > self.expected {
            return Err(Error::BytecodeSize {
                package: self.reader.summary().source.clone(),
                raw_offset: self.reader.absolute_position(),
                expected: self.expected,
                actual,
            });
        }
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn size_error(&self) -> Error {
        Error::BytecodeSize {
            package: self.reader.summary().source.clone(),
            raw_offset: self.reader.absolute_position(),
            expected: self.expected,
            actual: self.bytes.len(),
        }
    }
}

pub fn token_name(opcode: u8) -> &'static str {
    match opcode {
        0x00 => "LocalVariable",
        0x01 => "InstanceVariable",
        0x02 => "DefaultVariable",
        0x04 => "Return",
        0x05 => "Switch",
        0x06 => "Jump",
        0x07 => "JumpIfNot",
        0x08 => "Stop",
        0x09 => "Assert",
        0x0a => "Case",
        0x0b => "Nothing",
        0x0c => "LabelTable",
        0x0d => "GotoLabel",
        0x0e => "EatString",
        0x0f => "Let",
        0x10 => "DynArrayElement",
        0x11 => "New",
        0x12 => "ClassContext",
        0x13 => "MetaCast",
        0x14 => "LetBool",
        0x15 => "Unknown0x15",
        0x16 => "EndFunctionParms",
        0x17 => "Self",
        0x18 => "Skip",
        0x19 => "Context",
        0x1a => "ArrayElement",
        0x1b => "VirtualFunction",
        0x1c => "FinalFunction",
        0x1d => "IntConst",
        0x1e => "FloatConst",
        0x1f => "StringConst",
        0x20 => "ObjectConst",
        0x21 => "NameConst",
        0x22 => "RotationConst",
        0x23 => "VectorConst",
        0x24 => "ByteConst",
        0x25 => "IntZero",
        0x26 => "IntOne",
        0x27 => "True",
        0x28 => "False",
        0x29 => "NativeParm",
        0x2a => "NoObject",
        0x2b => "Unknown0x2b",
        0x2c => "IntConstByte",
        0x2d => "BoolVariable",
        0x2e => "DynamicCast",
        0x2f => "Iterator",
        0x30 => "IteratorPop",
        0x31 => "IteratorNext",
        0x32 => "StructCmpEq",
        0x33 => "StructCmpNe",
        0x34 => "UnicodeStringConst",
        0x36 => "StructMember",
        0x37 => "DynArrayToInt",
        0x38 => "GlobalFunction",
        0x39..=0x60 => "Conversion",
        0x61..=0x6f => "ExtendedNative",
        0x70..=0xff => "Native",
        _ => "Unknown",
    }
}
