use serde::Serialize;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::emitter::{EmitterAnimation, EmitterData};
use crate::enums::Version;
use crate::reader::ReaderExt;
use crate::structs::{
    Eft1Texture, Eft2Texture, Emitter, EmitterList, EmitterSet, EmitterSubSection, FtexTexture,
    MaterialInfo, Primitive, ShaderCbuf, ShaderRef, ShaderTextureRef,
};

const NONE_U32: u32 = u32::MAX;
const PTCL_HEADER_SIZE: usize = 32;
const PTCL_BLOCK_OFFSET: usize = 64;
const SECTION_HEADER_SIZE: usize = 32;

#[derive(Debug, Clone)]
struct BinaryHeader {
    magic: u64,
    graphics_api_version: u16,
    vfx_version: u16,
    byte_order: u16,
    alignment: u8,
    target_address_size: u8,
    name_offset: u32,
    flag: u16,
    block_offset: u16,
    relocation_table_offset: u32,
    file_size: u32,
}

#[derive(Debug, Clone)]
struct SectionHeader {
    magic: String,
    size: u32,
    children_offset: u32,
    next_section_offset: u32,
    attr_offset: u32,
    binary_offset: u32,
    #[allow(dead_code)]
    padding: u32,
    #[allow(dead_code)]
    children_count: u16,
    #[allow(dead_code)]
    unknown: u16,
}

