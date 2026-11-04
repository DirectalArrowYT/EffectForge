//! BNTX (NW Switch texture archive) load/save matching Syroot.NintenTools.NSW.Bntx.

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::path::Path;

use num_bigint::BigUint;

const SECTION1: usize = 0;
const SECTION2: usize = 1;

/// Merge single-texture BNTX exports back into one archive (matches C# BntxFile.Save after AddTexture).
pub fn merge_texture_files(files: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    let mut merged: Option<BntxFile> = None;
    for data in files {
        let file = BntxFile::read(data)?;
        match &mut merged {
            None => merged = Some(file),
            Some(existing) => existing.textures.extend(file.textures),
        }
    }
    merged
        .map(|file| file.write())
        .unwrap_or_else(|| Ok(Vec::new()))
}

/// Repopulate a Base.ptcl BNTX container from per-emitter exports (matches C# AddTexture + Save).
pub fn rebuild_from_base_and_exports(base: &[u8], exports: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    if exports.is_empty() {
        return Ok(Vec::new());
    }
    let mut textures = Vec::with_capacity(exports.len());
    for data in exports {
        let export = BntxFile::read(data)?;
        let Some(tex) = export.textures.into_iter().next() else {
            continue;
        };
        textures.push(tex);
    }
    rebuild_from_base_and_textures(base, None, &textures)
}

/// Like [`rebuild_from_base_and_exports`] but reuses textures parsed at load time.
pub(crate) fn rebuild_from_base_and_textures(
    base: &[u8],
    empty_base_shell: Option<&[u8]>,
    exports: &[Texture],
) -> io::Result<Vec<u8>> {
    if exports.is_empty() {
        return Ok(Vec::new());
    }
    let mut file = if !base.is_empty() {
        BntxFile::read(base)?
    } else {
        let shell = empty_base_shell.ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "missing BNTX shell for empty base")
        })?;
        BntxFile::read(shell)?
    };
    file.textures.clear();
    for tex in exports {
        if file
            .textures
            .iter()
            .any(|existing| existing.name == tex.name)
        {
            continue;
        }
        file.textures.push(tex.clone());
    }
    file.write()
}

/// Parse a single-texture export once (name + texture payload for pool rebuild).
pub(crate) fn parse_texture_export(data: &[u8]) -> io::Result<Texture> {
    let file = BntxFile::read(data)?;
    file.textures
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "BNTX export has no textures"))
}

/// Return the first texture name from a single-texture BNTX export.
pub fn first_texture_name(data: &[u8]) -> io::Result<String> {
    Ok(parse_texture_export(data)?.name)
}

pub fn reorder_and_save(data: &[u8], desired_order: &[String]) -> io::Result<Vec<u8>> {
    if data.len() < 0x58 || &data[..4] != b"BNTX" {
        return Ok(data.to_vec());
    }
    let mut file = BntxFile::read(data)?;
    for name in desired_order {
        if !file.textures.iter().any(|t| t.name == *name) {
            return Ok(data.to_vec());
        }
    }
    let mut by_name = HashMap::new();
    for tex in file.textures.drain(..) {
        by_name.insert(tex.name.clone(), tex);
    }
    file.textures = desired_order
        .iter()
        .map(|name| by_name.remove(name).unwrap())
        .collect();
    file.write()
}

pub fn export_single_texture(
    global_bntx: &[u8],
    texture_index: usize,
    texture_name: &str,
    output_path: &Path,
) -> io::Result<()> {
    let out = build_single_texture_bntx(global_bntx, texture_index, texture_name)?;
    std::fs::write(output_path, out)
}

pub fn build_single_texture_bntx_public(
    global_bntx: &[u8],
    texture_index: usize,
    texture_name: &str,
) -> io::Result<Vec<u8>> {
    build_single_texture_bntx(global_bntx, texture_index, texture_name)
}

