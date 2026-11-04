use std::collections::HashMap;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use byteorder::{LittleEndian, ReadBytesExt};
use num_bigint::BigUint;

const SHADER_STAGE_COUNT: usize = 6;
const SHADER_STAGE_HEADER_SIZE: usize = 64;

#[derive(Debug, Clone)]
pub struct BinaryHeader {
    pub magic: u64,
    pub version_micro: u8,
    pub version_minor: u8,
    pub version_major: u16,
    pub byte_order: u16,
    pub alignment: u8,
    pub target_address_size: u8,
    pub name_offset: u32,
    pub flag: u16,
    pub block_offset: u16,
    pub relocation_table_offset: u32,
    pub file_size: u32,
}

#[derive(Debug, Clone)]
pub struct BnshHeader {
    pub magic: u32,                  // "grsc" = 0x63737267
    pub block_offset: u32,           // Calculated during save
    pub block_size: u32,             // Calculated during save
    pub padding: u32,                // Reserved, usually 0
    pub api_type: u16,               // Default 4 for new files
    pub api_version: u16,            // Default 0
    pub code_target: u32,            // Default 0
    pub compiler_version: u32,       // Default 131330 (0x20102)
    pub num_variation: u32,          // Number of variations
    pub variation_start_offset: u64, // Offset to first variation
    pub memory_pool_offset: u64,     // Memory pool offset
    pub unknown2: u64,               // Default 4785117553819657 (0x00431D00001E3008)
}

#[derive(Debug, Clone, Default)]
pub struct ShaderCode {
    pub control_code: Vec<u8>,
    pub byte_code: Vec<u8>,
    pub reserved: [u8; 32],
}

#[derive(Debug, Clone, Default)]
struct ShaderReflectionHeader {
    output_idx: i32,
    sampler_idx: i32,
    const_buffer_idx: i32,
    unordered_access_buffer_idx: i32,
    compute_work_group_x: i32,
    compute_work_group_y: i32,
    compute_work_group_z: i32,
    image_idx: i32,
}

#[derive(Debug, Clone, Default)]
pub struct ShaderReflectionData {
    header: ShaderReflectionHeader,
    inputs: Vec<String>,
    outputs: Vec<String>,
    samplers: Vec<String>,
    constant_buffers: Vec<String>,
    unordered_access_buffers: Vec<String>,
    slots: Vec<i32>,
}

impl ShaderReflectionData {
    fn named(&self, names: &[String], first: i32) -> Vec<(String, i32)> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), self.slots.get(first.max(0) as usize + i).copied().unwrap_or(-1)))
            .collect()
    }

    /// Vertex inputs (attribute name, location). Visionary addition.
    pub fn input_slots(&self) -> Vec<(String, i32)> {
        self.named(&self.inputs, 0)
    }

    /// Samplers (name, texture unit). Visionary addition.
    pub fn sampler_slots(&self) -> Vec<(String, i32)> {
        self.named(&self.samplers, self.header.sampler_idx)
    }

    /// Constant buffers (block name, binding). Visionary addition.
    pub fn constant_buffer_slots(&self) -> Vec<(String, i32)> {
        self.named(&self.constant_buffers, self.header.const_buffer_idx)
    }
}

#[derive(Debug, Clone)]
struct PendingReflection<'a> {
    data: &'a ShaderReflectionData,
    input_offset_pos: usize,
    output_offset_pos: usize,
    sampler_offset_pos: usize,
    cbuf_offset_pos: usize,
    uab_offset_pos: usize,
    slot_offset_pos: usize,
}

#[derive(Debug, Clone, Default)]
pub struct BnshShaderProgram {
    pub flags: u8,
    pub code_type: u8,
    pub format: u8,
    pub padding: u8,
    pub binary_format: u32,
    pub memory_data: Vec<u8>,
    pub stages: [Option<ShaderCode>; SHADER_STAGE_COUNT],
    pub reflections: [Option<ShaderReflectionData>; SHADER_STAGE_COUNT],
}

#[derive(Debug, Clone, Default)]
pub struct ShaderVariation {
    pub binary_program: BnshShaderProgram,
}

#[derive(Debug, Clone)]
pub struct BnshFile {
    pub bin_header: BinaryHeader,
    pub header: BnshHeader,
    pub name: String,
    pub variations: Vec<ShaderVariation>,
}

#[derive(Debug, Default)]
struct BinWriter {
    output: Vec<u8>,
    saved_header_block_positions: Vec<usize>,
    end_of_block_offset: usize,
    /// Every position that holds a real 64-bit file offset. The `_RLT` table is derived from
    /// this set rather than from hand-placed entries: at load the runtime adds the image base
    /// to each listed slot, so listing a slot that holds 0 fabricates a pointer to the base
    /// address, and the game hangs following it. Recording the offset writes themselves is the
    /// only way to keep the table exactly in step with the layout the writer produced.
    pointer_slots: std::collections::BTreeSet<usize>,
}

#[derive(Debug, Clone)]
struct RelocationEntry {
    position: u32,
    struct_count: u32,
    offset_count: u32,
    padding_count: u32,
}

#[derive(Debug, Clone, Default)]
struct RelocationSection {
    entries: Vec<RelocationEntry>,
    position: u32,
    size: u32,
}

#[derive(Debug)]
struct RelocationTable {
    sections: Vec<RelocationSection>,
    relocation_table_offset_pos: usize,
}

#[derive(Debug, Default, Clone)]
struct StringEntry {
    positions: Vec<usize>,
}

#[derive(Debug, Default)]
struct StringTable {
    strings: HashMap<String, StringEntry>,
    file_name_offset_pos: usize,
    file_name: String,
}

#[derive(Debug, Clone)]
struct DictNode {
    reference: u32,
    left_index: u16,
    right_index: u16,
    key: String,
}

