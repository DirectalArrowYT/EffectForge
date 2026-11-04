use crate::reader::ReaderExt;
use serde::Serialize;
use std::io::Read;

/// VFXB Binary Header (16 bytes)
#[derive(Debug, Clone)]
pub struct BinaryHeader {
    pub magic: u64,
    pub graphics_api_version: u16,
    pub vfx_version: u16,
    pub byte_order: u16,
    pub alignment: u8,
    pub target_address_size: u8,
    pub name_offset: u32,
    pub flag: u16,
    pub block_offset: u16,
    pub relocation_table_offset: u32,
    pub file_size: u32,
}

/// Section Header (16 bytes)
#[derive(Debug, Clone)]
pub struct SectionHeader {
    pub magic: u32,
    pub size: u32,
    pub children_offset: u32,
    pub next_section_offset: u32,
    pub attr_offset: u32,
    pub binary_offset: u32,
    pub padding: u32,
    pub children_count: u16,
    pub unknown: u16,
}

/// EFFN File Header (16 bytes)
#[derive(Debug, Clone)]
pub struct EffnHeader {
    pub magic: u32,
    pub version: u32,
    pub num_effects: u16,
    pub num_external_models: u16,
    pub multi_part_effects: u16,
    pub header_chunk_align: u16,
}

/// Effect Header (16 bytes)
#[derive(Debug, Clone)]
pub struct EffectHeader {
    pub kind: u16,
    pub unknown: u16,
    pub emitter_set_id: u32,
    pub external_model_idx: u32,
    pub variant_start_idx: u16,
    pub variant_count: u16,
}

/// Effect Variant (4 bytes)
#[derive(Debug, Clone)]
pub struct EffectVariant {
    pub start_frame: u16,
    pub emitter_set_id: u16,
}

impl BinaryHeader {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(BinaryHeader {
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
        })
    }

    pub fn version(&self) -> u16 {
        self.vfx_version
    }
}

impl SectionHeader {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(SectionHeader {
            magic: reader.read_u32_le()?,
            size: reader.read_u32_le()?,
            children_offset: reader.read_u32_le()?,
            next_section_offset: reader.read_u32_le()?,
            attr_offset: reader.read_u32_le()?,
            binary_offset: reader.read_u32_le()?,
            padding: reader.read_u32_le()?,
            children_count: reader.read_u16_le()?,
            unknown: reader.read_u16_le()?,
        })
    }
}

impl EffnHeader {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EffnHeader {
            magic: reader.read_u32_le()?,
            version: reader.read_u32_le()?,
            num_effects: reader.read_u16_le()?,
            num_external_models: reader.read_u16_le()?,
            multi_part_effects: reader.read_u16_le()?,
            header_chunk_align: reader.read_u16_le()?,
        })
    }
}

impl EffectHeader {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EffectHeader {
            kind: reader.read_u16_le()?,
            unknown: reader.read_u16_le()?,
            emitter_set_id: reader.read_u32_le()?,
            external_model_idx: reader.read_u32_le()?,
            variant_start_idx: reader.read_u16_le()?,
            variant_count: reader.read_u16_le()?,
        })
    }
}

impl EffectVariant {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EffectVariant {
            start_frame: reader.read_u16_le()?,
            emitter_set_id: reader.read_u16_le()?,
        })
    }
}

// Placeholder structs for texture, shader, and primitive data
#[derive(Debug, Clone, Serialize)]
pub struct Primitive {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_data: Option<Vec<u8>>,
}

impl Primitive {
    pub fn read_list<R: Read>(
        _reader: &mut R,
        _data_size: u64,
        _version_num: &crate::enums::Version,
    ) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MaterialInfo {
    pub name: String,
}

impl MaterialInfo {
    pub fn read<R: Read>(_reader: &mut R) -> std::io::Result<Self> {
        Ok(MaterialInfo {
            name: String::new(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Eft1Texture {
    pub name: String,
}

impl Eft1Texture {
    pub fn read_list<R: Read>(_reader: &mut R, _data_size: u64) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Eft2Texture {
    pub name: String,
}

impl Eft2Texture {
    pub fn read_list<R: Read>(_reader: &mut R, _data_size: u64) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FtexTexture {
    pub name: String,
}

impl FtexTexture {
    pub fn read_list<R: Read>(_reader: &mut R, _data_size: u64) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShaderRef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_data: Option<Vec<u8>>,
}

impl ShaderRef {
    pub fn read_list<R: Read>(
        _reader: &mut R,
        _data_size: u64,
        _vfx_version: u16,
    ) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShaderCbuf {
    pub name: String,
}

impl ShaderCbuf {
    pub fn read_list<R: Read>(_reader: &mut R, _data_size: u64) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShaderTextureRef {
    pub name: String,
}

impl ShaderTextureRef {
    pub fn read_list<R: Read>(_reader: &mut R, _data_size: u64) -> std::io::Result<Vec<Self>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EmitterList {
    pub emitter_sets: Vec<EmitterSet>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmitterSet {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown1: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown2: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown3: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown4: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown5: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown6: Option<u32>,
    pub emitters: Vec<Emitter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmitterSubSection {
    pub magic: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Emitter {
    pub data: crate::emitter::EmitterData,
    #[serde(skip)]
    pub binary_data: Option<Vec<u8>>,
    /// Pre-serialized EMTR binary when `EmitterData::write` succeeds at load time.
    #[serde(skip)]
    pub cached_binary: Option<Vec<u8>>,
    pub subsections: Vec<EmitterSubSection>,
    pub children: Vec<Emitter>,
}