pub(crate) fn build_single_texture_bntx(
    global_bntx: &[u8],
    texture_index: usize,
    texture_name: &str,
) -> io::Result<Vec<u8>> {
    let texture_table_offset = read_u64_at(global_bntx, 0x28)? as usize;
    let global_brtd_offset = read_u64_at(global_bntx, 0x30)? as usize;
    let brti_offset = read_u64_at(global_bntx, texture_table_offset + texture_index * 8)? as usize;
    let brti_metadata_size = read_u32_at(global_bntx, brti_offset + 8)? as usize;
    let mip_count = read_u16_at(global_bntx, brti_offset + 0x16)? as usize;
    let total_texture_size = read_u32_at(global_bntx, brti_offset + 0x50)? as usize;
    let data_alignment = read_u32_at(global_bntx, brti_offset + 0x54)? as usize;
    let global_image_table_offset = read_u64_at(global_bntx, brti_offset + 0x70)? as usize;
    let dict_ref_bit = compute_single_entry_dict_ref_bit(texture_name);

    let string_section_offset = 0x1A0usize;
    let string_data_offset = string_section_offset + 0x18;
    let string_bytes_end = string_data_offset + 2 + texture_name.len() + 1;
    let dict_offset = align_up(string_bytes_end, 8);
    let brti_offset_new = align_up(dict_offset + 0x28, 8);
    let runtime_block_1_offset = brti_offset_new + 0xA0;
    let runtime_block_2_offset = runtime_block_1_offset + 0x100;
    let image_table_offset_new = runtime_block_2_offset + 0x100;
    let section1_size = image_table_offset_new + mip_count * 8;
    let brtd_offset_new = align_up(section1_size + 0x10, 0x1000) - 0x10;
    let brti_section_size = brtd_offset_new - brti_offset_new;

    let aligned_payload_size = align_up(total_texture_size, 0x1000.max(data_alignment.max(0x10)));

    let mut mip_offsets = Vec::with_capacity(mip_count);
    for i in 0..mip_count {
        mip_offsets.push(read_u64_at(global_bntx, global_image_table_offset + i * 8)?);
    }

    let global_brtd_payload_start = global_brtd_offset + 0x10;
    let first_mip_relative_offset = mip_offsets
        .first()
        .copied()
        .unwrap_or(global_brtd_payload_start as u64)
        .saturating_sub(global_brtd_payload_start as u64)
        as usize;
    let payload_start = global_brtd_payload_start + first_mip_relative_offset;
    let payload_end = payload_start + total_texture_size;
    let payload = global_bntx
        .get(payload_start..payload_end)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "texture payload out of bounds"))?;

    let rlt_offset = align_up(brtd_offset_new + 0x10 + aligned_payload_size, 0x1000);
    let file_size = rlt_offset + 0x90;
    let mut out = vec![0u8; file_size];

    out[0..8].copy_from_slice(b"BNTX\0\0\0\0");
    write_u32_at(&mut out, 0x08, 0x0004_0000);
    write_u16_at(&mut out, 0x0C, 0xFEFF);
    out[0x0E] = 0x0C;
    out[0x0F] = 0x40;
    write_u32_at(&mut out, 0x10, (string_data_offset + 2) as u32);
    write_u16_at(&mut out, 0x14, 0);
    write_u16_at(&mut out, 0x16, string_section_offset as u16);
    write_u32_at(&mut out, 0x18, rlt_offset as u32);
    write_u32_at(&mut out, 0x1C, file_size as u32);

    out[0x20..0x24].copy_from_slice(b"NX  ");
    write_u32_at(&mut out, 0x24, 1);
    write_u64_at(&mut out, 0x28, 0x198);
    write_u64_at(&mut out, 0x30, brtd_offset_new as u64);
    write_u64_at(&mut out, 0x38, dict_offset as u64);
    write_u64_at(&mut out, 0x40, 0x58);
    write_u64_at(&mut out, 0x48, 0);
    write_u32_at(&mut out, 0x50, 0);

    write_u64_at(&mut out, 0x198, brti_offset_new as u64);

    out[string_section_offset..string_section_offset + 4].copy_from_slice(b"_STR");
    write_u32_at(
        &mut out,
        string_section_offset + 0x04,
        (brti_offset_new - string_section_offset) as u32,
    );
    write_u32_at(
        &mut out,
        string_section_offset + 0x08,
        (brti_offset_new - string_section_offset) as u32,
    );
    write_u32_at(&mut out, string_section_offset + 0x10, 1);
    write_u32_at(
        &mut out,
        string_section_offset + 0x14,
        string_data_offset as u32,
    );
    write_u16_at(&mut out, string_data_offset, texture_name.len() as u16);
    out[string_data_offset + 2..string_data_offset + 2 + texture_name.len()]
        .copy_from_slice(texture_name.as_bytes());

    out[dict_offset..dict_offset + 4].copy_from_slice(b"_DIC");
    write_u32_at(&mut out, dict_offset + 0x04, 1);
    write_u32_at(&mut out, dict_offset + 0x08, u32::MAX);
    write_u16_at(&mut out, dict_offset + 0x0C, 1);
    write_u16_at(&mut out, dict_offset + 0x0E, 0);
    write_u64_at(
        &mut out,
        dict_offset + 0x10,
        (string_data_offset - 4) as u64,
    );
    write_u32_at(&mut out, dict_offset + 0x18, dict_ref_bit);
    write_u16_at(&mut out, dict_offset + 0x1C, 0);
    out[dict_offset + 0x1E] = 1;
    out[dict_offset + 0x1F] = 0;
    write_u64_at(&mut out, dict_offset + 0x20, string_data_offset as u64);

    out[brti_offset_new..brti_offset_new + brti_metadata_size]
        .copy_from_slice(&global_bntx[brti_offset..brti_offset + brti_metadata_size]);
    write_u32_at(&mut out, brti_offset_new + 0x04, brti_section_size as u32);
    write_u32_at(&mut out, brti_offset_new + 0x08, brti_section_size as u32);
    write_u64_at(&mut out, brti_offset_new + 0x60, string_data_offset as u64);
    write_u64_at(&mut out, brti_offset_new + 0x68, 0x20);
    write_u64_at(
        &mut out,
        brti_offset_new + 0x70,
        image_table_offset_new as u64,
    );
    write_u64_at(&mut out, brti_offset_new + 0x78, 0);
    write_u64_at(
        &mut out,
        brti_offset_new + 0x80,
        runtime_block_1_offset as u64,
    );
    write_u64_at(
        &mut out,
        brti_offset_new + 0x88,
        runtime_block_2_offset as u64,
    );
    write_u64_at(&mut out, brti_offset_new + 0x90, 0);
    write_u64_at(&mut out, brti_offset_new + 0x98, 0);

    for (i, mip_offset) in mip_offsets.iter().enumerate() {
        let relative = mip_offset
            .saturating_sub((global_brtd_offset + 0x10 + first_mip_relative_offset) as u64);
        write_u64_at(
            &mut out,
            image_table_offset_new + i * 8,
            brtd_offset_new as u64 + 0x10 + relative,
        );
    }

    out[brtd_offset_new..brtd_offset_new + 4].copy_from_slice(b"BRTD");
    write_u32_at(&mut out, brtd_offset_new + 0x04, 0);
    write_u32_at(
        &mut out,
        brtd_offset_new + 0x08,
        (0x10 + aligned_payload_size) as u32,
    );
    out[brtd_offset_new + 0x10..brtd_offset_new + 0x10 + total_texture_size]
        .copy_from_slice(payload);

    out[rlt_offset..rlt_offset + 4].copy_from_slice(b"_RLT");
    write_u32_at(&mut out, rlt_offset + 0x04, rlt_offset as u32);
    write_u32_at(&mut out, rlt_offset + 0x08, 2);

    write_u64_at(&mut out, rlt_offset + 0x10, 0);
    write_u32_at(&mut out, rlt_offset + 0x18, 0);
    write_u32_at(&mut out, rlt_offset + 0x1C, section1_size as u32);
    write_u32_at(&mut out, rlt_offset + 0x20, 0);
    write_u32_at(&mut out, rlt_offset + 0x24, 8);

    write_u64_at(&mut out, rlt_offset + 0x28, 0);
    write_u32_at(&mut out, rlt_offset + 0x30, brtd_offset_new as u32);
    write_u32_at(
        &mut out,
        rlt_offset + 0x34,
        (0x10 + aligned_payload_size) as u32,
    );
    write_u32_at(&mut out, rlt_offset + 0x38, 8);
    write_u32_at(&mut out, rlt_offset + 0x3C, 2);

    write_rlt_entry(&mut out, rlt_offset + 0x40, 0x28, 2, 1, 1);
    write_rlt_entry(&mut out, rlt_offset + 0x48, 0x40, 1, 1, 0);
    write_rlt_entry(&mut out, rlt_offset + 0x50, 0x198, 1, 1, 0);
    write_rlt_entry(
        &mut out,
        rlt_offset + 0x58,
        (dict_offset + 0x10) as u32,
        2,
        1,
        1,
    );
    write_rlt_entry(
        &mut out,
        rlt_offset + 0x60,
        (brti_offset_new + 0x60) as u32,
        1,
        3,
        0,
    );
    write_rlt_entry(
        &mut out,
        rlt_offset + 0x68,
        (brti_offset_new + 0x78) as u32,
        1,
        1,
        0,
    );
    write_rlt_entry(
        &mut out,
        rlt_offset + 0x70,
        (brti_offset_new + 0x80) as u32,
        1,
        2,
        0,
    );
    write_rlt_entry(
        &mut out,
        rlt_offset + 0x78,
        (brti_offset_new + 0x98) as u32,
        1,
        1,
        0,
    );
    write_rlt_entry(&mut out, rlt_offset + 0x80, 0x30, 1, 1, 0);
    write_rlt_entry(
        &mut out,
        rlt_offset + 0x88,
        image_table_offset_new as u32,
        1,
        mip_count as u8,
        0,
    );

    Ok(out)
}