#[derive(Debug, Clone, Default)]
struct DictTreeNode {
    bit_index: i32,
    data: BigUint,
    key: String,
    child: [usize; 2],
    parent: usize,
}

impl BinWriter {
    fn position(&self) -> usize {
        self.output.len()
    }

    fn write_u8(&mut self, value: u8) {
        self.output.push(value);
    }

    fn write_u16(&mut self, value: u16) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
    }

    fn write_signature(&mut self, magic: &str) {
        self.write_bytes(magic.as_bytes());
    }

    fn write_zeroes(&mut self, len: usize) {
        self.output.resize(self.output.len() + len, 0);
    }

    fn align_bytes(&mut self, align: usize) {
        let padding = (align - (self.output.len() % align)) % align;
        self.write_zeroes(padding);
    }

    fn patch_u32(&mut self, offset: usize, value: u32) {
        self.output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn patch_u64(&mut self, offset: usize, value: u64) {
        self.output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn save_offset(&mut self) -> usize {
        let offset = self.position();
        self.write_u64(0);
        offset
    }

    fn write_offset(&mut self, offset_pos: usize) {
        self.write_offset_to(offset_pos, self.position());
    }

    fn write_offset_to(&mut self, offset_pos: usize, target: usize) {
        if offset_pos == 0 {
            return;
        }
        self.patch_u64(offset_pos, target as u64);
        if target != 0 {
            self.pointer_slots.insert(offset_pos);
        }
    }

    /// Write a 64-bit file offset inline (rather than back-patching a reserved slot) and record
    /// it for relocation.
    fn write_pointer(&mut self, target: usize) {
        if target != 0 {
            self.pointer_slots.insert(self.position());
        }
        self.write_u64(target as u64);
    }

    fn save_header_block(&mut self) {
        self.saved_header_block_positions.push(self.position());
        self.write_u32(0);
        self.write_u64(0);
    }

    fn write_header_blocks(&mut self) {
        for index in 0..self.saved_header_block_positions.len() {
            let position = self.saved_header_block_positions[index];
            if index + 1 == self.saved_header_block_positions.len() {
                self.patch_u32(position, 0);
                self.patch_u64(position + 4, (self.end_of_block_offset - position) as u64);
            } else {
                let size = (self.saved_header_block_positions[index + 1]
                    - self.saved_header_block_positions[index]) as u32;
                self.patch_u32(position, size);
                self.patch_u64(position + 4, size as u64);
            }
        }
    }
}

impl RelocationTable {
    fn new(num_sections: usize) -> Self {
        Self {
            sections: vec![RelocationSection::default(); num_sections],
            relocation_table_offset_pos: 0,
        }
    }

    fn save_header_offset(&mut self, writer: &mut BinWriter) {
        self.relocation_table_offset_pos = writer.position();
        writer.write_u32(0);
    }

    fn save_entry(
        &mut self,
        position: usize,
        offset_count: u32,
        struct_count: u32,
        padding_count: u32,
        section_idx: usize,
    ) {
        let idx = if section_idx >= self.sections.len() {
            0
        } else {
            section_idx
        };
        self.sections[idx].entries.push(RelocationEntry {
            position: position as u32,
            struct_count,
            offset_count,
            padding_count,
        });
    }

    fn save_writer_entry(
        &mut self,
        writer: &BinWriter,
        offset_count: u32,
        struct_count: u32,
        padding_count: u32,
        section_idx: usize,
    ) {
        self.save_entry(
            writer.position(),
            offset_count,
            struct_count,
            padding_count,
            section_idx,
        );
    }

    fn set_section(&mut self, section_idx: usize, position: u32, size: u32) {
        self.sections[section_idx].position = position;
        self.sections[section_idx].size = size;
    }

    /// Rebuild every section's entry list from the offsets the writer actually emitted.
    ///
    /// The entries placed by hand while serializing described the layout the writer *intended*,
    /// which drifted from the one it produced: slots holding 0 were listed for relocation (the
    /// runtime then turns them into pointers to the image base and the game hangs dereferencing
    /// them), while genuine offsets were left out. Deriving the table from the recorded writes
    /// makes it exact by construction.
    ///
    /// An entry belongs to the section its pointer *targets*, not the one holding the slot —
    /// each region is relocated against its own base, so a pointer into the bytecode or string
    /// region filed under section 0 would be fixed up against the wrong one. Within a section, a
    /// run of consecutive 8-byte offsets collapses into one entry with `offset_count` slots, and
    /// evenly-spaced runs of equal width collapse further into `struct_count` repeats separated
    /// by `padding_count` 8-byte words, which is how the game's own tables are encoded.
    fn rebuild_from_pointer_slots(
        &mut self,
        slots: &std::collections::BTreeSet<usize>,
        output: &[u8],
    ) {
        for section in self.sections.iter_mut() {
            section.entries.clear();
        }
        let mut by_section: Vec<Vec<usize>> = vec![Vec::new(); self.sections.len()];
        for &slot in slots {
            let Some(raw) = output.get(slot..slot + 8) else {
                continue;
            };
            let target = u64::from_le_bytes(raw.try_into().unwrap()) as usize;
            by_section[self.section_of(target)].push(slot);
        }

        for (section_idx, section_slots) in by_section.into_iter().enumerate() {
            // Runs of adjacent offsets first.
            let mut groups: Vec<(usize, u32)> = Vec::new();
            for slot in section_slots {
                match groups.last_mut() {
                    Some((start, count)) if *start + (*count as usize) * 8 == slot => *count += 1,
                    _ => groups.push((slot, 1)),
                }
            }
            let mut index = 0;
            while index < groups.len() {
                let (position, offset_count) = groups[index];
                let mut struct_count = 1u32;
                let mut padding_count = 0u32;
                // `padding_count` and `offset_count` are u8 on disk and `struct_count` u16, so a
                // fold that overflows them would silently encode a different stride and fabricate
                // slots. Only fold where the encoded form is exact.
                if offset_count <= u8::MAX as u32 && index + 1 < groups.len() {
                    let stride = groups[index + 1].0 - position;
                    let span = offset_count as usize * 8;
                    let padding = stride.saturating_sub(span) / 8;
                    if groups[index + 1].1 == offset_count
                        && stride >= span
                        && stride.is_multiple_of(8)
                        && padding <= u8::MAX as usize
                    {
                        let mut next = index + 1;
                        while next < groups.len()
                            && groups[next].1 == offset_count
                            && groups[next].0 == position + stride * (next - index)
                            && (next - index) < u16::MAX as usize
                        {
                            struct_count += 1;
                            next += 1;
                        }
                        if struct_count > 1 {
                            padding_count = padding as u32;
                            index = next - 1;
                        }
                    }
                }
                self.sections[section_idx].entries.push(RelocationEntry {
                    position: position as u32,
                    struct_count,
                    offset_count,
                    padding_count,
                });
                index += 1;
            }
        }
    }

    /// The section whose region contains `position`; section 0 spans the header block and is the
    /// fallback for anything that falls outside a declared region.
    fn section_of(&self, position: usize) -> usize {
        self.sections
            .iter()
            .position(|section| {
                section.size != 0
                    && position >= section.position as usize
                    && position < (section.position as usize + section.size as usize)
            })
            .unwrap_or(0)
    }

    fn write(&self, writer: &mut BinWriter) {
        writer.align_bytes(8);
        let position = writer.position();
        writer.patch_u32(self.relocation_table_offset_pos, position as u32);
        writer.write_signature("_RLT");
        writer.end_of_block_offset = writer.position();
        writer.write_u32(position as u32);
        writer.write_u32(self.sections.len() as u32);
        writer.write_u32(0);
        let mut entry_start_index = 0i32;
        for section in &self.sections {
            writer.write_u64(0);
            writer.write_u32(section.position);
            writer.write_u32(section.size);
            writer.write_i32(entry_start_index);
            writer.write_i32(section.entries.len() as i32);
            entry_start_index += section.entries.len() as i32;
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

impl StringTable {
    fn add_file_name_entry(&mut self, position: usize, value: &str) {
        self.file_name_offset_pos = position;
        self.file_name = value.to_string();
    }

    fn add_entry(&mut self, position: usize, value: &str) {
        if let Some(entry) = self.strings.get_mut(value) {
            entry.positions.push(position);
            return;
        }
        self.strings.insert(
            value.to_string(),
            StringEntry {
                positions: vec![position],
            },
        );
    }

    fn write(&self, writer: &mut BinWriter) {
        writer.align_bytes(8);
        writer.write_signature("_STR");
        writer.save_header_block();
        let mut entries: Vec<(&str, &StringEntry)> = self
            .strings
            .iter()
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        entries.sort_by(|(left, _), (right, _)| res_string_compare(left, right));
        writer.write_i32(entries.len() as i32);
        if self.file_name_offset_pos != 0 {
            writer.write_u16(self.file_name.len() as u16);
            let string_offset = writer.position();
            writer.patch_u32(self.file_name_offset_pos, string_offset as u32);
            writer.write_bytes(self.file_name.as_bytes());
            writer.write_u8(0);
            writer.align_bytes(4);
        }
        for (key, entry) in entries {
            let string_offset = writer.position();
            for position in &entry.positions {
                // These are 64-bit slots reserved by `save_string`, patched in their low half.
                // The game relocates them like any other pointer, so they must be recorded —
                // the file-name offset above is a plain u32 field and is deliberately not.
                writer.patch_u32(*position, string_offset as u32);
                writer.pointer_slots.insert(*position);
            }
            writer.write_u16(key.len() as u16);
            writer.write_bytes(key.as_bytes());
            writer.write_u8(0);
            writer.align_bytes(4);
        }
    }
}

/// Merge single-variation BNSH exports into one file (matches C# BnshFile.Save after adding variations).
pub fn merge_variation_files(files: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    let mut merged: Option<BnshFile> = None;
    for data in files {
        let file = BnshFile::read(data)?;
        match &mut merged {
            None => merged = Some(file),
            Some(existing) => existing.variations.extend(file.variations),
        }
    }
    Ok(merged.map(|file| file.write()).unwrap_or_default())
}

/// Repopulate a Base.ptcl BNSH container from per-emitter exports (matches C# variation rebuild).
pub fn rebuild_from_base_and_exports(base: &[u8], exports: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    if exports.is_empty() {
        return Ok(Vec::new());
    }
    let mut file = if !base.is_empty() {
        BnshFile::read(base)?
    } else {
        BnshFile::read(&exports[0])?
    };
    file.variations.clear();
    for data in exports {
        let export = BnshFile::read(data)?;
        if let Some(variation) = export.variations.into_iter().next() {
            file.variations.push(variation);
        }
    }
    Ok(file.write())
}

pub fn canonicalize(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() < 4 || &data[..4] != b"BNSH" {
        return Ok(data.to_vec());
    }
    let file = BnshFile::read(data)?;
    Ok(file.write())
}

impl BnshFile {
    pub fn read(data: &[u8]) -> io::Result<Self> {
        let mut reader = Cursor::new(data);
        let bin_header = BinaryHeader {
            magic: reader.read_u64::<LittleEndian>()?,
            version_micro: reader.read_u8()?,
            version_minor: reader.read_u8()?,
            version_major: reader.read_u16::<LittleEndian>()?,
            byte_order: reader.read_u16::<LittleEndian>()?,
            alignment: reader.read_u8()?,
            target_address_size: reader.read_u8()?,
            name_offset: reader.read_u32::<LittleEndian>()?,
            flag: reader.read_u16::<LittleEndian>()?,
            block_offset: reader.read_u16::<LittleEndian>()?,
            relocation_table_offset: reader.read_u32::<LittleEndian>()?,
            file_size: reader.read_u32::<LittleEndian>()?,
        };

        let name = if bin_header.name_offset >= 2 {
            read_len_string_at(data, (bin_header.name_offset - 2) as usize)?
        } else {
            String::new()
        };

        reader.seek(SeekFrom::Start(bin_header.block_offset as u64))?;
        let magic = reader.read_u32::<LittleEndian>()?;
        if magic != u32::from_le_bytes(*b"grsc") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid grsc header",
            ));
        }
        let block_size = reader.read_u32::<LittleEndian>()?;
        let block_offset_field = reader.read_u32::<LittleEndian>()?;
        let padding = reader.read_u32::<LittleEndian>()?;
        let api_type = reader.read_u16::<LittleEndian>()?;
        let api_version = reader.read_u16::<LittleEndian>()?;
        let code_target = reader.read_u32::<LittleEndian>()?;
        let compiler_version = reader.read_u32::<LittleEndian>()?;
        let variation_count = reader.read_u32::<LittleEndian>()?;
        let variation_start_offset = reader.read_u64::<LittleEndian>()?;
        let memory_pool_offset = reader.read_u64::<LittleEndian>()?;
        let unknown2 = reader.read_u64::<LittleEndian>()?;
        reader.seek(SeekFrom::Current(40))?;

        let header = BnshHeader {
            magic,
            block_offset: block_offset_field,
            block_size,
            padding,
            api_type,
            api_version,
            code_target,
            compiler_version,
            num_variation: variation_count,
            variation_start_offset,
            memory_pool_offset,
            unknown2,
        };

        let mut variations = Vec::with_capacity(variation_count as usize);
        for index in 0..variation_count as usize {
            let variation_offset = variation_start_offset as usize + index * 64;
            variations.push(ShaderVariation {
                binary_program: read_program(data, variation_offset)?,
            });
        }

        Ok(Self {
            bin_header,
            header,
            name,
            variations,
        })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut writer = BinWriter::default();
        let mut relocation_table = RelocationTable::new(6);
        let mut string_table = StringTable::default();

        writer.write_u64(self.bin_header.magic);
        writer.write_u8(self.bin_header.version_micro);
        writer.write_u8(self.bin_header.version_minor);
        writer.write_u16(self.bin_header.version_major);
        writer.write_u16(self.bin_header.byte_order);
        writer.write_u8(self.bin_header.alignment);
        writer.write_u8(self.bin_header.target_address_size);
        let file_name_offset_pos = writer.position();
        writer.write_u32(0);
        string_table.add_file_name_entry(file_name_offset_pos, &self.name);
        writer.write_u16(self.bin_header.flag);
        writer.write_u16(0x60);
        relocation_table.save_header_offset(&mut writer);
        let file_size_pos = writer.position();
        writer.write_u32(0);
        writer.write_zeroes(64);

        let grsc_start = writer.position();
        writer.write_signature("grsc");
        writer.save_header_block();
        writer.write_u16(self.header.api_type);
        writer.write_u16(self.header.api_version);
        writer.write_u32(self.header.code_target);
        writer.write_u32(self.header.compiler_version);
        writer.write_u32(self.variations.len() as u32);
        relocation_table.save_writer_entry(&writer, 2, 1, 0, 0);
        let variation_start_offset_pos = writer.save_offset();
        let pool_header_offset_pos = writer.save_offset();
        writer.write_u64(self.header.unknown2);
        writer.write_zeroes(40);

        let program_headers_offset = writer.position();
        writer.write_offset(variation_start_offset_pos);
        relocation_table.save_entry(
            writer.position() + 16,
            2,
            self.variations.len() as u32,
            6,
            0,
        );

        for _ in &self.variations {
            writer.write_zeroes(24);
            writer.write_pointer(grsc_start);
            writer.write_zeroes(32);
        }

        let mut stage_byte_code_offsets = vec![[0usize; SHADER_STAGE_COUNT]; self.variations.len()];
        let mut object_data_offset_positions = vec![0usize; self.variations.len()];
        let mut reflection_program_offset_positions = vec![0usize; self.variations.len()];

        let mut control_code_blocks: HashMap<Vec<u8>, usize> = HashMap::new();

        for (variation_index, variation) in self.variations.iter().enumerate() {
            writer.align_bytes(8);
            writer.write_offset(program_headers_offset + variation_index * 64 + 16);
            let program = &variation.binary_program;

            writer.write_u8(program.flags);
            writer.write_u8(program.code_type);
            writer.write_u8(program.format);
            writer.write_u8(program.padding);
            writer.write_u32(program.binary_format);

            let mut stage_offset_positions = [0usize; SHADER_STAGE_COUNT];
            relocation_table.save_writer_entry(&writer, 6, 1, 0, 0);
            for item in &mut stage_offset_positions {
                *item = writer.save_offset();
            }

            writer.write_zeroes(40);
            writer.write_u32(program.memory_data.len() as u32);
            writer.write_u32(0);
            relocation_table.save_writer_entry(&writer, 1, 1, 0, 1);
            object_data_offset_positions[variation_index] = writer.save_offset();

            relocation_table.save_writer_entry(&writer, 2, 1, 0, 0);
            writer.write_pointer(program_headers_offset + variation_index * 64);
            let reflection_stage_array_offset_pos = writer.save_offset();
            reflection_program_offset_positions[variation_index] =
                reflection_stage_array_offset_pos;
            writer.write_zeroes(32);

            let mut control_code_offset_positions = [0usize; SHADER_STAGE_COUNT];
            let program_stage_headers_start = writer.position();

            for stage_index in [0usize, 3, 4, 5] {
                if let Some(code) = &program.stages[stage_index] {
                    writer.write_offset(stage_offset_positions[stage_index]);
                    writer.write_u64(0);
                    control_code_offset_positions[stage_index] = writer.save_offset();
                    stage_byte_code_offsets[variation_index][stage_index] = writer.save_offset();
                    writer.write_u32(code.byte_code.len() as u32);
                    writer.write_u32(code.control_code.len() as u32);
                    writer.write_bytes(&code.reserved);
                }
            }

            let struct_count = ((writer.position() - program_stage_headers_start)
                / SHADER_STAGE_HEADER_SIZE) as u32;
            relocation_table.save_entry(program_stage_headers_start + 8, 1, struct_count, 7, 0);
            relocation_table.save_entry(program_stage_headers_start + 16, 1, struct_count, 7, 4);

            let has_reflection = program.reflections[0].is_some()
                || program.reflections[1].is_some()
                || program.reflections[2].is_some()
                || program.reflections[3].is_some()
                || program.reflections[4].is_some()
                || program.reflections[5].is_some();
            let mut reflection_stage_offset_positions = [0usize; SHADER_STAGE_COUNT];
            if has_reflection {
                writer.write_offset(reflection_stage_array_offset_pos);
                relocation_table.save_writer_entry(&writer, 6, 1, 0, 0);
                for item in &mut reflection_stage_offset_positions {
                    *item = writer.save_offset();
                }
                writer.write_zeroes(16);
            }

            for (code, &control_code_offset_position) in
                program.stages.iter().zip(&control_code_offset_positions)
            {
                if let Some(code) = code {
                    if !code.control_code.is_empty() {
                        if let Some(saved_offset) = control_code_blocks.get(&code.control_code) {
                            writer.write_offset_to(control_code_offset_position, *saved_offset);
                        } else {
                            writer.align_bytes(8);
                            let offset = writer.position();
                            control_code_blocks.insert(code.control_code.clone(), offset);
                            writer.write_offset(control_code_offset_position);
                            writer.write_bytes(&code.control_code);
                        }
                    }
                }
            }

            let mut pending_reflections = Vec::new();
            for (reflection, &reflection_stage_offset_position) in program
                .reflections
                .iter()
                .zip(&reflection_stage_offset_positions)
            {
                let Some(reflection) = reflection else {
                    continue;
                };
                writer.align_bytes(8);
                writer.write_offset(reflection_stage_offset_position);
                relocation_table.save_writer_entry(&writer, 5, 1, 0, 0);
                let input_offset_pos = writer.save_offset();
                let output_offset_pos = writer.save_offset();
                let sampler_offset_pos = writer.save_offset();
                let cbuf_offset_pos = writer.save_offset();
                let uab_offset_pos = writer.save_offset();
                writer.write_i32(reflection.header.output_idx);
                writer.write_i32(reflection.header.sampler_idx);
                writer.write_i32(reflection.header.const_buffer_idx);
                writer.write_i32(reflection.header.unordered_access_buffer_idx);
                relocation_table.save_writer_entry(&writer, 1, 1, 0, 0);
                let slot_offset_pos = writer.position();
                writer.write_u32(0);
                writer.write_i32(reflection.header.compute_work_group_x);
                writer.write_i32(reflection.header.compute_work_group_y);
                writer.write_i32(reflection.header.compute_work_group_z);
                writer.write_i32(reflection.header.image_idx);
                writer.write_i32(reflection.slots.len() as i32);
                writer.write_u64(0);
                writer.write_u64(0);
                pending_reflections.push(PendingReflection {
                    data: reflection,
                    input_offset_pos,
                    output_offset_pos,
                    sampler_offset_pos,
                    cbuf_offset_pos,
                    uab_offset_pos,
                    slot_offset_pos,
                });
            }

            if !pending_reflections.is_empty() {
                writer.align_bytes(8);
            }
            for reflection in pending_reflections {
                write_dictionary(
                    &mut writer,
                    &mut relocation_table,
                    &mut string_table,
                    &reflection.data.inputs,
                    reflection.input_offset_pos,
                );
                write_dictionary(
                    &mut writer,
                    &mut relocation_table,
                    &mut string_table,
                    &reflection.data.outputs,
                    reflection.output_offset_pos,
                );
                write_dictionary(
                    &mut writer,
                    &mut relocation_table,
                    &mut string_table,
                    &reflection.data.samplers,
                    reflection.sampler_offset_pos,
                );
                write_dictionary(
                    &mut writer,
                    &mut relocation_table,
                    &mut string_table,
                    &reflection.data.constant_buffers,
                    reflection.cbuf_offset_pos,
                );
                write_dictionary(
                    &mut writer,
                    &mut relocation_table,
                    &mut string_table,
                    &reflection.data.unordered_access_buffers,
                    reflection.uab_offset_pos,
                );
                if !reflection.data.slots.is_empty() {
                    writer.align_bytes(8);
                    let slot_position = writer.position();
                    // A 64-bit slot patched in its low half, like the string-table offsets.
                    writer.patch_u32(reflection.slot_offset_pos, slot_position as u32);
                    writer.pointer_slots.insert(reflection.slot_offset_pos);
                    for slot in &reflection.data.slots {
                        writer.write_i32(*slot);
                    }
                }
            }
        }

        writer.align_bytes(8);
        writer.write_offset(pool_header_offset_pos);
        writer.write_u32(97);
        let pool_size_pos = writer.position();
        writer.write_u32(0);
        relocation_table.save_writer_entry(&writer, 1, 1, 0, 4);
        let pool_buffer_offset_pos = writer.save_offset();
        writer.write_zeroes(16);
        relocation_table.save_writer_entry(&writer, 1, 1, 0, 0);
        let pool_offset_pos = writer.save_offset();
        writer.write_zeroes(40);
        writer.write_offset(pool_offset_pos);
        writer.write_zeroes(320);

        relocation_table.set_section(0, 0, writer.position() as u32);
        let object_data_start = writer.position();
        for (variation_index, variation) in self.variations.iter().enumerate() {
            writer.align_bytes(4);
            writer.write_offset(object_data_offset_positions[variation_index]);
            writer.write_bytes(&variation.binary_program.memory_data);
        }
        let object_data_size = (writer.position() - object_data_start) as u32;
        // The section base is the START of the object data; using the end shifted every
        // relocation in this section onto the wrong region.
        relocation_table.set_section(1, object_data_start as u32, object_data_size);
        relocation_table.set_section(2, writer.position() as u32, 0);
        relocation_table.set_section(3, writer.position() as u32, 0);

        // Align to 4096 bytes before bytecode section (matching C# BnshSaver.cs line 341)
        writer.align_bytes(4096);
        let byte_code_start = writer.position();
        writer.write_offset(pool_buffer_offset_pos);
        let mut byte_code_blocks: HashMap<Vec<u8>, usize> = HashMap::new();
        for (variation_index, variation) in self.variations.iter().enumerate() {
            for stage_index in [0usize, 3, 4, 5] {
                if let Some(code) = &variation.binary_program.stages[stage_index] {
                    if code.byte_code.is_empty() {
                        continue;
                    }
                    let offset_save = stage_byte_code_offsets[variation_index][stage_index];
                    if let Some(saved_offset) = byte_code_blocks.get(&code.byte_code) {
                        writer.write_offset_to(offset_save, *saved_offset);
                    } else {
                        writer.align_bytes(8);
                        let offset = writer.position();
                        byte_code_blocks.insert(code.byte_code.clone(), offset);
                        writer.write_offset(offset_save);
                        writer.write_bytes(&code.byte_code);
                    }
                }
            }
        }
        writer.align_bytes(1usize << self.bin_header.alignment);
        let byte_code_end = writer.position();
        let byte_code_size = (byte_code_end - byte_code_start) as u32;
        writer.patch_u32(pool_size_pos, byte_code_size);
        relocation_table.set_section(4, byte_code_start as u32, byte_code_size);

        writer.align_bytes(8);
        let grsc_size = (writer.position() - grsc_start) as u32;
        writer.patch_u32(grsc_start + 4, grsc_size);
        writer.patch_u32(grsc_start + 8, grsc_size);

        let string_table_start = writer.position();
        string_table.write(&mut writer);
        relocation_table.set_section(
            5,
            string_table_start as u32,
            (writer.position() - string_table_start) as u32,
        );
        // Sections are final, so the relocation entries can now be derived from the offsets that
        // were actually written (see `rebuild_from_pointer_slots`).
        let pointer_slots = std::mem::take(&mut writer.pointer_slots);
        relocation_table.rebuild_from_pointer_slots(&pointer_slots, &writer.output);
        relocation_table.write(&mut writer);
        writer.write_header_blocks();

        writer.patch_u32(file_size_pos, writer.position() as u32);
        writer.output
    }
}

fn read_program(data: &[u8], variation_header_offset: usize) -> io::Result<BnshShaderProgram> {
    let mut reader = Cursor::new(data);
    reader.seek(SeekFrom::Start(variation_header_offset as u64 + 16))?;
    let binary_offset = reader.read_u64::<LittleEndian>()? as usize;
    reader.seek(SeekFrom::Start(binary_offset as u64))?;

    let flags = reader.read_u8()?;
    let code_type = reader.read_u8()?;
    let format = reader.read_u8()?;
    let padding = reader.read_u8()?;
    let binary_format = reader.read_u32::<LittleEndian>()?;

    let mut stage_offsets = [0u64; SHADER_STAGE_COUNT];
    for item in &mut stage_offsets {
        *item = reader.read_u64::<LittleEndian>()?;
    }

    reader.seek(SeekFrom::Current(40))?;
    let object_size = reader.read_u32::<LittleEndian>()?;
    let _object_padding = reader.read_u32::<LittleEndian>()?;
    let object_offset = reader.read_u64::<LittleEndian>()?;
    let _parent_variation_offset = reader.read_u64::<LittleEndian>()?;
    let shader_reflection_offset = reader.read_u64::<LittleEndian>()?;
    reader.seek(SeekFrom::Current(32))?;

    let mut memory_data = vec![0; object_size as usize];
    if object_size > 0 {
        let mut object_reader = Cursor::new(data);
        object_reader.seek(SeekFrom::Start(object_offset))?;
        object_reader.read_exact(&mut memory_data)?;
    }

    let mut stages: [Option<ShaderCode>; SHADER_STAGE_COUNT] = Default::default();
    for stage_index in 0..SHADER_STAGE_COUNT {
        if stage_offsets[stage_index] != 0 {
            stages[stage_index] =
                Some(read_shader_code(data, stage_offsets[stage_index] as usize)?);
        }
    }

    let mut reflections: [Option<ShaderReflectionData>; SHADER_STAGE_COUNT] = Default::default();
    if shader_reflection_offset != 0 {
        let mut reflection_reader = Cursor::new(data);
        reflection_reader.seek(SeekFrom::Start(shader_reflection_offset))?;
        let mut reflection_offsets = [0u64; SHADER_STAGE_COUNT];
        for item in &mut reflection_offsets {
            *item = reflection_reader.read_u64::<LittleEndian>()?;
        }
        for stage_index in 0..SHADER_STAGE_COUNT {
            if reflection_offsets[stage_index] != 0 {
                reflections[stage_index] = Some(read_shader_reflection(
                    data,
                    reflection_offsets[stage_index] as usize,
                )?);
            }
        }
    }

    Ok(BnshShaderProgram {
        flags,
        code_type,
        format,
        padding,
        binary_format,
        memory_data,
        stages,
        reflections,
    })
}

fn read_shader_code(data: &[u8], offset: usize) -> io::Result<ShaderCode> {
    let mut reader = Cursor::new(data);
    reader.seek(SeekFrom::Start(offset as u64))?;
    reader.seek(SeekFrom::Current(8))?;
    let control_code_offset = reader.read_u64::<LittleEndian>()? as usize;
    let byte_code_offset = reader.read_u64::<LittleEndian>()? as usize;
    let byte_code_size = reader.read_u32::<LittleEndian>()? as usize;
    let control_code_size = reader.read_u32::<LittleEndian>()? as usize;
    let mut reserved = [0u8; 32];
    reader.read_exact(&mut reserved)?;

    let control_code = if control_code_offset != 0 && control_code_size != 0 {
        data[control_code_offset..control_code_offset + control_code_size].to_vec()
    } else {
        Vec::new()
    };
    let byte_code = if byte_code_offset != 0 && byte_code_size != 0 {
        data[byte_code_offset..byte_code_offset + byte_code_size].to_vec()
    } else {
        Vec::new()
    };

    Ok(ShaderCode {
        control_code,
        byte_code,
        reserved,
    })
}

fn read_shader_reflection(data: &[u8], offset: usize) -> io::Result<ShaderReflectionData> {
    let mut reader = Cursor::new(data);
    reader.seek(SeekFrom::Start(offset as u64))?;
    let input_offset = reader.read_u64::<LittleEndian>()? as usize;
    let output_offset = reader.read_u64::<LittleEndian>()? as usize;
    let sampler_offset = reader.read_u64::<LittleEndian>()? as usize;
    let cbuf_offset = reader.read_u64::<LittleEndian>()? as usize;
    let uab_offset = reader.read_u64::<LittleEndian>()? as usize;
    let output_idx = reader.read_i32::<LittleEndian>()?;
    let sampler_idx = reader.read_i32::<LittleEndian>()?;
    let const_buffer_idx = reader.read_i32::<LittleEndian>()?;
    let unordered_access_buffer_idx = reader.read_i32::<LittleEndian>()?;
    let slot_offset = reader.read_u32::<LittleEndian>()? as usize;
    let compute_work_group_x = reader.read_i32::<LittleEndian>()?;
    let compute_work_group_y = reader.read_i32::<LittleEndian>()?;
    let compute_work_group_z = reader.read_i32::<LittleEndian>()?;
    let image_idx = reader.read_i32::<LittleEndian>()?;
    let slot_count = reader.read_i32::<LittleEndian>()? as usize;
    reader.seek(SeekFrom::Current(16))?;

    let slots = if slot_count > 0 && slot_offset != 0 {
        let mut slot_reader = Cursor::new(data);
        slot_reader.seek(SeekFrom::Start(slot_offset as u64))?;
        let mut values = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            values.push(slot_reader.read_i32::<LittleEndian>()?);
        }
        values
    } else {
        Vec::new()
    };

    Ok(ShaderReflectionData {
        header: ShaderReflectionHeader {
            output_idx,
            sampler_idx,
            const_buffer_idx,
            unordered_access_buffer_idx,
            compute_work_group_x,
            compute_work_group_y,
            compute_work_group_z,
            image_idx,
        },
        inputs: read_dictionary_keys(data, input_offset)?,
        outputs: read_dictionary_keys(data, output_offset)?,
        samplers: read_dictionary_keys(data, sampler_offset)?,
        constant_buffers: read_dictionary_keys(data, cbuf_offset)?,
        unordered_access_buffers: read_dictionary_keys(data, uab_offset)?,
        slots,
    })
}

