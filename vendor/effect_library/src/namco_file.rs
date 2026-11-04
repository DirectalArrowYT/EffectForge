use crate::ptcl_file::PtclFile;
use crate::reader::ReaderExt;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Clone, Serialize)]
pub struct EffnHeader {
    pub magic: String,
    pub version: u32,
    pub num_effects: u16,
    pub num_external_models: u16,
    pub multi_part_effects: u16,
    pub header_chunk_align: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectHeader {
    pub kind: u16,
    pub unknown: u16,
    pub emitter_set_id: u32,
    pub external_model_idx: u32,
    pub variant_start_idx: u16,
    pub variant_count: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectVariant {
    pub start_frame: u16,
    pub emitter_set_id: u16,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct JsonExportEntry {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Kind")]
    pub kind: u16,
    #[serde(rename = "Unknown")]
    pub unknown: u16,
    #[serde(rename = "EmitterSet_ID")]
    pub emitter_set_id: u32,
    #[serde(rename = "ExternalModelFlag")]
    pub external_model_flag: u8,
    #[serde(rename = "ExternalModelID")]
    pub external_model_id: u32,
    #[serde(rename = "ExternalModelString")]
    pub external_model_string: String,
    #[serde(rename = "Variants")]
    pub variants: Vec<JsonExportVariant>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct JsonExportVariant {
    #[serde(rename = "BoneName")]
    pub bone_name: String,
    #[serde(rename = "StartFrame")]
    pub start_frame: u16,
    #[serde(rename = "EmitterSetID")]
    pub emitter_set_id: u16,
}

#[derive(Debug)]
pub struct NamcoEffectFile {
    pub header: EffnHeader,
    pub entries: Vec<EffectHeader>,
    pub effect_variants: Vec<EffectVariant>,
    pub effect_models: Vec<u8>,
    pub entry_names: Vec<String>,
    pub external_model_names: Vec<String>,
    pub external_bone_names: Vec<String>,
    pub ptcl_file: Option<PtclFile>,
}

impl NamcoEffectFile {
    pub fn load(data: &[u8]) -> std::io::Result<Self> {
        use std::io::Cursor;
        let mut reader = Cursor::new(data);
        Self::read(&mut reader)
    }

    pub fn read<R: Read + Seek>(reader: &mut R) -> std::io::Result<Self> {
        // Read header
        let magic = reader.read_magic(4)?;
        if magic != "EFFN" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid magic: expected EFFN, got {}", magic),
            ));
        }

        let version = reader.read_u32_le()?;
        let num_effects = reader.read_u16_le()?;
        let num_external_models = reader.read_u16_le()?;
        let multi_part_effects = reader.read_u16_le()?;
        let header_chunk_align = reader.read_u16_le()?;

        let header = EffnHeader {
            magic,
            version,
            num_effects,
            num_external_models,
            multi_part_effects,
            header_chunk_align,
        };

        // Read effect headers
        let mut entries = Vec::new();
        for _ in 0..num_effects {
            let kind = reader.read_u16_le()?;
            let unknown = reader.read_u16_le()?;
            let emitter_set_id = reader.read_u32_le()?;
            let external_model_idx = reader.read_u32_le()?;
            let variant_start_idx = reader.read_u16_le()?;
            let variant_count = reader.read_u16_le()?;

            entries.push(EffectHeader {
                kind,
                unknown,
                emitter_set_id,
                external_model_idx,
                variant_start_idx,
                variant_count,
            });
        }

        // Read effect variants
        let mut effect_variants = Vec::new();
        for _ in 0..multi_part_effects {
            let start_frame = reader.read_u16_le()?;
            let emitter_set_id = reader.read_u16_le()?;
            effect_variants.push(EffectVariant {
                start_frame,
                emitter_set_id,
            });
        }

        // Read effect models (flags)
        let mut effect_models = Vec::new();
        for _ in 0..num_external_models {
            effect_models.push(reader.read_u8()?);
        }

        // Read entry names
        let mut entry_names = Vec::new();
        for _ in 0..num_effects {
            let s = reader.read_string_z(256)?;
            entry_names.push(s);
        }

        // Read external model names
        let mut external_model_names = Vec::new();
        for _ in 0..num_external_models {
            let s = reader.read_string_z(256)?;
            external_model_names.push(s);
        }

        // Read external bone names
        let mut external_bone_names = Vec::new();
        for _ in 0..multi_part_effects {
            let s = reader.read_string_z(256)?;
            external_bone_names.push(s);
        }

        // Seek to the embedded PTCL chunk using the stored align value when present.
        let pos = reader.stream_position()?;
        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(pos))?;

        let ptcl_offset = if header.header_chunk_align > 0 {
            header.header_chunk_align as u64 * 0x1000
        } else {
            let align = Self::get_required_chunk_align(
                &entries,
                &effect_variants,
                &entry_names,
                &external_model_names,
                &external_bone_names,
            ) as u64;
            pos.div_ceil(align) * align
        };

        if ptcl_offset >= file_len {
            return Ok(NamcoEffectFile {
                header,
                entries,
                effect_variants,
                effect_models,
                entry_names,
                external_model_names,
                external_bone_names,
                ptcl_file: None,
            });
        }

        reader.seek(SeekFrom::Start(ptcl_offset))?;

        // Check if we're at EOF
        let mut test_buf = [0u8; 1];
        let test_read = reader.read(&mut test_buf)?;
        if test_read == 0 {
            return Ok(NamcoEffectFile {
                header,
                entries,
                effect_variants,
                effect_models,
                entry_names,
                external_model_names,
                external_bone_names,
                ptcl_file: None,
            });
        }
        // Seek back
        reader.seek(SeekFrom::Current(-1))?;

        // Read PTCL file
        let mut ptcl_data = Vec::new();
        reader.read_to_end(&mut ptcl_data)?;
        let ptcl_file = PtclFile::load(&ptcl_data).ok();

        Ok(NamcoEffectFile {
            header,
            entries,
            effect_variants,
            effect_models,
            entry_names,
            external_model_names,
            external_bone_names,
            ptcl_file,
        })
    }

