use std::collections::HashMap;
use std::fmt;
use std::io;

use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BfresError {
    OutOfBounds { pos: usize, need: usize, len: usize },
    InvalidOffset { offset: usize },
    InvalidMagic,
    InvalidData(String),
}

impl fmt::Display for BfresError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds { pos, need, len } => {
                write!(f, "read past end at 0x{pos:x} (need {need}, len {len})")
            }
            Self::InvalidOffset { offset } => write!(f, "invalid offset 0x{offset:x}"),
            Self::InvalidMagic => write!(f, "invalid FRES magic"),
            Self::InvalidData(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for BfresError {}

impl From<BfresError> for io::Error {
    fn from(err: BfresError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, err)
    }
}

pub type BfresResult<T> = Result<T, BfresError>;

pub const SECTION1: usize = 0;
pub const SECTION2: usize = 1;
pub const SECTION3: usize = 2;
pub const SECTION4: usize = 3;
pub const SECTION5: usize = 4;

#[derive(Debug, Default)]
pub struct BinWriter {
    pub output: Vec<u8>,
    pub saved_header_block_positions: Vec<usize>,
    pub binary_header_block_positions: Vec<usize>,
    pub end_of_block_offset: usize,
}

impl BinWriter {
    pub fn position(&self) -> usize {
        self.output.len()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.output
    }

    pub fn write_u8(&mut self, value: u8) {
        self.output.push(value);
    }