#[derive(Debug, Clone)]
pub(crate) struct Texture {
    pub(crate) name: String,
    brti_header: Vec<u8>,
    mip_count: u16,
    mip_offsets: Vec<u64>,
    payload: Vec<u8>,
}

#[derive(Debug, Clone)]
struct BntxFile {
    version: u32,
    byte_order: u16,
    alignment_log2: u8,
    target_address_size: u8,
    name: String,
    flag: u16,
    target: [u8; 4],
    textures: Vec<Texture>,
    string_table_order: Vec<String>,
}

impl BntxFile {
    fn read(data: &[u8]) -> io::Result<Self> {
        if data.len() < 0x58 || &data[..4] != b"BNTX" {
            return Err(io::Error::new(ErrorKind::InvalidData, "invalid BNTX"));
        }

        let version = read_u32_at(data, 0x08)?;
        let byte_order = read_u16_at(data, 0x0C)?;
        let alignment_log2 = data[0x0E];
        let target_address_size = data[0x0F];
        let name_offset = read_u32_at(data, 0x10)? as usize;
        let flag = read_u16_at(data, 0x14)?;
        let mut target = [0u8; 4];
        target.copy_from_slice(&data[0x20..0x24]);
        let texture_count = read_i32_at(data, 0x24)? as usize;
        let texture_array_offset = read_u64_at(data, 0x28)? as usize;
        let brtd_offset = read_u64_at(data, 0x30)? as usize;
        let dict_offset = read_u64_at(data, 0x38)? as usize;

        let name = if name_offset >= 2 && name_offset + 2 <= data.len() {
            let name_pos = name_offset;
            let name_len = read_u16_at(data, name_pos - 2)? as usize;
            read_utf8(data, name_pos, name_len)?
        } else {
            String::new()
        };

        let mut textures = Vec::with_capacity(texture_count);
        for i in 0..texture_count {
            let brti_offset = read_u64_at(data, texture_array_offset + i * 8)? as usize;
            textures.push(load_texture(data, brti_offset, brtd_offset)?);
        }

        let string_table_order = load_string_table_order(data, dict_offset)?;

        Ok(Self {
            version,
            byte_order,
            alignment_log2,
            target_address_size,
            name,
            flag,
            target,
            textures,
            string_table_order,
        })
    }