fn read_dictionary_keys(data: &[u8], offset: usize) -> io::Result<Vec<String>> {
    if offset == 0 {
        return Ok(Vec::new());
    }
    let mut reader = Cursor::new(data);
    reader.seek(SeekFrom::Start(offset as u64))?;
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"_DIC" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid dictionary",
        ));
    }
    let count = reader.read_i32::<LittleEndian>()?;
    let mut keys = Vec::new();
    for index in 0..=count {
        let _reference = reader.read_u32::<LittleEndian>()?;
        let _left = reader.read_u16::<LittleEndian>()?;
        let _right = reader.read_u16::<LittleEndian>()?;
        let string_offset = reader.read_u64::<LittleEndian>()? as usize;
        if index > 0 {
            keys.push(read_len_string_at(data, string_offset)?);
        }
    }
    Ok(keys)
}

fn read_len_string_at(data: &[u8], offset: usize) -> io::Result<String> {
    if offset + 2 > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "string length out of range",
        ));
    }
    let len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
    let start = offset + 2;
    let end = start + len;
    if end > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "string bytes out of range",
        ));
    }
    String::from_utf8(data[start..end].to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid utf-8 string"))
}

fn write_dictionary(
    writer: &mut BinWriter,
    relocation_table: &mut RelocationTable,
    string_table: &mut StringTable,
    keys: &[String],
    offset_pos: usize,
) {
    if keys.is_empty() {
        return;
    }
    writer.align_bytes(8);
    writer.write_offset(offset_pos);
    let nodes = generate_dict_nodes(keys);
    writer.write_signature("_DIC");
    writer.write_i32(nodes.len() as i32 - 1);
    for (index, node) in nodes.iter().enumerate() {
        writer.write_u32(node.reference);
        writer.write_u16(node.left_index);
        writer.write_u16(node.right_index);
        if index == 0 {
            relocation_table.save_writer_entry(writer, 1, nodes.len() as u32, 1, 5);
            save_string(writer, string_table, "");
        } else {
            save_string(writer, string_table, &node.key);
        }
    }
}