    pub fn write_u16(&mut self, value: u16) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i16(&mut self, value: i16) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i32(&mut self, value: i32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u32(&mut self, value: u32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u64(&mut self, value: u64) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i64(&mut self, value: i64) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_f32(&mut self, value: f32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
    }

    pub fn write_zeroes(&mut self, len: usize) {
        self.output.resize(self.output.len() + len, 0);
    }

    pub fn align_bytes(&mut self, align: usize) {
        let padding = (align - (self.output.len() % align)) % align;
        self.write_zeroes(padding);
    }

    pub fn patch_u16(&mut self, offset: usize, value: u16) {
        self.output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    pub fn patch_u32(&mut self, offset: usize, value: u32) {
        self.output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    pub fn patch_i64(&mut self, offset: usize, value: i64) {
        self.output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    pub fn patch_u64(&mut self, offset: usize, value: u64) {
        self.output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    pub fn save_offset(&mut self) -> usize {
        let offset = self.position();
        self.write_u64(0);
        offset
    }

    pub fn write_offset(&mut self, offset_pos: usize) {
        self.write_offset_to(offset_pos, self.position());
    }

    pub fn write_offset_to(&mut self, offset_pos: usize, target: usize) {
        if offset_pos == 0 {
            return;
        }
        self.patch_u64(offset_pos, target as u64);
    }

    pub fn save_header_block(&mut self, binary_only: bool) {
        if binary_only {
            self.binary_header_block_positions.push(self.position());
            self.write_u16(0);
        } else {
            self.saved_header_block_positions.push(self.position());
            self.write_u32(0);
            self.write_u64(0);
        }
    }

    pub fn write_header_blocks(&mut self) {
        let str_block_start = self.saved_header_block_positions.first().copied();
        for position in self.binary_header_block_positions.clone() {
            if let Some(next) = str_block_start {
                self.patch_u16(position, (next - 4) as u16);
            }
        }
        let block_positions = self.saved_header_block_positions.clone();
        for index in 0..block_positions.len() {
            let position = block_positions[index];
            if index + 1 == block_positions.len() {
                self.patch_u32(position, 0);
                self.patch_u64(
                    position + 4,
                    (self.end_of_block_offset.saturating_sub(position)) as u64,
                );
            } else {
                let size = (block_positions[index + 1] - block_positions[index]) as u32;
                self.patch_u32(position, size);
                self.patch_u64(position + 4, size as u64);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelocationEntry {
    pub position: u32,
    pub struct_count: u32,
    pub offset_count: u32,
    pub padding_count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RelocationSection {
    pub entries: Vec<RelocationEntry>,
    pub position: u32,
    pub size: u32,
}

#[derive(Debug, Default)]
pub struct RelocationTable {
    pub sections: Vec<RelocationSection>,
    pub relocation_table_offset_pos: usize,
    pub written_offset: usize,
}

impl RelocationTable {
    pub fn new(count: usize) -> Self {
        Self {
            sections: vec![RelocationSection::default(); count],
            relocation_table_offset_pos: 0,
            written_offset: 0,
        }
    }

    pub fn save_header_offset(&mut self, writer: &mut BinWriter) {
        self.relocation_table_offset_pos = writer.position();
        writer.write_u32(0);
    }

    pub fn save_entry(
        &mut self,
        position: usize,
        offset_count: u32,
        struct_count: u32,
        padding_count: u32,
        section_idx: usize,
    ) {
        if offset_count > 255 {
            self.save_entry(position, 255, struct_count, padding_count, section_idx);
            self.save_entry(
                position + 255 * 8,
                offset_count - 255,
                struct_count,
                padding_count,
                section_idx,
            );
            return;
        }
        let idx = section_idx.min(self.sections.len().saturating_sub(1));
        self.sections[idx].entries.push(RelocationEntry {
            position: position as u32,
            struct_count,
            offset_count,
            padding_count,
        });
    }

    pub fn set_section(&mut self, section_idx: usize, position: u32, size: u32) {
        if section_idx < self.sections.len() {
            self.sections[section_idx].position = position;
            self.sections[section_idx].size = size;
        }
    }

    pub fn write(&mut self, writer: &mut BinWriter) {
        for section in &mut self.sections {
            section.entries.sort_by_key(|entry| entry.position);
        }
        // No alignment: the game writes `_RLT` immediately after the buffer section. Padding to
        // 256 put it at the next boundary (0x2200 where `ef_return.eff` has 0x2120) and inflated
        // every container by the gap.
        let position = writer.position();
        self.written_offset = position;
        if self.relocation_table_offset_pos != 0 {
            writer.patch_u32(self.relocation_table_offset_pos, position as u32);
        }
        writer.write_bytes(b"_RLT");
        writer.end_of_block_offset = writer.position();
        writer.write_u32(position as u32);
        writer.write_u32(self.sections.len() as u32);
        writer.write_u32(0);
        let mut entry_start = 0i32;
        for section in &self.sections {
            writer.write_u64(0);
            writer.write_u32(section.position);
            writer.write_u32(section.size);
            writer.write_i32(entry_start);
            writer.write_i32(section.entries.len() as i32);
            entry_start += section.entries.len() as i32;
        }
        for section in &self.sections {
            for entry in &section.entries {
                writer.write_u32(entry.position);
                writer.write_u16(entry.struct_count as u16);
                writer.write_u8(entry.offset_count as u8);
                writer.write_u8(entry.padding_count as u8);
            }
        }
    }
}

impl BinWriter {
    pub fn write_signature(&mut self, magic: &str) {
        self.write_bytes(magic.as_bytes());
    }
}

#[derive(Debug, Default, Clone)]
struct StringEntry {
    positions: Vec<usize>,
}

#[derive(Debug, Default)]
pub struct StringTable {
    strings: IndexMap<String, StringEntry>,
    sorted_keys: Vec<String>,
    pub file_name: String,
    pub pool_start: usize,
    pub pool_len: usize,
}

impl StringTable {
    pub fn collect_keys(&mut self, file_name: &str, extra: &[String]) {
        self.file_name = file_name.to_string();
        self.sorted_keys = extra.to_vec();
    }

    fn res_string_cmp(a: &str, b: &str) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if a.is_empty() && b.is_empty() {
            return Ordering::Equal;
        }
        if a.is_empty() {
            return Ordering::Greater;
        }
        if b.is_empty() {
            return Ordering::Less;
        }
        a.cmp(b)
    }

    pub fn ordered_keys(&self) -> Vec<String> {
        let mut sorted = Vec::new();
        for key in &self.sorted_keys {
            if self.strings.contains_key(key) && !sorted.iter().any(|existing| existing == key) {
                sorted.push(key.clone());
            }
        }
        let mut rest: Vec<String> = self
            .strings
            .keys()
            .filter(|key| !sorted.iter().any(|existing| existing == *key))
            .cloned()
            .collect();
        rest.sort_by(|a, b| Self::res_string_cmp(a, b));
        sorted.extend(rest);
        sorted
    }

    pub fn pool_keys(&self) -> Vec<String> {
        self.ordered_keys()
    }

    pub fn add_entry(&mut self, position: usize, value: &str) {
        self.strings
            .entry(value.to_string())
            .or_default()
            .positions
            .push(position);
    }

    pub fn write_in_pool(&self, writer: &mut BinWriter, file_name_header_pos: usize) {
        let keys = self.pool_keys();
        let mut pool = Vec::new();
        let mut string_positions = HashMap::new();
        let mut file_name_string_pos = 0u32;
        let mut cursor = 0usize;
        for key in &keys {
            let abs_pos = self.pool_start + cursor;
            string_positions.insert(key.clone(), abs_pos as u32);
            if key == &self.file_name {
                file_name_string_pos = (abs_pos + 2) as u32;
            }
            pool.extend_from_slice(&(key.len() as u16).to_le_bytes());
            pool.extend_from_slice(key.as_bytes());
            pool.push(0);
            cursor += 2 + key.len() + 1;
            while !cursor.is_multiple_of(2) {
                pool.push(0);
                cursor += 1;
            }
        }
        while pool.len() < self.pool_len {
            pool.push(0);
        }
        pool.truncate(self.pool_len);
        writer.output[self.pool_start..self.pool_start + self.pool_len].copy_from_slice(&pool);

        for (key, entry) in &self.strings {
            let Some(&target) = string_positions.get(key) else {
                continue;
            };
            for offset in &entry.positions {
                writer.patch_u32(*offset, target);
            }
        }
        if file_name_string_pos != 0 {
            writer.patch_u32(file_name_header_pos, file_name_string_pos);
        }
    }
}

pub struct BinReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> BinReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn seek(&mut self, pos: usize) -> BfresResult<()> {
        if pos > self.data.len() {
            return Err(BfresError::InvalidOffset { offset: pos });
        }
        self.pos = pos;
        Ok(())
    }

    fn ensure(&self, need: usize) -> BfresResult<()> {
        if self.pos + need > self.data.len() {
            Err(BfresError::OutOfBounds {
                pos: self.pos,
                need,
                len: self.data.len(),
            })
        } else {
            Ok(())
        }
    }

    pub fn read_u8(&mut self) -> BfresResult<u8> {
        self.ensure(1)?;
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn read_u16(&mut self) -> BfresResult<u16> {
        self.ensure(2)?;
        let v = u16::from_le_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        Ok(v)
    }

    pub fn read_i16(&mut self) -> BfresResult<i16> {
        self.ensure(2)?;
        let v = i16::from_le_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        Ok(v)
    }

    pub fn read_u32(&mut self) -> BfresResult<u32> {
        self.ensure(4)?;
        let v = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    pub fn read_i32(&mut self) -> BfresResult<i32> {
        self.ensure(4)?;
        let v = i32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    pub fn read_u64(&mut self) -> BfresResult<u64> {
        self.ensure(8)?;
        let v = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }

    pub fn read_i64(&mut self) -> BfresResult<i64> {
        self.ensure(8)?;
        let v = i64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }

    pub fn read_f32(&mut self) -> BfresResult<f32> {
        self.ensure(4)?;
        let v = f32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    pub fn read_bytes(&mut self, len: usize) -> BfresResult<Vec<u8>> {
        self.ensure(len)?;
        let bytes = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(bytes)
    }

    /// Switch BFRES stores offsets in u64 slots. BfresLibrary `ReadOffset()` uses `(uint)ReadUInt64()`
    /// (lower 32 bits only). Runtime GPU pool tags in the upper half (e.g. 0xDE00000000) therefore
    /// resolve to offset 0, which matches tolerant C# load behavior.
    pub fn read_switch_offset(&mut self) -> BfresResult<u64> {
        let lo = self.read_u64()? as u32;
        if lo == 0 || lo == u32::MAX {
            Ok(0)
        } else {
            Ok(u64::from(lo))
        }
    }

    pub fn read_offset_u64(&mut self) -> BfresResult<u64> {
        self.read_switch_offset()
    }

    pub fn read_string_at(&mut self, offset: usize) -> BfresResult<String> {
        if offset >= self.data.len() {
            return Err(BfresError::InvalidOffset { offset });
        }
        self.seek(offset)?;
        let _len = self.read_u16()?;
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        Ok(String::from_utf8_lossy(&self.data[start..self.pos]).into_owned())
    }

    pub fn read_string_ref(&mut self) -> BfresResult<String> {
        let offset = self.read_u64()? as usize;
        if offset == 0 || offset >= self.data.len() {
            return Ok(String::new());
        }
        let resume = self.pos;
        let value = self.read_string_at(offset);
        self.seek(resume)?;
        value
    }

    pub fn align(&mut self, align: usize) -> BfresResult<()> {
        if align == 0 {
            return Ok(());
        }
        let padding = (align - (self.pos % align)) % align;
        self.ensure(padding)?;
        self.pos += padding;
        Ok(())
    }
}

pub fn encode_version(major: u32, major2: u32, minor: u32, minor2: u32) -> u32 {
    let hi = if major != 0 { major } else { major2 };
    hi << 16 | minor << 8 | minor2
}

pub fn decode_version(version: u32) -> (u32, u32, u32) {
    (
        (version >> 16) & 0xFFFF,
        (version >> 8) & 0xFF,
        version & 0xFF,
    )
}

pub fn data_alignment(alignment_field: u8) -> usize {
    1 << alignment_field as usize
}