impl SectionHeader {
    fn read<R: Read + Seek>(reader: &mut R) -> io::Result<(Self, u64)> {
        let start = reader.stream_position()?;
        Ok((
            SectionHeader {
                magic: reader.read_magic(4)?,
                size: reader.read_u32_le()?,
                children_offset: reader.read_u32_le()?,
                next_section_offset: reader.read_u32_le()?,
                attr_offset: reader.read_u32_le()?,
                binary_offset: reader.read_u32_le()?,
                padding: reader.read_u32_le()?,
                children_count: reader.read_u16_le()?,
                unknown: reader.read_u16_le()?,
            },
            start,
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextureInfo {
    pub descriptors: Vec<TextureDescriptor>,
    pub binary_data: Option<Vec<u8>>,
    pub section_offset: u64,
    #[serde(skip)]
    pub desc_table_magic: [u8; 4],
}

#[derive(Debug, Clone, Serialize)]
pub struct TextureDescriptor {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShaderInfo {
    pub binary_data: Option<Vec<u8>>,
    pub compute_binary: Option<Vec<u8>>,
    pub section_offset: u64,
    pub compute_section_offset: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variations: Vec<ShaderVariation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShaderVariation {
    pub name: String,
    pub binary_data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrimitiveInfo {
    pub descriptors: Vec<PrimitiveDescriptor>,
    pub binary_data: Option<Vec<u8>>,
    pub section_offset: u64,
    #[serde(skip)]
    pub desc_table_magic: [u8; 4],
    /// Write `binary_data` back out verbatim instead of re-encoding it through
    /// [`crate::bfres::ResFile::canonicalize`].
    ///
    /// Off by default, and normally best left that way: canonicalizing produces the layout
    /// the game itself writes (see the corpus fidelity tests), so it is the safer output for
    /// a pool that has been edited at all. Turn it on when a caller has a container it must
    /// ship byte-for-byte — one this crate does not model fully, say — and accepts that no
    /// structural edit to the pool will be reflected.
    #[serde(skip)]
    pub preserve_binary_data: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrimitiveDescriptor {
    pub id: u64,
    pub position_index: i8,
    pub normal_index: i8,
    pub tangent_index: i8,
    pub color_index: i8,
    pub tex_coord0_index: i8,
    pub tex_coord1_index: i8,
    pub padding: u16,
}

#[derive(Debug, Clone)]
struct RawPrimitiveSection {
    raw_binary: Vec<u8>,
}

#[derive(Debug, Clone)]
enum TopLevelSection {
    EmitterList,
    TextureInfo,
    PrimitiveList,
    PrimitiveInfo,
    ShaderInfo,
    Raw(Vec<u8>),
}

#[derive(Debug, Clone, Serialize)]
pub struct PtclFile {
    #[serde(skip)]
    pub base_bytes: Vec<u8>,
    #[serde(skip)]
    pub file_size: u32,
    #[serde(skip)]
    pub magic: u64,
    #[serde(skip)]
    pub graphics_api_version: u16,
    #[serde(skip)]
    pub alignment: u8,
    #[serde(skip)]
    pub name_offset: u32,
    #[serde(skip)]
    pub flag: u16,
    #[serde(skip)]
    pub block_offset: u16,
    #[serde(skip)]
    pub relocation_table_offset: u32,
    #[serde(skip)]
    pub name: String,
    #[serde(skip)]
    pub byte_order: u16,
    #[serde(rename = "VersionNum")]
    pub version_num: Version,
    #[serde(rename = "VFXVersion")]
    pub vfx_version: u16,
    #[serde(rename = "IsVersion64Bit")]
    pub is_version_64_bit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primitives: Option<Vec<Primitive>>,
    pub materials: Vec<MaterialInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textures: Option<Vec<Eft1Texture>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textures_eft2: Option<Vec<Eft2Texture>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textures_ftex: Option<Vec<FtexTexture>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shaders: Option<Vec<ShaderRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader_cbuf: Option<Vec<ShaderCbuf>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader_texture_ref: Option<Vec<ShaderTextureRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture_info: Option<TextureInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader_info: Option<ShaderInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primitive_info: Option<PrimitiveInfo>,
    pub emitter_list: EmitterList,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emitter_animation: Option<Vec<EmitterAnimation>>,
    pub emitter_order: Vec<usize>,
    #[serde(skip)]
    primitive_list_sections: Vec<RawPrimitiveSection>,
    #[serde(skip)]
    section_order: Vec<TopLevelSection>,
}

fn read_fixed_string<R: Read>(reader: &mut R, len: usize) -> io::Result<String> {
    Ok(reader.read_string(len)?.trim_end_matches('\0').to_string())
}

fn align_vec(output: &mut Vec<u8>, align: usize, pad: u8) {
    let remainder = output.len() % align;
    if remainder != 0 {
        output.resize(output.len() + (align - remainder), pad);
    }
}

fn write_u32_at(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u16_at(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn desc_table_magic_bytes(magic: &str) -> io::Result<[u8; 4]> {
    if magic == "GTNT" || magic == "G3NT" {
        Ok(magic.as_bytes().try_into().expect("four-byte magic"))
    } else if magic.is_empty() {
        Ok([0; 4])
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected descriptor table magic {magic:?}"),
        ))
    }
}

fn empty_desc_table_magic() -> [u8; 4] {
    [0; 4]
}

fn desc_table_magic_for_export(descriptors_empty: bool, stored: [u8; 4]) -> [u8; 4] {
    if descriptors_empty {
        empty_desc_table_magic()
    } else {
        stored
    }
}

/// Match C# `OrderBy(x => x.Name)` / `OrderTextures()` (en-US culture rules for ASCII names).
fn csharp_descriptor_name_order(left: &str, right: &str) -> std::cmp::Ordering {
    fn char_class(ch: char) -> u8 {
        if ch.is_ascii_digit() {
            1
        } else if ch.is_ascii_alphabetic() {
            2
        } else {
            0
        }
    }

    let mut left_chars = left.chars();
    let mut right_chars = right.chars();
    loop {
        match (left_chars.next(), right_chars.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(left_ch), Some(right_ch)) => {
                let class_cmp = char_class(left_ch).cmp(&char_class(right_ch));
                if class_cmp != std::cmp::Ordering::Equal {
                    return class_cmp;
                }
                let lower_cmp = left_ch
                    .to_ascii_lowercase()
                    .cmp(&right_ch.to_ascii_lowercase());
                if lower_cmp != std::cmp::Ordering::Equal {
                    return lower_cmp;
                }
                if left_ch != right_ch {
                    return left_ch.cmp(&right_ch);
                }
            }
        }
    }
}

fn write_section_header(output: &mut Vec<u8>, magic: &str) -> usize {
    write_section_header_magic(
        output,
        magic.as_bytes().try_into().expect("four-byte magic"),
    )
}

fn write_section_header_magic(output: &mut Vec<u8>, magic: [u8; 4]) -> usize {
    let start = output.len();
    output.extend_from_slice(&magic);
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&NONE_U32.to_le_bytes());
    output.extend_from_slice(&NONE_U32.to_le_bytes());
    output.extend_from_slice(&NONE_U32.to_le_bytes());
    output.extend_from_slice(&NONE_U32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes());
    start
}

fn patch_section_size(output: &mut [u8], section_start: usize, size: u32) {
    write_u32_at(output, section_start + 4, size);
}

fn patch_child_offset(output: &mut [u8], section_start: usize, child_start: usize) {
    write_u32_at(
        output,
        section_start + 8,
        (child_start - section_start) as u32,
    );
}

fn patch_next_offset(output: &mut [u8], section_start: usize, next_start: Option<usize>) {
    let value = next_start
        .map(|next| (next - section_start) as u32)
        .unwrap_or(NONE_U32);
    write_u32_at(output, section_start + 12, value);
}

fn patch_attr_offset(output: &mut [u8], section_start: usize, attr_start: usize) {
    write_u32_at(
        output,
        section_start + 16,
        (attr_start - section_start) as u32,
    );
}

fn patch_binary_offset(output: &mut [u8], section_start: usize, binary_start: usize) {
    write_u32_at(
        output,
        section_start + 20,
        (binary_start - section_start) as u32,
    );
}

fn patch_children_count(output: &mut [u8], section_start: usize, count: u16) {
    write_u16_at(output, section_start + 28, count);
}

fn write_texture_desc_binary(descriptors: &[TextureDescriptor]) -> Vec<u8> {
    let mut output = Vec::new();
    for (idx, descriptor) in descriptors.iter().enumerate() {
        let entry_start = output.len();
        output.extend_from_slice(&descriptor.id.to_le_bytes());
        output.extend_from_slice(&0u32.to_le_bytes());
        output.extend_from_slice(&((descriptor.name.len() + 1) as i32).to_le_bytes());
        output.extend_from_slice(descriptor.name.as_bytes());
        output.extend_from_slice(&0i16.to_le_bytes());
        align_vec(&mut output, 8, 0);
        if idx + 1 < descriptors.len() {
            let next_offset = (output.len() - entry_start) as u32;
            write_u32_at(&mut output, entry_start + 8, next_offset);
        }
    }
    output
}

fn write_primitive_desc_binary(descriptors: &[PrimitiveDescriptor]) -> Vec<u8> {
    let mut output = Vec::new();
    for (idx, descriptor) in descriptors.iter().enumerate() {
        output.extend_from_slice(&descriptor.id.to_le_bytes());
        output.extend_from_slice(
            &(if idx + 1 == descriptors.len() {
                0
            } else {
                24u32
            })
            .to_le_bytes(),
        );
        output.extend_from_slice(&8u32.to_le_bytes());
        output.push(descriptor.position_index as u8);
        output.push(descriptor.normal_index as u8);
        output.push(descriptor.tangent_index as u8);
        output.push(descriptor.color_index as u8);
        output.push(descriptor.tex_coord0_index as u8);
        output.push(descriptor.tex_coord1_index as u8);
        output.extend_from_slice(&descriptor.padding.to_le_bytes());
    }
    output
}

fn parse_texture_desc_table<R: Read + Seek>(
    reader: &mut R,
    ptcl_header: &BinaryHeader,
) -> io::Result<(Vec<TextureDescriptor>, [u8; 4])> {
    let (header, start) = SectionHeader::read(reader)?;
    let desc_table_magic = desc_table_magic_bytes(&header.magic)?;
    if header.binary_offset == NONE_U32 {
        return Ok((Vec::new(), empty_desc_table_magic()));
    }

    let mut descriptors = Vec::new();
    let end = start + header.binary_offset as u64 + header.size as u64;
    reader.seek(SeekFrom::Start(start + header.binary_offset as u64))?;
    while reader.stream_position()? < end {
        let entry_start = reader.stream_position()?;
        let id = reader.read_u64_le()?;
        let next_offset = reader.read_u32_le()?;
        let name_len = reader.read_i32_le()? as usize;
        let name = read_fixed_string(reader, name_len)?;
        descriptors.push(TextureDescriptor { id, name });
        if next_offset == 0 {
            break;
        }
        reader.seek(SeekFrom::Start(entry_start + next_offset as u64))?;
    }

    let _ = ptcl_header;
    Ok((descriptors, desc_table_magic))
}

fn parse_primitive_desc_table<R: Read + Seek>(
    reader: &mut R,
) -> io::Result<(Vec<PrimitiveDescriptor>, [u8; 4])> {
    let (header, start) = SectionHeader::read(reader)?;
    let desc_table_magic = desc_table_magic_bytes(&header.magic)?;
    if header.binary_offset == NONE_U32 {
        return Ok((Vec::new(), empty_desc_table_magic()));
    }

    let mut descriptors = Vec::new();
    let end = start + header.binary_offset as u64 + header.size as u64;
    reader.seek(SeekFrom::Start(start + header.binary_offset as u64))?;
    while reader.stream_position()? < end {
        let entry_start = reader.stream_position()?;
        let id = reader.read_u64_le()?;
        let next_offset = reader.read_u32_le()?;
        let _always_eight = reader.read_u32_le()?;
        let position_index = reader.read_u8()? as i8;
        let normal_index = reader.read_u8()? as i8;
        let tangent_index = reader.read_u8()? as i8;
        let color_index = reader.read_u8()? as i8;
        let tex_coord0_index = reader.read_u8()? as i8;
        let tex_coord1_index = reader.read_u8()? as i8;
        let padding = reader.read_u16_le()?;
        descriptors.push(PrimitiveDescriptor {
            id,
            position_index,
            normal_index,
            tangent_index,
            color_index,
            tex_coord0_index,
            tex_coord1_index,
            padding,
        });
        if next_offset == 0 {
            break;
        }
        reader.seek(SeekFrom::Start(entry_start + next_offset as u64))?;
    }
    Ok((descriptors, desc_table_magic))
}

fn parse_texture_info<R: Read + Seek>(
    reader: &mut R,
    header: &SectionHeader,
    section_start: u64,
    ptcl_header: &BinaryHeader,
) -> io::Result<TextureInfo> {
    let (descriptors, desc_table_magic) = if header.children_offset != NONE_U32 {
        reader.seek(SeekFrom::Start(
            section_start + header.children_offset as u64,
        ))?;
        parse_texture_desc_table(reader, ptcl_header)?
    } else {
        (Vec::new(), *b"GTNT")
    };

    let binary_data = if header.binary_offset != NONE_U32 && header.size > 0 {
        reader.seek(SeekFrom::Start(section_start + header.binary_offset as u64))?;
        Some(reader.read_bytes(header.size as usize)?)
    } else {
        None
    };

    Ok(TextureInfo {
        descriptors,
        binary_data,
        section_offset: section_start,
        desc_table_magic,
    })
}

fn parse_shader_info<R: Read + Seek>(
    reader: &mut R,
    header: &SectionHeader,
    section_start: u64,
) -> io::Result<ShaderInfo> {
    let binary_data = if header.binary_offset != NONE_U32 && header.size > 0 {
        reader.seek(SeekFrom::Start(section_start + header.binary_offset as u64))?;
        Some(reader.read_bytes(header.size as usize)?)
    } else {
        None
    };

    let (compute_binary, compute_section_offset) = if header.children_offset != NONE_U32 {
        reader.seek(SeekFrom::Start(
            section_start + header.children_offset as u64,
        ))?;
        let (compute_header, compute_start) = SectionHeader::read(reader)?;
        if compute_header.magic != "GRSC" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected GRSC"));
        }
        let binary = if compute_header.binary_offset != NONE_U32 && compute_header.size > 0 {
            reader.seek(SeekFrom::Start(
                compute_start + compute_header.binary_offset as u64,
            ))?;
            Some(reader.read_bytes(compute_header.size as usize)?)
        } else {
            None
        };
        (binary, Some(compute_start))
    } else {
        (None, None)
    };

    Ok(ShaderInfo {
        binary_data,
        compute_binary,
        section_offset: section_start,
        compute_section_offset,
        variations: Vec::new(),
    })
}

fn parse_primitive_info<R: Read + Seek>(
    reader: &mut R,
    header: &SectionHeader,
    section_start: u64,
) -> io::Result<PrimitiveInfo> {
    let (descriptors, desc_table_magic) = if header.children_offset != NONE_U32 {
        reader.seek(SeekFrom::Start(
            section_start + header.children_offset as u64,
        ))?;
        parse_primitive_desc_table(reader)?
    } else if header.children_count >= 1 {
        // C# skips PrimDescTable.Read when ChildrenOffset is unset but still writes an
        // empty descriptor child with zero magic on export.
        (Vec::new(), [0, 0, 0, 0])
    } else {
        (Vec::new(), *b"G3NT")
    };

    let binary_data = if header.binary_offset != NONE_U32 && header.size > 0 {
        reader.seek(SeekFrom::Start(section_start + header.binary_offset as u64))?;
        Some(reader.read_bytes(header.size as usize)?)
    } else {
        None
    };

    Ok(PrimitiveInfo {
        descriptors,
        binary_data,
        section_offset: section_start,
        desc_table_magic,
        preserve_binary_data: false,
    })
}

fn parse_primitive_list<R: Read + Seek>(
    reader: &mut R,
    header: &SectionHeader,
    section_start: u64,
) -> io::Result<Vec<RawPrimitiveSection>> {
    let mut primitives = Vec::new();
    if header.children_offset == NONE_U32 || header.children_count == 0 {
        return Ok(primitives);
    }

    reader.seek(SeekFrom::Start(
        section_start + header.children_offset as u64,
    ))?;
    for idx in 0..header.children_count as usize {
        let (child_header, child_start) = SectionHeader::read(reader)?;
        if child_header.magic != "PRIM" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected PRIM"));
        }
        let raw_binary = if child_header.binary_offset != NONE_U32 && child_header.size > 0 {
            reader.seek(SeekFrom::Start(
                child_start + child_header.binary_offset as u64,
            ))?;
            reader.read_bytes(child_header.size as usize)?
        } else {
            Vec::new()
        };
        primitives.push(RawPrimitiveSection { raw_binary });
        if idx + 1 < header.children_count as usize && child_header.next_section_offset != NONE_U32
        {
            reader.seek(SeekFrom::Start(
                child_start + child_header.next_section_offset as u64,
            ))?;
        }
    }
    Ok(primitives)
}

fn parse_emitter_subsection<R: Read + Seek>(
    reader: &mut R,
    ptcl_header: &BinaryHeader,
) -> io::Result<(EmitterSubSection, SectionHeader, u64)> {
    let (header, start) = SectionHeader::read(reader)?;
    let data = if header.binary_offset != NONE_U32 && header.size >= header.binary_offset {
        reader.seek(SeekFrom::Start(start + header.binary_offset as u64))?;
        reader.read_bytes((header.size - header.binary_offset) as usize)?
    } else {
        Vec::new()
    };
    let _ = ptcl_header;
    Ok((
        EmitterSubSection {
            magic: header.magic.clone(),
            data,
        },
        header,
        start,
    ))
}

fn parse_emitter<R: Read + Seek>(
    reader: &mut R,
    ptcl_header: &BinaryHeader,
) -> io::Result<(Emitter, SectionHeader, u64)> {
    let (header, start) = SectionHeader::read(reader)?;
    if header.magic != "EMTR" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "expected EMTR"));
    }

    let binary_end = if header.attr_offset != NONE_U32 {
        header.attr_offset
    } else {
        header.size
    };
    let binary_data = if header.binary_offset != NONE_U32 && binary_end >= header.binary_offset {
        reader.seek(SeekFrom::Start(start + header.binary_offset as u64))?;
        reader.read_bytes((binary_end - header.binary_offset) as usize)?
    } else {
        Vec::new()
    };

    let mut data_reader = Cursor::new(&binary_data);
    let data = EmitterData::read(&mut data_reader, ptcl_header.vfx_version)?;

    let mut subsections = Vec::new();
    if header.attr_offset != NONE_U32 {
        reader.seek(SeekFrom::Start(start + header.attr_offset as u64))?;
        loop {
            let (subsection, subsection_header, subsection_start) =
                parse_emitter_subsection(reader, ptcl_header)?;
            subsections.push(subsection);
            if subsection_header.next_section_offset == NONE_U32 {
                break;
            }
            reader.seek(SeekFrom::Start(
                subsection_start + subsection_header.next_section_offset as u64,
            ))?;
        }
    }

    let mut children = Vec::new();
    if header.children_offset != NONE_U32 && header.children_count > 0 {
        reader.seek(SeekFrom::Start(start + header.children_offset as u64))?;
        for idx in 0..header.children_count as usize {
            let (mut child, child_header, child_start) = parse_emitter(reader, ptcl_header)?;
            child.data.order = idx;
            children.push(child);
            if idx + 1 < header.children_count as usize
                && child_header.next_section_offset != NONE_U32
            {
                reader.seek(SeekFrom::Start(
                    child_start + child_header.next_section_offset as u64,
                ))?;
            }
        }
    }

    Ok((
        Emitter {
            data,
            binary_data: Some(binary_data),
            cached_binary: None,
            subsections,
            children,
        },
        header,
        start,
    ))
}

fn parse_emitter_set<R: Read + Seek>(
    reader: &mut R,
    ptcl_header: &BinaryHeader,
) -> io::Result<(EmitterSet, SectionHeader, u64)> {
    let (header, start) = SectionHeader::read(reader)?;
    if header.magic != "ESET" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "expected ESET"));
    }

    reader.seek(SeekFrom::Start(start + header.binary_offset as u64))?;
    let _padding = reader.read_bytes(16)?;
    let name = read_fixed_string(reader, 64)?;
    let _emitter_count = reader.read_u32_le()?;
    let _zero0 = reader.read_u32_le()?;
    let _zero1 = reader.read_u32_le()?;
    let _zero2 = reader.read_u32_le()?;
    let unknown1 = (ptcl_header.vfx_version >= 0x16)
        .then(|| reader.read_u32_le())
        .transpose()?;
    let unknown2 = (ptcl_header.vfx_version >= 0x16)
        .then(|| reader.read_u32_le())
        .transpose()?;
    let unknown3 = (ptcl_header.vfx_version >= 0x24)
        .then(|| reader.read_u32_le())
        .transpose()?;
    let unknown4 = (ptcl_header.vfx_version >= 0x24)
        .then(|| reader.read_u32_le())
        .transpose()?;
    let unknown5 = (ptcl_header.vfx_version >= 0x24)
        .then(|| reader.read_u32_le())
        .transpose()?;
    let unknown6 = (ptcl_header.vfx_version >= 0x24)
        .then(|| reader.read_u32_le())
        .transpose()?;

    let mut emitters = Vec::new();
    if header.children_offset != NONE_U32 && header.children_count > 0 {
        reader.seek(SeekFrom::Start(start + header.children_offset as u64))?;
        for idx in 0..header.children_count as usize {
            let (mut emitter, emitter_header, emitter_start) = parse_emitter(reader, ptcl_header)?;
            emitter.data.order = idx;
            emitters.push(emitter);
            if idx + 1 < header.children_count as usize
                && emitter_header.next_section_offset != NONE_U32
            {
                reader.seek(SeekFrom::Start(
                    emitter_start + emitter_header.next_section_offset as u64,
                ))?;
            }
        }
    }

    Ok((
        EmitterSet {
            name,
            unknown1,
            unknown2,
            unknown3,
            unknown4,
            unknown5,
            unknown6,
            emitters,
        },
        header,
        start,
    ))
}