fn save_string(writer: &mut BinWriter, string_table: &mut StringTable, value: &str) {
    let offset = writer.save_offset();
    string_table.add_entry(offset, value);
}

fn generate_dict_nodes(keys: &[String]) -> Vec<DictNode> {
    let mut tree_nodes = vec![DictTreeNode::default()];
    tree_nodes[0].bit_index = -1;
    tree_nodes[0].parent = 0;
    tree_nodes[0].child = [0, 0];
    let mut entry_order = vec![BigUint::default()];
    let mut entry_indexes = HashMap::new();
    entry_indexes.insert(BigUint::default(), 0usize);

    for key in keys {
        let data = BigUint::from_bytes_be(key.as_bytes());
        insert_dict_node(
            &mut tree_nodes,
            &mut entry_order,
            &mut entry_indexes,
            key,
            data,
        );
    }

    let mut nodes = vec![
        DictNode {
            reference: 0,
            left_index: 0,
            right_index: 0,
            key: String::new(),
        };
        keys.len() + 1
    ];

    for (output_index, data) in entry_order.iter().enumerate() {
        let node_index = *entry_indexes.get(data).unwrap();
        let node = &tree_nodes[node_index];
        nodes[output_index] = DictNode {
            reference: compact_bit_index(node.bit_index),
            left_index: *entry_indexes.get(&tree_nodes[node.child[0]].data).unwrap() as u16,
            right_index: *entry_indexes.get(&tree_nodes[node.child[1]].data).unwrap() as u16,
            key: node.key.clone(),
        };
    }
    nodes[0].key.clear();
    nodes
}