    fn write(&self) -> io::Result<Vec<u8>> {
        let data_alignment = 1usize << self.alignment_log2;
        let mut writer = BinWriter::default();
        let mut rlt = RelocationTable::new(2);
        let mut strings = StringTable::default();

        writer.write_signature("BNTX");
        writer.write_u32(0);
        writer.write_u32(self.version);
        writer.write_u16(self.byte_order);
        writer.write_u8(self.alignment_log2);
        writer.write_u8(self.target_address_size);
        let file_name_pos = writer.position();
        writer.write_u32(0);
        writer.write_u16(self.flag);
        writer.save_header_block(true);
        rlt.save_header_offset(&mut writer);
        let file_size_pos = writer.position();
        writer.write_u32(0);

        writer.write_bytes(&self.target);
        writer.write_i32(self.textures.len() as i32);
        rlt.save_entry(writer.position(), 1, 2, 1, SECTION1);
        let texture_array_pos = writer.save_offset();
        rlt.save_entry(writer.position(), 1, 1, 0, SECTION2);
        let brtd_ptr_pos = writer.save_offset();
        let dict_ptr_pos = writer.save_offset();
        rlt.save_entry(writer.position(), 1, 1, 0, SECTION1);
        writer.write_u64(0x58);
        writer.write_u64(0);
        writer.write_u64(0);
        writer.write_zeroes(0x140);
        rlt.save_entry(
            writer.position(),
            self.textures.len() as u32,
            1,
            0,
            SECTION1,
        );

        writer.align_bytes(8);
        writer.write_offset(texture_array_pos);
        let mut texture_ptr_positions = Vec::with_capacity(self.textures.len());
        for _ in &self.textures {
            texture_ptr_positions.push(writer.save_offset());
        }

        setup_string_pool(
            &mut writer,
            &mut strings,
            &self.name,
            &self.textures,
            &self.string_table_order,
        );

        writer.align_bytes(8);
        writer.write_offset(dict_ptr_pos);
        write_dict(
            &mut writer,
            &mut strings,
            &mut rlt,
            &self
                .textures
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
        );

        writer.align_bytes(8);
        let mut mip_placeholder_positions = Vec::new();
        for (idx, tex) in self.textures.iter().enumerate() {
            writer.write_offset(texture_ptr_positions[idx]);
            let (_, placeholders) = save_texture(&mut writer, &mut strings, &mut rlt, tex)?;
            mip_placeholder_positions.extend(placeholders);
        }

        let section1_size = writer.position() as u32;
        writer.write_zeroes(16);
        let alignment = round_up(writer.position(), data_alignment) - writer.position();
        if alignment != 0 {
            writer.write_zeroes(alignment - 16);
        }
        let data_block_pos = writer.position();
        write_texture_block(
            &mut writer,
            &self.textures,
            &mip_placeholder_positions,
            data_block_pos,
        );
        writer.write_offset_to(brtd_ptr_pos, data_block_pos);

        rlt.set_section(SECTION1, 0, section1_size);
        writer.align_bytes(data_alignment);
        rlt.set_section(
            SECTION2,
            data_block_pos as u32,
            (writer.position() - data_block_pos) as u32,
        );
        rlt.write(&mut writer);
        strings.write_in_pool(&mut writer, file_name_pos);
        writer.write_header_blocks();
        writer.patch_u32(file_size_pos, writer.position() as u32);
        Ok(writer.into_bytes())
    }
}