fn parse_emitter_list<R: Read + Seek>(
    reader: &mut R,
    header: &SectionHeader,
    section_start: u64,
    ptcl_header: &BinaryHeader,
) -> io::Result<EmitterList> {
    let mut emitter_sets = Vec::new();
    if header.children_offset == NONE_U32 || header.children_count == 0 {
        return Ok(EmitterList { emitter_sets });
    }

    reader.seek(SeekFrom::Start(
        section_start + header.children_offset as u64,
    ))?;
    for idx in 0..header.children_count as usize {
        let (emitter_set, set_header, set_start) = parse_emitter_set(reader, ptcl_header)?;
        emitter_sets.push(emitter_set);
        if idx + 1 < header.children_count as usize && set_header.next_section_offset != NONE_U32 {
            reader.seek(SeekFrom::Start(
                set_start + set_header.next_section_offset as u64,
            ))?;
        }
    }

    Ok(EmitterList { emitter_sets })
}

fn serialize_subsection(
    output: &mut Vec<u8>,
    subsection: &EmitterSubSection,
    is_last: bool,
) -> usize {
    let start = write_section_header(output, &subsection.magic);
    let binary_start = output.len();
    patch_binary_offset(output, start, binary_start);
    output.extend_from_slice(&subsection.data);
    let section_size = (output.len() - start) as u32;
    patch_section_size(output, start, section_size);
    align_vec(output, 4, 0);
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_emitter(
    output: &mut Vec<u8>,
    emitter: &Emitter,
    vfx_version: u16,
    is_last: bool,
) -> usize {
    let start = write_section_header(output, "EMTR");
    patch_children_count(output, start, emitter.children.len() as u16);

    align_vec(output, 256, 0);
    let binary_start = output.len();
    patch_binary_offset(output, start, binary_start);
    if let Some(binary) = &emitter.cached_binary {
        output.extend_from_slice(binary);
    } else {
        match emitter.data.write(vfx_version) {
            Ok(binary) => output.extend_from_slice(&binary),
            Err(_) if emitter.binary_data.is_some() => {
                output.extend_from_slice(emitter.binary_data.as_ref().unwrap());
            }
            Err(err) => {
                eprintln!(
                    "failed to serialize emitter {:?}: {}",
                    emitter.data.display_name(),
                    err
                );
            }
        }
    }

    if !emitter.subsections.is_empty() {
        align_vec(output, 8, 0);
        let attr_start = output.len();
        patch_attr_offset(output, start, attr_start);
        for (idx, subsection) in emitter.subsections.iter().enumerate() {
            serialize_subsection(output, subsection, idx + 1 == emitter.subsections.len());
        }
    }

    let section_size = (output.len() - start) as u32;
    patch_section_size(output, start, section_size);

    if !emitter.children.is_empty() {
        align_vec(output, 8, 0);
        let child_start = output.len();
        patch_child_offset(output, start, child_start);
        for (idx, child) in emitter.children.iter().enumerate() {
            serialize_emitter(
                output,
                child,
                vfx_version,
                idx + 1 == emitter.children.len(),
            );
        }
    }

    align_vec(output, 4, 0);
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_emitter_set(
    output: &mut Vec<u8>,
    emitter_set: &EmitterSet,
    vfx_version: u16,
    is_last: bool,
) -> usize {
    let start = write_section_header(output, "ESET");
    patch_children_count(output, start, emitter_set.emitters.len() as u16);

    let binary_start = output.len();
    patch_binary_offset(output, start, binary_start);
    output.extend_from_slice(&[0u8; 16]);
    let name_start = output.len();
    output.extend_from_slice(emitter_set.name.as_bytes());
    if output.len() < name_start + 64 {
        output.resize(name_start + 64, 0);
    } else {
        output.truncate(name_start + 64);
    }
    let child_count = emitter_set.emitters.len()
        + emitter_set
            .emitters
            .iter()
            .map(|emitter| emitter.children.len())
            .sum::<usize>();
    output.extend_from_slice(&(child_count as u32).to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    if vfx_version >= 0x16 {
        output.extend_from_slice(&emitter_set.unknown1.unwrap_or(0).to_le_bytes());
        output.extend_from_slice(&emitter_set.unknown2.unwrap_or(0).to_le_bytes());
    }
    if vfx_version >= 0x24 {
        output.extend_from_slice(&emitter_set.unknown3.unwrap_or(0).to_le_bytes());
        output.extend_from_slice(&emitter_set.unknown4.unwrap_or(0).to_le_bytes());
        output.extend_from_slice(&emitter_set.unknown5.unwrap_or(0).to_le_bytes());
        output.extend_from_slice(&emitter_set.unknown6.unwrap_or(0).to_le_bytes());
    }

    // NONE is meaningful here. The game-side ESET reader does not safely accept a non-NONE
    // child offset when the set has no children: it can continue at the following ESET and make
    // an intentionally empty emitter set inherit that set's first emitter. Leave the header's
    // initialized NONE offset untouched for an empty set.
    if !emitter_set.emitters.is_empty() {
        let child_start = output.len();
        patch_child_offset(output, start, child_start);
        for (idx, emitter) in emitter_set.emitters.iter().enumerate() {
            serialize_emitter(
                output,
                emitter,
                vfx_version,
                idx + 1 == emitter_set.emitters.len(),
            );
        }
    }

    let section_size = (output.len() - start) as u32;
    patch_section_size(output, start, section_size);
    align_vec(output, 4, 0);
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_emitter_list_section(output: &mut Vec<u8>, ptcl: &PtclFile, is_last: bool) -> usize {
    let start = write_section_header(output, "ESTA");
    patch_children_count(output, start, ptcl.emitter_list.emitter_sets.len() as u16);
    let child_start = output.len();
    patch_child_offset(output, start, child_start);
    patch_binary_offset(output, start, child_start);
    for (idx, emitter_set) in ptcl.emitter_list.emitter_sets.iter().enumerate() {
        serialize_emitter_set(
            output,
            emitter_set,
            ptcl.vfx_version,
            idx + 1 == ptcl.emitter_list.emitter_sets.len(),
        );
    }
    let section_size = (output.len() - start) as u32;
    patch_section_size(output, start, section_size);
    align_vec(output, 16, 0);
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_texture_info_section(
    output: &mut Vec<u8>,
    texture_info: &TextureInfo,
    is_last: bool,
) -> usize {
    let start = write_section_header(output, "GRTF");
    patch_children_count(output, start, 1);
    let child_start = output.len();
    patch_child_offset(output, start, child_start);

    let mut descriptors: Vec<TextureDescriptor> = texture_info.descriptors.clone();
    descriptors.sort_by(|left, right| csharp_descriptor_name_order(&left.name, &right.name));

    let gtnt_start = write_section_header_magic(
        output,
        desc_table_magic_for_export(
            texture_info.descriptors.is_empty(),
            texture_info.desc_table_magic,
        ),
    );
    let gtnt_binary = write_texture_desc_binary(&descriptors);
    patch_section_size(output, gtnt_start, gtnt_binary.len() as u32);
    let gtnt_binary_start = output.len();
    patch_binary_offset(output, gtnt_start, gtnt_binary_start);
    output.extend_from_slice(&gtnt_binary);
    align_vec(output, 16, 0);
    let gtnt_next = output.len();
    patch_next_offset(output, gtnt_start, Some(gtnt_next));

    if let Some(binary_data) = &texture_info.binary_data {
        if !binary_data.is_empty() {
            let texture_names: Vec<String> = descriptors.iter().map(|d| d.name.clone()).collect();
            let canonical = crate::bntx::reorder_and_save(binary_data, &texture_names)
                .unwrap_or_else(|_| binary_data.clone());
            align_vec(output, 4096, 0);
            let binary_start = output.len();
            patch_binary_offset(output, start, binary_start);
            patch_section_size(output, start, canonical.len() as u32);
            output.extend_from_slice(&canonical);
        }
    }

    align_vec(output, 16, 0);
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_primitive_list_section(
    output: &mut Vec<u8>,
    primitives: &[RawPrimitiveSection],
    is_last: bool,
) -> usize {
    let start = write_section_header(output, "PRMA");
    patch_children_count(output, start, primitives.len() as u16);
    if !primitives.is_empty() {
        let child_start = output.len();
        patch_child_offset(output, start, child_start);
        patch_binary_offset(output, start, child_start);
        for (idx, primitive) in primitives.iter().enumerate() {
            let prim_start = write_section_header(output, "PRIM");
            let prim_binary_start = output.len();
            patch_binary_offset(output, prim_start, prim_binary_start);
            output.extend_from_slice(&primitive.raw_binary);
            patch_section_size(output, prim_start, primitive.raw_binary.len() as u32);
            align_vec(output, 4, 0);
            let next_start = (idx + 1 != primitives.len()).then_some(output.len());
            patch_next_offset(output, prim_start, next_start);
        }
    }
    let section_size = (output.len() - start - SECTION_HEADER_SIZE) as u32;
    patch_section_size(output, start, section_size);
    align_vec(output, 4, 0);
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_primitive_info_section(
    output: &mut Vec<u8>,
    primitive_info: &PrimitiveInfo,
    is_last: bool,
) -> usize {
    let start = write_section_header(output, "G3PR");
    patch_children_count(output, start, 1);
    let child_start = output.len();
    patch_child_offset(output, start, child_start);

    let g3nt_start = write_section_header_magic(
        output,
        desc_table_magic_for_export(
            primitive_info.descriptors.is_empty(),
            primitive_info.desc_table_magic,
        ),
    );
    let g3nt_binary = write_primitive_desc_binary(&primitive_info.descriptors);
    patch_section_size(output, g3nt_start, g3nt_binary.len() as u32);
    let g3nt_binary_start = output.len();
    patch_binary_offset(output, g3nt_start, g3nt_binary_start);
    output.extend_from_slice(&g3nt_binary);
    align_vec(output, 16, 0);
    let g3nt_next = output.len();
    patch_next_offset(output, g3nt_start, Some(g3nt_next));

    if let Some(binary_data) = &primitive_info.binary_data {
        if !binary_data.is_empty() {
            let serialized = if primitive_info.preserve_binary_data {
                binary_data.clone()
            } else {
                crate::bfres::ResFile::canonicalize(binary_data)
                    .unwrap_or_else(|_| binary_data.clone())
            };
            align_vec(output, 4096, 0);
            let binary_start = output.len();
            patch_binary_offset(output, start, binary_start);
            patch_section_size(output, start, serialized.len() as u32);
            output.extend_from_slice(&serialized);
        }
    }

    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_shader_info_section(
    output: &mut Vec<u8>,
    shader_info: &ShaderInfo,
    is_last: bool,
) -> usize {
    let start = write_section_header(output, "GRSN");
    let mut grsc_start = None;

    if shader_info.compute_binary.is_some() {
        let child_start = output.len();
        patch_child_offset(output, start, child_start);
        let section_start = write_section_header(output, "GRSC");
        grsc_start = Some(section_start);
        patch_next_offset(output, section_start, None);
    }

    if let Some(binary_data) = &shader_info.binary_data {
        if !binary_data.is_empty() {
            let canonical =
                crate::bnsh::canonicalize(binary_data).unwrap_or_else(|_| binary_data.clone());
            align_vec(output, 4096, 0);
            let binary_start = output.len();
            patch_binary_offset(output, start, binary_start);
            patch_section_size(output, start, canonical.len() as u32);
            output.extend_from_slice(&canonical);
        }
    }

    if let (Some(section_start), Some(compute_binary)) =
        (grsc_start, shader_info.compute_binary.as_ref())
    {
        let canonical =
            crate::bnsh::canonicalize(compute_binary).unwrap_or_else(|_| compute_binary.clone());
        align_vec(output, 4096, 0);
        let binary_start = output.len();
        patch_binary_offset(output, section_start, binary_start);
        patch_section_size(output, section_start, canonical.len() as u32);
        output.extend_from_slice(&canonical);
    }
    let next_start = (!is_last).then_some(output.len());
    patch_next_offset(output, start, next_start);
    start
}

fn serialize_header(output: &mut Vec<u8>, ptcl: &PtclFile) {
    output.extend_from_slice(&ptcl.magic.to_le_bytes());
    output.extend_from_slice(&ptcl.graphics_api_version.to_le_bytes());
    output.extend_from_slice(&ptcl.vfx_version.to_le_bytes());
    output.extend_from_slice(&ptcl.byte_order.to_le_bytes());
    output.push(ptcl.alignment);
    output.push(if ptcl.is_version_64_bit { 64 } else { 32 });
    output.extend_from_slice(&ptcl.name_offset.to_le_bytes());
    output.extend_from_slice(&ptcl.flag.to_le_bytes());
    output.extend_from_slice(&ptcl.block_offset.to_le_bytes());
    output.extend_from_slice(&ptcl.relocation_table_offset.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());

    output.extend_from_slice(ptcl.name.as_bytes());
    output.push(0);
    if output.len() < PTCL_BLOCK_OFFSET {
        output.resize(PTCL_BLOCK_OFFSET, 0);
    }
}

impl PtclFile {
    pub fn serialize(&self) -> Vec<u8> {
        self.serialize_rebuilt()
    }

    fn serialize_rebuilt(&self) -> Vec<u8> {
        let mut output = Vec::new();
        serialize_header(&mut output, self);

        let has_sections = self.section_order.iter().any(|section| match section {
            TopLevelSection::EmitterList => !self.emitter_list.emitter_sets.is_empty(),
            TopLevelSection::TextureInfo => self.texture_info.is_some(),
            TopLevelSection::PrimitiveList => !self.primitive_list_sections.is_empty(),
            TopLevelSection::PrimitiveInfo => self.primitive_info.is_some(),
            TopLevelSection::ShaderInfo => self.shader_info.is_some(),
            TopLevelSection::Raw(bytes) => !bytes.is_empty(),
        });

        let order = if has_sections {
            self.section_order.clone()
        } else {
            let mut fallback = Vec::new();
            if !self.emitter_list.emitter_sets.is_empty() {
                fallback.push(TopLevelSection::EmitterList);
            }
            if self.texture_info.is_some() {
                fallback.push(TopLevelSection::TextureInfo);
            }
            if !self.primitive_list_sections.is_empty() {
                fallback.push(TopLevelSection::PrimitiveList);
            }
            if self.primitive_info.is_some() {
                fallback.push(TopLevelSection::PrimitiveInfo);
            }
            if self.shader_info.is_some() {
                fallback.push(TopLevelSection::ShaderInfo);
            }
            fallback
        };

        let active_indexes: Vec<usize> = order
            .iter()
            .enumerate()
            .filter_map(|(idx, section)| match section {
                TopLevelSection::EmitterList if !self.emitter_list.emitter_sets.is_empty() => {
                    Some(idx)
                }
                TopLevelSection::TextureInfo if self.texture_info.is_some() => Some(idx),
                TopLevelSection::PrimitiveList => Some(idx),
                TopLevelSection::PrimitiveInfo if self.primitive_info.is_some() => Some(idx),
                TopLevelSection::ShaderInfo if self.shader_info.is_some() => Some(idx),
                TopLevelSection::Raw(bytes) if !bytes.is_empty() => Some(idx),
                _ => None,
            })
            .collect();

        for (position, idx) in active_indexes.iter().enumerate() {
            let is_last = position + 1 == active_indexes.len();
            match &order[*idx] {
                TopLevelSection::EmitterList => {
                    serialize_emitter_list_section(&mut output, self, is_last);
                }
                TopLevelSection::TextureInfo => {
                    serialize_texture_info_section(
                        &mut output,
                        self.texture_info.as_ref().unwrap(),
                        is_last,
                    );
                }
                TopLevelSection::PrimitiveList => {
                    serialize_primitive_list_section(
                        &mut output,
                        &self.primitive_list_sections,
                        is_last,
                    );
                }
                TopLevelSection::PrimitiveInfo => {
                    serialize_primitive_info_section(
                        &mut output,
                        self.primitive_info.as_ref().unwrap(),
                        is_last,
                    );
                }
                TopLevelSection::ShaderInfo => {
                    serialize_shader_info_section(
                        &mut output,
                        self.shader_info.as_ref().unwrap(),
                        is_last,
                    );
                }
                TopLevelSection::Raw(bytes) => {
                    let start = output.len();
                    output.extend_from_slice(bytes);
                    if !is_last && bytes.len() >= 16 {
                        let next_start = output.len();
                        patch_next_offset(&mut output, start, Some(next_start));
                    }
                }
            }
        }

        let file_size = output.len() as u32;
        write_u32_at(&mut output, 28, file_size);
        output
    }

    pub fn load(data: &[u8]) -> io::Result<Self> {
        Self::load_with_options(data, true)
    }

    /// Load a PTCL for folder rebuild: header and asset pools only (skip ESTA emitter tree).
    pub fn load_for_rebuild(data: &[u8]) -> io::Result<Self> {
        Self::load_with_options(data, false)
    }

    fn load_with_options(data: &[u8], parse_emitter_tree: bool) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        Self::read(&mut cursor, parse_emitter_tree)
    }

    pub fn save(&self) -> Vec<u8> {
        self.serialize()
    }

    fn read<R: Read + Seek>(reader: &mut R, parse_emitter_tree: bool) -> io::Result<Self> {
        let header = BinaryHeader {
            magic: reader.read_u64_le()?,
            graphics_api_version: reader.read_u16_le()?,
            vfx_version: reader.read_u16_le()?,
            byte_order: reader.read_u16_le()?,
            alignment: reader.read_u8()?,
            target_address_size: reader.read_u8()?,
            name_offset: reader.read_u32_le()?,
            flag: reader.read_u16_le()?,
            block_offset: reader.read_u16_le()?,
            relocation_table_offset: reader.read_u32_le()?,
            file_size: reader.read_u32_le()?,
        };
        let name = read_fixed_string(reader, PTCL_BLOCK_OFFSET - PTCL_HEADER_SIZE)?;

        reader.seek(SeekFrom::Start(0))?;
        let base_bytes = reader.read_bytes(header.file_size as usize)?;

        let is_version_64_bit = header.target_address_size == 64;
        let version_num = Version {
            major: (header.vfx_version >> 12) as u8,
            minor: ((header.vfx_version >> 8) & 0xF) as u8,
            patch: ((header.vfx_version >> 4) & 0xF) as u8,
            build: (header.vfx_version & 0xF) as u8,
        };

        let mut cursor = Cursor::new(&base_bytes);
        cursor.seek(SeekFrom::Start(header.block_offset as u64))?;

        let mut emitter_list = EmitterList {
            emitter_sets: Vec::new(),
        };
        let mut texture_info = None;
        let mut primitive_info = None;
        let mut shader_info = None;
        let mut primitive_list_sections = Vec::new();
        let mut section_order = Vec::new();

        while cursor.stream_position()? < header.file_size as u64 {
            let section_start = cursor.stream_position()?;
            let (section_header, _) = SectionHeader::read(&mut cursor)?;
            cursor.seek(SeekFrom::Start(section_start))?;

            match section_header.magic.as_str() {
                "ESTA" => {
                    section_order.push(TopLevelSection::EmitterList);
                    if parse_emitter_tree {
                        cursor.seek(SeekFrom::Start(section_start))?;
                        let (header_again, start_again) = SectionHeader::read(&mut cursor)?;
                        emitter_list =
                            parse_emitter_list(&mut cursor, &header_again, start_again, &header)?;
                    }
                }
                "GRTF" => {
                    let (header_again, start_again) = SectionHeader::read(&mut cursor)?;
                    texture_info = Some(parse_texture_info(
                        &mut cursor,
                        &header_again,
                        start_again,
                        &header,
                    )?);
                    section_order.push(TopLevelSection::TextureInfo);
                }
                "PRMA" => {
                    let (header_again, start_again) = SectionHeader::read(&mut cursor)?;
                    primitive_list_sections =
                        parse_primitive_list(&mut cursor, &header_again, start_again)?;
                    section_order.push(TopLevelSection::PrimitiveList);
                }
                "G3PR" => {
                    let (header_again, start_again) = SectionHeader::read(&mut cursor)?;
                    primitive_info = Some(parse_primitive_info(
                        &mut cursor,
                        &header_again,
                        start_again,
                    )?);
                    section_order.push(TopLevelSection::PrimitiveInfo);
                }
                "GRSN" => {
                    let (header_again, start_again) = SectionHeader::read(&mut cursor)?;
                    shader_info = Some(parse_shader_info(&mut cursor, &header_again, start_again)?);
                    section_order.push(TopLevelSection::ShaderInfo);
                }
                _ => {
                    let raw_size = if section_header.next_section_offset != NONE_U32 {
                        section_header.next_section_offset as usize
                    } else {
                        (header.file_size as u64 - section_start) as usize
                    };
                    cursor.seek(SeekFrom::Start(section_start))?;
                    section_order.push(TopLevelSection::Raw(cursor.read_bytes(raw_size)?));
                }
            }

            if section_header.next_section_offset == NONE_U32 {
                break;
            }
            cursor.seek(SeekFrom::Start(
                section_start + section_header.next_section_offset as u64,
            ))?;
        }

        let primitives = if primitive_list_sections.is_empty() {
            None
        } else {
            Some(
                primitive_list_sections
                    .iter()
                    .enumerate()
                    .map(|(idx, primitive)| Primitive {
                        name: format!("Primitive_{idx}"),
                        binary_data: Some(primitive.raw_binary.clone()),
                    })
                    .collect(),
            )
        };

        let emitter_order = emitter_list
            .emitter_sets
            .iter()
            .flat_map(|set| set.emitters.iter().map(|emitter| emitter.data.order))
            .collect();

        Ok(PtclFile {
            base_bytes,
            file_size: header.file_size,
            magic: header.magic,
            graphics_api_version: header.graphics_api_version,
            alignment: header.alignment,
            name_offset: header.name_offset,
            flag: header.flag,
            block_offset: header.block_offset,
            relocation_table_offset: header.relocation_table_offset,
            name,
            byte_order: header.byte_order,
            version_num,
            vfx_version: header.vfx_version,
            is_version_64_bit,
            primitives,
            materials: Vec::new(),
            textures: None,
            textures_eft2: None,
            textures_ftex: None,
            shaders: None,
            shader_cbuf: None,
            shader_texture_ref: None,
            texture_info,
            shader_info,
            primitive_info,
            emitter_list,
            emitter_animation: None,
            emitter_order,
            primitive_list_sections,
            section_order,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{serialize_emitter_set, NONE_U32};
    use crate::structs::EmitterSet;

    #[test]
    fn csharp_descriptor_name_order_matches_culture_sort() {
        assert_eq!(
            super::csharp_descriptor_name_order("ef_cmn_fire05", "ef_cmn_fire_indirect01"),
            std::cmp::Ordering::Greater,
        );
        assert_eq!(
            super::csharp_descriptor_name_order("ef_cmn_fire_indirect01", "ef_cmn_fire05"),
            std::cmp::Ordering::Less,
        );
    }

    #[test]
    fn empty_emitter_set_keeps_its_children_offset_none() {
        let set = EmitterSet {
            name: "empty".into(),
            unknown1: None,
            unknown2: None,
            unknown3: None,
            unknown4: None,
            unknown5: None,
            unknown6: None,
            emitters: Vec::new(),
        };
        let mut output = Vec::new();
        let start = serialize_emitter_set(&mut output, &set, 0x2200, true);
        let children_offset = u32::from_le_bytes(output[start + 8..start + 12].try_into().unwrap());
        assert_eq!(children_offset, NONE_U32);
    }
}