fn insert_dict_node(
    tree_nodes: &mut Vec<DictTreeNode>,
    entry_order: &mut Vec<BigUint>,
    entry_indexes: &mut HashMap<BigUint, usize>,
    key: &str,
    data: BigUint,
) {
    let mut node_index = search_node(tree_nodes, &data, true);
    let mismatch = bit_mismatch(&tree_nodes[node_index].data, &data);
    while mismatch < tree_nodes[tree_nodes[node_index].parent].bit_index {
        node_index = tree_nodes[node_index].parent;
    }

    if mismatch < tree_nodes[node_index].bit_index {
        let parent = tree_nodes[node_index].parent;
        let new_index = tree_nodes.len();
        let mut new_node = DictTreeNode {
            bit_index: mismatch,
            data: data.clone(),
            key: key.to_string(),
            child: [new_index, new_index],
            parent,
        };
        new_node.child[bit_at(&data, mismatch) ^ 1] = node_index;
        tree_nodes.push(new_node);
        let parent_branch = bit_at(&data, tree_nodes[parent].bit_index);
        tree_nodes[parent].child[parent_branch] = new_index;
        tree_nodes[node_index].parent = new_index;
        entry_indexes.insert(data.clone(), entry_order.len());
        entry_order.push(data);
        return;
    }

    if mismatch > tree_nodes[node_index].bit_index {
        let new_index = tree_nodes.len();
        let mut new_node = DictTreeNode {
            bit_index: mismatch,
            data: data.clone(),
            key: key.to_string(),
            child: [new_index, new_index],
            parent: node_index,
        };
        if bit_at(&tree_nodes[node_index].data, mismatch) == (bit_at(&data, mismatch) ^ 1) {
            new_node.child[bit_at(&data, mismatch) ^ 1] = node_index;
        } else {
            new_node.child[bit_at(&data, mismatch) ^ 1] = 0;
        }
        tree_nodes.push(new_node);
        let branch = bit_at(&data, tree_nodes[node_index].bit_index);
        tree_nodes[node_index].child[branch] = new_index;
        entry_indexes.insert(data.clone(), entry_order.len());
        entry_order.push(data);
        return;
    }

    let branch = bit_at(&data, mismatch);
    let mut next_bit = first_one_bit(&data);
    let child_index = tree_nodes[node_index].child[branch];
    if child_index != 0 {
        next_bit = bit_mismatch(&tree_nodes[child_index].data, &data);
    }
    let new_index = tree_nodes.len();
    let mut new_node = DictTreeNode {
        bit_index: next_bit,
        data: data.clone(),
        key: key.to_string(),
        child: [new_index, new_index],
        parent: node_index,
    };
    new_node.child[bit_at(&data, next_bit) ^ 1] = child_index;
    tree_nodes.push(new_node);
    tree_nodes[node_index].child[branch] = new_index;
    entry_indexes.insert(data.clone(), entry_order.len());
    entry_order.push(data);
}