fn load_texture(data: &[u8], brti_offset: usize, brtd_offset: usize) -> io::Result<Texture> {
    if brti_offset + 0xA0 > data.len() || &data[brti_offset..brti_offset + 4] != b"BRTI" {
        return Err(io::Error::new(ErrorKind::InvalidData, "invalid BRTI"));
    }

    let brti_section_size = read_u32_at(data, brti_offset + 8)? as usize;
    let mut brti_header = data
        .get(brti_offset..brti_offset + 0xA0.min(brti_section_size))
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "brti header"))?
        .to_vec();
    while brti_header.len() < 0xA0 {
        brti_header.push(0);
    }

    let mip_count = read_u16_at(data, brti_offset + 0x16)?;
    let image_size = read_u32_at(data, brti_offset + 0x50)?;
    let name_offset = read_u64_at(data, brti_offset + 0x60)? as usize;
    let name_len = read_u16_at(data, name_offset)? as usize;
    let name = read_utf8(data, name_offset + 2, name_len)?;
    let image_table_abs = read_u64_at(data, brti_offset + 0x70)? as usize;

    let mut mip_offsets = Vec::with_capacity(mip_count as usize);
    for i in 0..mip_count as usize {
        mip_offsets.push(read_u64_at(data, image_table_abs + i * 8)?);
    }

    let brtd_payload_start = brtd_offset + 0x10;
    let first_mip = mip_offsets
        .first()
        .copied()
        .unwrap_or(brtd_payload_start as u64);
    let payload_start =
        brtd_payload_start + first_mip.saturating_sub(brtd_payload_start as u64) as usize;
    let payload_end = payload_start + image_size as usize;
    let payload = data
        .get(payload_start..payload_end)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "texture payload"))?
        .to_vec();

    let start_mip = mip_offsets[0];
    let mip_offsets: Vec<u64> = mip_offsets
        .into_iter()
        .map(|o| o.saturating_sub(start_mip))
        .collect();

    Ok(Texture {
        name,
        brti_header,
        mip_count,
        mip_offsets,
        payload,
    })
}