    fn get_required_chunk_align(
        entries: &[EffectHeader],
        effect_variants: &[EffectVariant],
        entry_names: &[String],
        external_model_names: &[String],
        external_bone_names: &[String],
    ) -> usize {
        let mut size = 0x10; // header size
        size += entries.len() * 0x10;
        size += effect_variants.len() * 0x4;
        size += external_model_names.len(); // model flags
        size += entry_names.iter().map(|n| n.len() + 1).sum::<usize>();
        size += external_model_names
            .iter()
            .map(|n| n.len() + 1)
            .sum::<usize>();
        size += external_bone_names
            .iter()
            .map(|n| n.len() + 1)
            .sum::<usize>();

        let align = 0x1000;
        (size + align) & !(align - 1)
    }

    pub fn export_to_json(&self) -> Vec<JsonExportEntry> {
        let mut list = Vec::new();

        for (i, entry) in self.entries.iter().enumerate() {
            let entry_name = self.entry_names.get(i).cloned().unwrap_or_default();

            let mut json_entry = JsonExportEntry {
                name: entry_name,
                kind: entry.kind,
                unknown: entry.unknown,
                emitter_set_id: entry.emitter_set_id,
                external_model_flag: 0,
                external_model_id: 0,
                external_model_string: String::new(),
                variants: Vec::new(),
            };

            let model_idx = (entry.external_model_idx as i32) - 1;
            if model_idx >= 0 && (model_idx as usize) < self.effect_models.len() {
                json_entry.external_model_flag = self.effect_models[model_idx as usize];
                if (model_idx as usize) < self.external_model_names.len() {
                    json_entry.external_model_string =
                        self.external_model_names[model_idx as usize].clone();
                }
            }

            let start_idx = (entry.variant_start_idx as i32) - 1;
            for j in 0..entry.variant_count {
                if (start_idx + j as i32) >= 0
                    && (start_idx + j as i32) < self.effect_variants.len() as i32
                {
                    let var_idx = (start_idx + j as i32) as usize;
                    let variant = &self.effect_variants[var_idx];
                    let bone_name = self
                        .external_bone_names
                        .get(var_idx)
                        .cloned()
                        .unwrap_or_default();

                    json_entry.variants.push(JsonExportVariant {
                        start_frame: variant.start_frame,
                        emitter_set_id: variant.emitter_set_id,
                        bone_name,
                    });
                }
            }

            list.push(json_entry);
        }

        list
    }

    pub fn new(ptcl: PtclFile) -> Self {
        NamcoEffectFile {
            header: EffnHeader {
                magic: "EFFN".to_string(),
                version: 131_072,
                num_effects: 0,
                num_external_models: 0,
                multi_part_effects: 0,
                header_chunk_align: 1,
            },
            entries: Vec::new(),
            effect_variants: Vec::new(),
            effect_models: Vec::new(),
            entry_names: Vec::new(),
            external_model_names: Vec::new(),
            external_bone_names: Vec::new(),
            ptcl_file: Some(ptcl),
        }
    }