fn search_node(tree_nodes: &[DictTreeNode], data: &BigUint, previous: bool) -> usize {
    if tree_nodes[0].child[0] == 0 {
        return 0;
    }
    let mut node = tree_nodes[0].child[0];
    let mut last;
    loop {
        last = node;
        let bit = bit_at(data, tree_nodes[node].bit_index);
        node = tree_nodes[node].child[bit];
        if tree_nodes[node].bit_index <= tree_nodes[last].bit_index {
            break;
        }
    }
    if previous {
        last
    } else {
        node
    }
}

fn bit_at(data: &BigUint, bit_index: i32) -> usize {
    if bit_index < 0 {
        return 0;
    }
    let mask = (data >> bit_index as usize) & BigUint::from(1u8);
    if mask == BigUint::from(0u8) {
        0
    } else {
        1
    }
}

fn first_one_bit(data: &BigUint) -> i32 {
    let bit_len = bit_length(data);
    for bit in 0..bit_len {
        if bit_at(data, bit) == 1 {
            return bit;
        }
    }
    0
}

fn bit_mismatch(left: &BigUint, right: &BigUint) -> i32 {
    let max_bits = bit_length(left).max(bit_length(right));
    for bit in 0..max_bits {
        if bit_at(left, bit) != bit_at(right, bit) {
            return bit;
        }
    }
    -1
}

fn bit_length(data: &BigUint) -> i32 {
    let bits = data.bits();
    if bits == 0 {
        1
    } else {
        bits as i32
    }
}

fn compact_bit_index(bit_index: i32) -> u32 {
    if bit_index < 0 {
        return u32::MAX;
    }
    let byte_index = bit_index / 8;
    ((byte_index << 3) | (bit_index - 8 * byte_index)) as u32
}

fn res_string_compare(left: &str, right: &str) -> std::cmp::Ordering {
    if left == right {
        return std::cmp::Ordering::Equal;
    }
    if left.is_empty() {
        return std::cmp::Ordering::Greater;
    }
    if right.is_empty() {
        return std::cmp::Ordering::Less;
    }
    left.cmp(right)
}