fn load_string_table_order(data: &[u8], dict_offset: usize) -> io::Result<Vec<String>> {
    let str_offset = data
        .windows(4)
        .position(|window| window == b"_STR")
        .unwrap_or(dict_offset);
    if str_offset + 0x18 > data.len() {
        return Ok(Vec::new());
    }
    let pool_start = str_offset + 0x10;
    let count = read_u32_at(data, pool_start)? as usize;
    let mut order = Vec::with_capacity(count + 1);
    let mut pos = pool_start + 4;
    for _ in 0..=count {
        if pos + 2 > data.len() {
            break;
        }
        let len = read_u16_at(data, pos)? as usize;
        pos += 2;
        let name = read_utf8(data, pos, len)?;
        pos += len + 1;
        while pos % 2 != 0 {
            pos += 1;
        }
        order.push(name);
    }
    Ok(order)
}

fn setup_string_pool(
    writer: &mut BinWriter,
    strings: &mut StringTable,
    file_name: &str,
    textures: &[Texture],
    string_table_order: &[String],
) {
    strings.collect_keys(file_name, textures, string_table_order);
    writer.align_bytes(4);
    writer.write_signature("_STR");
    writer.save_header_block(false);
    strings.pool_start = writer.position();
    writer.write_i32(0);
    strings.pool_len = strings.build_pool_bytes().len();
    writer.write_zeroes(strings.pool_len.saturating_sub(4));
}

fn save_texture(
    writer: &mut BinWriter,
    strings: &mut StringTable,
    rlt: &mut RelocationTable,
    tex: &Texture,
) -> io::Result<(usize, Vec<usize>)> {
    let brti_start = writer.position();
    writer.write_bytes(&tex.brti_header[..0xA0]);
    writer.saved_header_block_positions.push(brti_start + 4);

    let runtime_block_1 = brti_start + 0xA0;
    let runtime_block_2 = runtime_block_1 + 0x100;
    let image_table = runtime_block_2 + 0x100;

    writer.write_zeroes(0x200);
    writer.align_bytes(8);

    writer.patch_u64(brti_start + 0x68, 0x20);
    writer.patch_u64(brti_start + 0x70, image_table as u64);
    writer.patch_u64(brti_start + 0x78, 0);
    writer.patch_u64(brti_start + 0x80, runtime_block_1 as u64);
    writer.patch_u64(brti_start + 0x88, runtime_block_2 as u64);
    writer.patch_u64(brti_start + 0x90, 0);
    writer.patch_u64(brti_start + 0x98, 0);

    strings.add_entry(brti_start + 0x60, &tex.name);
    rlt.save_entry(brti_start + 0x60, 3, 1, 0, SECTION1);
    rlt.save_entry(brti_start + 0x78, 1, 1, 0, SECTION1);
    rlt.save_entry(brti_start + 0x80, 2, 1, 0, SECTION1);
    rlt.save_entry(brti_start + 0x98, 1, 1, 0, SECTION1);
    rlt.save_entry(image_table, tex.mip_count as u32, 1, 0, SECTION2);

    let mut placeholders = Vec::with_capacity(tex.mip_count as usize);
    for i in 0..tex.mip_count as usize {
        placeholders.push(image_table + i * 8);
        writer.write_u64(0);
    }
    Ok((brti_start, placeholders))
}

fn write_texture_block(
    writer: &mut BinWriter,
    textures: &[Texture],
    mip_placeholders: &[usize],
    data_block_pos: usize,
) {
    writer.write_signature("BRTD");
    writer.save_header_block(false);
    let mut mip_idx = 0;
    for tex in textures {
        let block_offset = writer.position();
        for offset in &tex.mip_offsets {
            writer.patch_u64(mip_placeholders[mip_idx], block_offset as u64 + *offset);
            mip_idx += 1;
        }
        writer.write_bytes(&tex.payload);
    }
    let _ = data_block_pos;
}