    pub fn from_json(json_content: &str, ptcl: Option<PtclFile>) -> std::io::Result<Self> {
        let entries: Vec<JsonExportEntry> = serde_json::from_str(json_content)?;

        let mut namco = match ptcl {
            Some(ptcl) => NamcoEffectFile::new(ptcl),
            None => NamcoEffectFile {
                header: EffnHeader {
                    magic: "EFFN".to_string(),
                    version: 131_072,
                    num_effects: 0,
                    num_external_models: 0,
                    multi_part_effects: 0,
                    header_chunk_align: 0xFFFF,
                },
                entries: Vec::new(),
                effect_variants: Vec::new(),
                effect_models: Vec::new(),
                entry_names: Vec::new(),
                external_model_names: Vec::new(),
                external_bone_names: Vec::new(),
                ptcl_file: None,
            },
        };
        namco.header.num_effects = entries.len() as u16;

        for entry in &entries {
            namco.entry_names.push(entry.name.clone());

            let mut effect_entry = EffectHeader {
                kind: entry.kind,
                unknown: entry.unknown,
                emitter_set_id: entry.emitter_set_id,
                external_model_idx: 0,
                variant_start_idx: 0,
                variant_count: 0,
            };

            if !entry.external_model_string.is_empty() {
                effect_entry.external_model_idx = namco.effect_models.len() as u32 + 1;
                namco.effect_models.push(entry.external_model_flag);
                namco
                    .external_model_names
                    .push(entry.external_model_string.clone());
            }

            if !entry.variants.is_empty() {
                effect_entry.variant_start_idx = namco.effect_variants.len() as u16 + 1;
                effect_entry.variant_count = entry.variants.len() as u16;
                for variant in &entry.variants {
                    namco.effect_variants.push(EffectVariant {
                        start_frame: variant.start_frame,
                        emitter_set_id: variant.emitter_set_id,
                    });
                    namco.external_bone_names.push(variant.bone_name.clone());
                }
            }

            namco.entries.push(effect_entry);
        }

        namco.header.multi_part_effects = namco.effect_variants.len() as u16;
        namco.header.num_external_models = namco.effect_models.len() as u16;

        Ok(namco)
    }

    pub fn save(&self) -> std::io::Result<Vec<u8>> {
        use std::io::Write;
        let mut data = Vec::new();

        let chunk_align = Self::get_required_chunk_align(
            &self.entries,
            &self.effect_variants,
            &self.entry_names,
            &self.external_model_names,
            &self.external_bone_names,
        );
        let mut header = self.header.clone();
        header.num_effects = self.entries.len() as u16;
        header.multi_part_effects = self.effect_variants.len() as u16;
        header.num_external_models = self.external_model_names.len() as u16;
        // The field is the PTCL offset in 0x1000 pages, and the reader seeks by exactly that. It
        // used to be written as 2 for any header of 0x2000 or more, which put the PTCL of a file
        // with a few hundred entries (a 0x3000 or 0x4000 header) where the reader never looks: it
        // then parsed the name table as PTCL and reported the file as having none.
        header.header_chunk_align = (chunk_align / 0x1000) as u16;

        // Write header
        data.write_all(header.magic.as_bytes())?;
        data.write_all(&header.version.to_le_bytes())?;
        data.write_all(&header.num_effects.to_le_bytes())?;
        data.write_all(&header.num_external_models.to_le_bytes())?;
        data.write_all(&header.multi_part_effects.to_le_bytes())?;
        data.write_all(&header.header_chunk_align.to_le_bytes())?;

        // Write entries
        for entry in &self.entries {
            data.write_all(&entry.kind.to_le_bytes())?;
            data.write_all(&entry.unknown.to_le_bytes())?;
            data.write_all(&entry.emitter_set_id.to_le_bytes())?;
            data.write_all(&entry.external_model_idx.to_le_bytes())?;
            data.write_all(&entry.variant_start_idx.to_le_bytes())?;
            data.write_all(&entry.variant_count.to_le_bytes())?;
        }

        // Write effect variants
        for variant in &self.effect_variants {
            data.write_all(&variant.start_frame.to_le_bytes())?;
            data.write_all(&variant.emitter_set_id.to_le_bytes())?;
        }

        // Write effect models
        data.write_all(&self.effect_models)?;

        // Write entry names
        for name in &self.entry_names {
            data.write_all(name.as_bytes())?;
            data.write_all(&[0u8])?; // null terminator
        }

        // Write external model names
        for name in &self.external_model_names {
            data.write_all(name.as_bytes())?;
            data.write_all(&[0u8])?;
        }

        // Write external bone names
        for name in &self.external_bone_names {
            data.write_all(name.as_bytes())?;
            data.write_all(&[0u8])?;
        }

        // Align to the page the header's own field names. Measured from what was actually
        // written rather than the estimate, and rounded to whole 0x1000 pages: masking with a
        // non-power-of-two alignment (0x3000) put the PTCL at 0x5000 while the header said 0x3000.
        let current_size = data.len();
        let pages = current_size.div_ceil(0x1000).max(chunk_align / 0x1000).max(1);
        let aligned_size = pages * 0x1000;
        data[14..16].copy_from_slice(&(pages as u16).to_le_bytes());
        let padding = aligned_size - current_size;
        data.extend(std::iter::repeat_n(0u8, padding));

        // Write PTCL data
        if let Some(ptcl) = &self.ptcl_file {
            data.extend(&ptcl.save());
        }

        Ok(data)
    }
}