fn write_dict(
    writer: &mut BinWriter,
    strings: &mut StringTable,
    rlt: &mut RelocationTable,
    keys: &[&str],
) {
    let nodes = generate_dict_nodes(keys);
    writer.write_signature("_DIC");
    writer.write_i32(nodes.len() as i32 - 1);
    for (index, node) in nodes.iter().enumerate() {
        writer.write_u32(node.reference);
        writer.write_u16(node.left_index);
        writer.write_u16(node.right_index);
        if index == 0 {
            rlt.save_entry(writer.position(), 1, nodes.len() as u32, 1, SECTION1);
            save_string_ref(writer, strings, "");
        } else {
            save_string_ref(writer, strings, &node.key);
        }
    }
}

fn save_string_ref(writer: &mut BinWriter, strings: &mut StringTable, value: &str) {
    let pos = writer.position();
    strings.add_entry(pos, value);
    writer.write_u32(u32::MAX);
    writer.write_u32(0);
}

#[derive(Debug, Default)]
struct BinWriter {
    output: Vec<u8>,
    saved_header_block_positions: Vec<usize>,
    binary_header_block_positions: Vec<usize>,
    end_of_block_offset: usize,
}

impl BinWriter {
    fn position(&self) -> usize {
        self.output.len()
    }

    fn into_bytes(self) -> Vec<u8> {
        self.output
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

    fn patch_u16(&mut self, offset: usize, value: u16) {
        self.output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn patch_u32(&mut self, offset: usize, value: u32) {
        if offset + 4 <= self.output.len() {
            self.output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    fn patch_u64(&mut self, offset: usize, value: u64) {
        if offset + 8 <= self.output.len() {
            self.output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
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
    }

    fn save_header_block(&mut self, binary_only: bool) {
        if binary_only {
            self.binary_header_block_positions.push(self.position());
            self.write_u16(0);
        } else {
            self.saved_header_block_positions.push(self.position());
            self.write_u32(0);
            self.write_u64(0);
        }
    }

    fn write_header_blocks(&mut self) {
        let str_block_start = self.saved_header_block_positions.first().copied();
        let binary_blocks = self.binary_header_block_positions.clone();
        for position in binary_blocks {
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

#[derive(Debug, Default)]
struct RelocationTable {
    sections: Vec<RelocationSection>,
    relocation_table_offset_pos: usize,
}

impl RelocationTable {
    fn new(count: usize) -> Self {
        Self {
            sections: vec![RelocationSection::default(); count],
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

    fn set_section(&mut self, section_idx: usize, position: u32, size: u32) {
        if section_idx < self.sections.len() {
            self.sections[section_idx].position = position;
            self.sections[section_idx].size = size;
        }
    }

    fn write(&mut self, writer: &mut BinWriter) {
        for section in &mut self.sections {
            section.entries.sort_by_key(|entry| entry.position);
        }
        writer.align_bytes(4096);
        let position = writer.position();
        writer.patch_u32(self.relocation_table_offset_pos, position as u32);
        writer.write_signature("_RLT");
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

#[derive(Debug, Default, Clone)]
struct StringEntry {
    positions: Vec<usize>,
}

#[derive(Debug, Default)]
struct StringTable {
    strings: HashMap<String, StringEntry>,
    sorted_keys: Vec<String>,
    file_name: String,
    pool_start: usize,
    pool_len: usize,
}

impl StringTable {
    fn collect_keys(
        &mut self,
        file_name: &str,
        textures: &[Texture],
        string_table_order: &[String],
    ) {
        self.file_name = file_name.to_string();
        let mut sorted: Vec<String> = string_table_order
            .iter()
            .filter(|s| s.is_empty() || **s == file_name || textures.iter().any(|t| &t.name == *s))
            .cloned()
            .collect();
        for tex in textures {
            if !sorted.iter().any(|s| s == &tex.name) {
                sorted.push(tex.name.clone());
            }
        }
        if !sorted.iter().any(|s| s == file_name) {
            sorted.push(file_name.to_string());
        }
        if !sorted.iter().any(String::is_empty) {
            sorted.insert(0, String::new());
        }
        self.sorted_keys = sorted;
    }

    fn pool_keys(&self) -> Vec<String> {
        let mut keys = self.sorted_keys.clone();
        for key in self.strings.keys() {
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.clone());
            }
        }
        keys
    }

    fn build_pool_bytes(&self) -> Vec<u8> {
        let keys = self.pool_keys();
        let mut pool = Vec::new();
        pool.extend_from_slice(&((keys.len().saturating_sub(1)) as i32).to_le_bytes());
        for key in &keys {
            pool.extend_from_slice(&(key.len() as u16).to_le_bytes());
            pool.extend_from_slice(key.as_bytes());
            pool.push(0);
            while pool.len() % 2 != 0 {
                pool.push(0);
            }
        }
        pool
    }

    fn add_entry(&mut self, position: usize, value: &str) {
        self.strings
            .entry(value.to_string())
            .or_default()
            .positions
            .push(position);
    }

    fn write_in_pool(&self, writer: &mut BinWriter, file_name_header_pos: usize) {
        let keys = self.pool_keys();
        let mut pool = Vec::with_capacity(self.pool_len);
        pool.extend_from_slice(&((keys.len().saturating_sub(1)) as i32).to_le_bytes());
        let mut string_positions = HashMap::new();
        let mut file_name_string_pos = 0u32;
        let mut cursor = 4usize;
        for key in &keys {
            let abs_pos = self.pool_start + cursor;
            string_positions.insert(key.as_str(), abs_pos as u32);
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
        let end = self.pool_start + pool.len();
        if end > writer.output.len() {
            writer.output.resize(end, 0);
        }
        writer.output[self.pool_start..end].copy_from_slice(&pool);

        for (key, entry) in &self.strings {
            let Some(&target) = string_positions.get(key.as_str()) else {
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

fn generate_dict_nodes(keys: &[&str]) -> Vec<DictNode> {
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
            reference: u32::MAX,
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

pub(crate) fn dict_name_bit(texture_name: &str, bit_index: i32) -> u32 {
    if bit_index < 0 {
        return 0;
    }
    let bytes = texture_name.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    let bit_index = bit_index as usize;
    let byte_from_end = bit_index / 8;
    let bit_in_byte = bit_index % 8;
    let Some(&byte) = bytes.get(bytes.len() - 1 - byte_from_end) else {
        return 0;
    };
    ((byte >> bit_in_byte) & 1) as u32
}

pub(crate) fn compute_single_entry_dict_ref_bit(texture_name: &str) -> u32 {
    let byte_len = texture_name.len();
    let max_bit = (byte_len * 8).max(8) as i32;
    for bit_index in 0..max_bit {
        if dict_name_bit(texture_name, bit_index) == 1 {
            return bit_index as u32;
        }
    }
    1
}

fn align_up(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }
    (value + align - 1) & !(align - 1)
}

fn round_up(value: usize, align: usize) -> usize {
    align_up(value, align)
}

fn read_u16_at(data: &[u8], offset: usize) -> io::Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "u16"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_at(data: &[u8], offset: usize) -> io::Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "u32"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i32_at(data: &[u8], offset: usize) -> io::Result<i32> {
    Ok(read_u32_at(data, offset)? as i32)
}

fn read_u64_at(data: &[u8], offset: usize) -> io::Result<u64> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "u64"))?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_utf8(data: &[u8], offset: usize, len: usize) -> io::Result<String> {
    let end = offset + len;
    let slice = data
        .get(offset..end)
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "string"))?;
    Ok(String::from_utf8_lossy(slice).into_owned())
}

fn write_u16_at(data: &mut [u8], offset: usize, value: u16) {
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32_at(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64_at(data: &mut [u8], offset: usize, value: u64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn write_rlt_entry(
    data: &mut [u8],
    offset: usize,
    entry_offset: u32,
    array_count: u16,
    offset_count: u8,
    padding_size: u8,
) {
    write_u32_at(data, offset, entry_offset);
    write_u16_at(data, offset + 4, array_count);
    data[offset + 6] = offset_count;
    data[offset + 7] = padding_size;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_single_entry_dict_ref_bit() {
        assert_eq!(
            compute_single_entry_dict_ref_bit("ef_mario_localcoin00_nor"),
            1
        );
        assert_eq!(
            compute_single_entry_dict_ref_bit("ef_cmn_bomb_indirect00"),
            4
        );
        assert_eq!(compute_single_entry_dict_ref_bit("ef_cmn_cloud01"), 0);
    }
}
