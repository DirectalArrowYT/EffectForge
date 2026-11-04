use crate::bfres::ResFile;
use crate::bnsh;
use crate::bntx;
use crate::dumper::serialize_ea_section_from_json;
use crate::emitter::EmitterData;
use crate::namco_file::NamcoEffectFile;
use crate::ptcl_file::{
    PrimitiveDescriptor, PrimitiveInfo, PtclFile, ShaderInfo, TextureDescriptor, TextureInfo,
};
use crate::structs::{Emitter, EmitterList, EmitterSet, EmitterSubSection};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct PtclHeaderFile {
    #[serde(rename = "Header")]
    header: PtclHeaderFields,
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct PtclHeaderFields {
    #[serde(rename = "Magic")]
    magic: u64,
    #[serde(rename = "GraphicsAPIVersion")]
    graphics_api_version: u16,
    #[serde(rename = "VFXVersion")]
    vfx_version: u16,
    #[serde(rename = "ByteOrder")]
    byte_order: u16,
    #[serde(rename = "Alignment")]
    alignment: u8,
    #[serde(rename = "TargetAddressSize")]
    target_address_size: u8,
    #[serde(rename = "NameOffset")]
    name_offset: u32,
    #[serde(rename = "Flag")]
    flag: u16,
    #[serde(rename = "BlockOffset")]
    block_offset: u16,
    #[serde(rename = "RelocationTableOffset")]
    relocation_table_offset: u32,
    #[serde(rename = "FileSize")]
    file_size: u32,
}

#[derive(Debug, Deserialize)]
struct OrderFile {
    #[serde(rename = "Order")]
    order: Vec<String>,
}

/// C# `PtclFileCreator.LoadEmitter` subsection order (`IndexOf`; unknown => -1).
const SUBSECTION_ORDER: &[&str] = &["FCOV", "CSDP", "CUDP", "CADP", "EAA0", "EAA1", "EATR"];

fn subsection_sort_key(magic: &str) -> i32 {
    SUBSECTION_ORDER
        .iter()
        .position(|item| *item == magic)
        .map(|pos| pos as i32)
        .unwrap_or(-1)
}

fn sort_subsections_like_csharp(subsections: &mut [EmitterSubSection]) {
    subsections.sort_by_key(|section| subsection_sort_key(&section.magic));
}

struct RebuildState {
    texture_blobs: Vec<Vec<u8>>,
    texture_descriptors: Vec<TextureDescriptor>,
    texture_ids: HashSet<(u64, String)>,
    primitive_blobs: Vec<Vec<u8>>,
    primitive_descriptors: Vec<PrimitiveDescriptor>,
    primitive_ids: HashSet<u64>,
    shader_blobs: Vec<Vec<u8>>,
    compute_blobs: Vec<Vec<u8>>,
    saved_orphan_compute: Option<Vec<u8>>,
}

impl RebuildState {
    fn from_ptcl(ptcl: &PtclFile) -> Self {
        let saved_orphan_compute = ptcl
            .shader_info
            .as_ref()
            .and_then(|info| info.compute_binary.clone());
        Self {
            texture_blobs: Vec::new(),
            texture_descriptors: Vec::new(),
            texture_ids: HashSet::new(),
            primitive_blobs: Vec::new(),
            primitive_descriptors: Vec::new(),
            primitive_ids: HashSet::new(),
            shader_blobs: Vec::new(),
            compute_blobs: Vec::new(),
            saved_orphan_compute,
        }
    }

    fn clear_pools(&self, ptcl: &mut PtclFile) {
        if let Some(texture_info) = &mut ptcl.texture_info {
            texture_info.descriptors.clear();
            texture_info.binary_data = Some(Vec::new());
        }
        if let Some(primitive_info) = &mut ptcl.primitive_info {
            primitive_info.descriptors.clear();
            primitive_info.binary_data = Some(Vec::new());
        }
        if let Some(shader_info) = &mut ptcl.shader_info {
            shader_info.variations.clear();
            shader_info.binary_data = Some(Vec::new());
            shader_info.compute_binary = None;
        }
    }

    fn add_texture(&mut self, id: u64, data: Vec<u8>) -> io::Result<()> {
        let name = bntx::first_texture_name(&data)?;
        if !self.texture_ids.insert((id, name.clone())) {
            return Ok(());
        }
        self.texture_descriptors
            .push(TextureDescriptor { id, name });
        self.texture_blobs.push(data);
        Ok(())
    }

    fn add_primitive(&mut self, id: u64, data: Vec<u8>) -> io::Result<()> {
        if !self.primitive_ids.insert(id) {
            return Ok(());
        }
        let indices = ResFile::first_model_attribute_indices(&data)?;
        self.primitive_descriptors.push(PrimitiveDescriptor {
            id,
            position_index: *indices.get("_p0").unwrap_or(&-1),
            normal_index: *indices.get("_n0").unwrap_or(&-1),
            tangent_index: *indices.get("_t0").unwrap_or(&-1),
            color_index: *indices.get("_c0").unwrap_or(&-1),
            tex_coord0_index: *indices.get("_u0").unwrap_or(&-1),
            tex_coord1_index: *indices.get("_u1").unwrap_or(&-1),
            padding: 0,
        });
        self.primitive_blobs.push(data);
        Ok(())
    }

    fn add_shader_variation(&mut self, data: Vec<u8>) {
        self.shader_blobs.push(data);
    }

    fn add_compute_variation(&mut self, data: Vec<u8>) {
        self.compute_blobs.push(data);
    }

    fn restore_orphan_compute(&mut self) {
        if !self.compute_blobs.is_empty() {
            return;
        }
        let Some(saved) = self.saved_orphan_compute.as_ref() else {
            return;
        };
        if bnsh::BnshFile::read(saved)
            .map(|file| file.variations.len())
            .unwrap_or(0)
            == 1
        {
            self.compute_blobs.push(saved.clone());
        }
    }

    fn finalize_pools(
        &self,
        ptcl: &mut PtclFile,
        saved_texture: Option<&TextureInfo>,
        saved_primitive: Option<&PrimitiveInfo>,
        saved_shader: Option<&ShaderInfo>,
    ) -> io::Result<()> {
        if !self.texture_blobs.is_empty() {
            let base = saved_texture
                .and_then(|info| info.binary_data.as_deref())
                .unwrap_or(&[]);
            let binary_data = bntx::rebuild_from_base_and_exports(base, &self.texture_blobs)?;
            let desc_table_magic = saved_texture
                .map(|info| info.desc_table_magic)
                .unwrap_or(*b"GTNT");
            ptcl.texture_info = Some(TextureInfo {
                descriptors: self.texture_descriptors.clone(),
                binary_data: Some(binary_data),
                section_offset: saved_texture.map(|info| info.section_offset).unwrap_or(0),
                desc_table_magic,
            });
        }

        if !self.primitive_blobs.is_empty() {
            let base = saved_primitive
                .and_then(|info| info.binary_data.as_deref())
                .unwrap_or(&[]);
            let binary_data = ResFile::rebuild_from_base_and_exports(base, &self.primitive_blobs)?;
            let desc_table_magic = saved_primitive
                .map(|info| info.desc_table_magic)
                .unwrap_or(*b"G3NT");
            ptcl.primitive_info = Some(PrimitiveInfo {
                descriptors: self.primitive_descriptors.clone(),
                binary_data: Some(binary_data),
                section_offset: saved_primitive.map(|info| info.section_offset).unwrap_or(0),
                desc_table_magic,
                preserve_binary_data: false,
            });
        }

        if !self.shader_blobs.is_empty() || !self.compute_blobs.is_empty() {
            let shader_base = saved_shader
                .and_then(|info| info.binary_data.as_deref())
                .unwrap_or(&[]);
            let binary_data = if self.shader_blobs.is_empty() {
                Vec::new()
            } else {
                bnsh::rebuild_from_base_and_exports(shader_base, &self.shader_blobs)?
            };
            let compute_base = saved_shader
                .and_then(|info| info.compute_binary.as_deref())
                .unwrap_or(&[]);
            let compute_binary = if self.compute_blobs.is_empty() {
                None
            } else {
                Some(bnsh::rebuild_from_base_and_exports(
                    compute_base,
                    &self.compute_blobs,
                )?)
            };
            ptcl.shader_info = Some(ShaderInfo {
                binary_data: Some(binary_data),
                compute_binary,
                section_offset: saved_shader.map(|info| info.section_offset).unwrap_or(0),
                compute_section_offset: saved_shader.and_then(|info| info.compute_section_offset),
                variations: Vec::new(),
            });
        }

        Ok(())
    }
}

pub struct Creator;

impl Creator {
    /// Create a PTCL file from a decompiled folder structure (C# `PtclFileCreator.FromFolder`).
    pub fn create_ptcl_from_folder(folder: &str) -> io::Result<PtclFile> {
        let folder_path = Path::new(folder);
        let base_ptcl_path = folder_path.join("Base.ptcl");
        if !base_ptcl_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Base.ptcl not found",
            ));
        }

        let base_data = fs::read(&base_ptcl_path)?;
        let mut ptcl = PtclFile::load_for_rebuild(&base_data)?;
        apply_ptcl_header(&mut ptcl, folder_path)?;

        let mut state = RebuildState::from_ptcl(&ptcl);
        let saved_texture_info = ptcl.texture_info.clone();
        let saved_shader_info = ptcl.shader_info.clone();
        let saved_primitive_info = ptcl.primitive_info.clone();
        state.clear_pools(&mut ptcl);
        ptcl.emitter_list = EmitterList {
            emitter_sets: Vec::new(),
        };

        let version = ptcl.vfx_version;
        for set_dir in ordered_subdirs(folder_path, "EmitterSetInfo.txt")? {
            let set_name = dir_name(&set_dir);
            let mut emitter_set = EmitterSet {
                name: set_name.clone(),
                unknown1: None,
                unknown2: None,
                unknown3: None,
                unknown4: None,
                unknown5: None,
                unknown6: None,
                emitters: Vec::new(),
            };

            for emitter_dir in ordered_subdirs(&set_dir, "EmitterOrder.txt")? {
                emitter_set
                    .emitters
                    .push(load_emitter(&mut state, &emitter_dir, version)?);
            }

            emitter_set
                .emitters
                .sort_by_key(|emitter| emitter.data.order);
            ptcl.emitter_list.emitter_sets.push(emitter_set);
        }

        state.restore_orphan_compute();
        apply_rebuilt_pools(
            &mut ptcl,
            &state,
            saved_texture_info,
            saved_shader_info,
            saved_primitive_info,
        )?;
        Ok(ptcl)
    }

    /// Create a NAMCO effect file from a decompiled folder structure.
    pub fn create_namco_from_folder(folder: &str) -> io::Result<Option<NamcoEffectFile>> {
        let folder_path = Path::new(folder);
        let base_ptcl_path = folder_path.join("Base.ptcl");
        let namco_json_path = folder_path.join("NamcoFile.json");

        if base_ptcl_path.exists() {
            let ptcl = Self::create_ptcl_from_folder(folder)?;
            if namco_json_path.exists() {
                let content = fs::read_to_string(&namco_json_path)?;
                NamcoEffectFile::from_json(&content, Some(ptcl)).map(Some)
            } else {
                Ok(Some(NamcoEffectFile::new(ptcl)))
            }
        } else if namco_json_path.exists() {
            // Header-only EFFN (e.g. ef_jack_cutin): C# build returns without output.
            Ok(None)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Base.ptcl not found",
            ))
        }
    }
}

fn apply_ptcl_header(ptcl: &mut PtclFile, folder: &Path) -> io::Result<()> {
    let header_path = folder.join("PtclHeader.txt");
    if !header_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&header_path)?;
    let info: PtclHeaderFile = serde_json::from_str(&content)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    ptcl.magic = info.header.magic;
    ptcl.graphics_api_version = info.header.graphics_api_version;
    ptcl.vfx_version = info.header.vfx_version;
    ptcl.byte_order = info.header.byte_order;
    ptcl.alignment = info.header.alignment;
    ptcl.is_version_64_bit = info.header.target_address_size == 64;
    ptcl.name_offset = info.header.name_offset;
    ptcl.flag = info.header.flag;
    ptcl.block_offset = info.header.block_offset;
    ptcl.relocation_table_offset = info.header.relocation_table_offset;
    ptcl.file_size = info.header.file_size;
    ptcl.name = info.name;
    Ok(())
}

fn ordered_subdirs(parent: &Path, order_file_name: &str) -> io::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(parent)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();

    let order_path = parent.join(order_file_name);
    if order_path.is_file() {
        let content = fs::read_to_string(&order_path)?;
        let order: OrderFile = serde_json::from_str(&content)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        dirs.sort_by_key(|path| {
            let name = dir_name(path);
            order
                .order
                .iter()
                .position(|item| item == &name)
                .map(|pos| pos as i32)
                .unwrap_or(-1)
        });
    } else {
        dirs.sort_by_key(|left| dir_name(left));
    }

    Ok(dirs)
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string()
}

fn load_emitter(state: &mut RebuildState, dir: &Path, version: u16) -> io::Result<Emitter> {
    let json_path = dir.join("EmitterData.json");
    let content = fs::read_to_string(&json_path)?;
    let mut data = EmitterData::from_json(&content, version)?;

    let mut subsections = Vec::new();
    let mut child_dirs = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            child_dirs.push(path);
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if file_name == "EmitterData.bin" || file_name == "EmitterData.json" {
            continue;
        }

        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if ext == "bntx" {
            let Some(id_str) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(id) = id_str.parse::<u64>() else {
                continue;
            };
            state.add_texture(id, fs::read(path)?)?;
            continue;
        }
        if ext == "bfres" {
            let Some(id_str) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(id) = id_str.parse::<u64>() else {
                continue;
            };
            state.add_primitive(id, fs::read(path)?)?;
            continue;
        }
        if ext != "bin" && ext != "json" {
            continue;
        }

        let magic = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_string();
        if magic.is_empty() {
            continue;
        }

        let subsection_data = if magic.starts_with("EA") && ext == "json" {
            let json = fs::read_to_string(&path)?;
            serialize_ea_section_from_json(&json)?
        } else {
            fs::read(&path)?
        };

        subsections.push(EmitterSubSection {
            magic,
            data: subsection_data,
        });
    }

    sort_subsections_like_csharp(&mut subsections);

    data.shader_references.shader_index = state.shader_blobs.len() as i32;
    let shader_path = dir.join("Shader.bnsh");
    if !shader_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("No shader present for emitter in {}", dir.display()),
        ));
    }
    state.add_shader_variation(fs::read(shader_path)?);

    let user_shader1_path = dir.join("UserShader1.bnsh");
    if user_shader1_path.is_file() {
        data.shader_references.user_shader_index1 = state.shader_blobs.len() as i32;
        state.add_shader_variation(fs::read(user_shader1_path)?);
    }

    let user_shader2_path = dir.join("UserShader2.bnsh");
    if user_shader2_path.is_file() {
        data.shader_references.user_shader_index2 = state.shader_blobs.len() as i32;
        state.add_shader_variation(fs::read(user_shader2_path)?);
    }

    let compute_shader_path = dir.join("ComputeShader.bnsh");
    if compute_shader_path.is_file() {
        data.shader_references.compute_shader_index = state.compute_blobs.len() as i32;
        state.add_compute_variation(fs::read(compute_shader_path)?);
    }

    let mut children = Vec::new();
    for child_dir in child_dirs {
        children.push(load_emitter(state, &child_dir, version)?);
    }
    children.sort_by_key(|child| child.data.order);

    let cached_binary = data.write(version).ok();
    let binary_data = if cached_binary.is_none() {
        dir.join("EmitterData.bin")
            .is_file()
            .then(|| fs::read(dir.join("EmitterData.bin")))
            .transpose()?
    } else {
        None
    };

    Ok(Emitter {
        data,
        binary_data,
        cached_binary,
        subsections,
        children,
    })
}

fn apply_rebuilt_pools(
    ptcl: &mut PtclFile,
    state: &RebuildState,
    saved_texture: Option<TextureInfo>,
    saved_shader: Option<ShaderInfo>,
    saved_primitive: Option<PrimitiveInfo>,
) -> io::Result<()> {
    state.finalize_pools(
        ptcl,
        saved_texture.as_ref(),
        saved_primitive.as_ref(),
        saved_shader.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_emitter_write_matches_dumped_bin() {
        let folder = Path::new("/tmp/esta_ef_common/ef_common");
        if !folder.is_dir() {
            return;
        }
        let header = fs::read_to_string(folder.join("PtclHeader.txt")).expect("header");
        let info: PtclHeaderFile = serde_json::from_str(&header).expect("parse header");
        let version = info.header.vfx_version;
        let mut mismatches = 0usize;
        let mut stack = vec![folder.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().and_then(|name| name.to_str()) != Some("EmitterData.json") {
                    continue;
                }
                let bin_path = path.with_file_name("EmitterData.bin");
                if !bin_path.is_file() {
                    continue;
                }
                let json = fs::read_to_string(&path).expect("json");
                let data = EmitterData::from_json(&json, version).expect("from_json");
                let written = data.write(version).expect("write");
                let original = fs::read(&bin_path).expect("bin");
                if written != original {
                    mismatches += 1;
                    if mismatches <= 10 {
                        let first_diff = written
                            .iter()
                            .zip(original.iter())
                            .position(|(left, right)| left != right)
                            .unwrap_or(0);
                        eprintln!(
                            "mismatch {} at byte {}",
                            path.strip_prefix(folder).unwrap().display(),
                            first_diff
                        );
                    }
                }
            }
        }
        assert_eq!(
            mismatches, 0,
            "{mismatches} emitters failed json->write roundtrip vs bin"
        );
    }

    #[test]
    fn sparks1c_m_subsection_order_matches_base() {
        let folder = Path::new("/tmp/matchup_fix/cs/ef_matchup");
        if !folder.is_dir() {
            return;
        }
        let ptcl = Creator::create_ptcl_from_folder(folder.to_str().unwrap()).expect("build");
        let set = ptcl
            .emitter_list
            .emitter_sets
            .iter()
            .find(|set| set.name == "P_MatchUpSparks")
            .expect("set");
        let emitter = set
            .emitters
            .iter()
            .find(|emitter| emitter.data.display_name() == "sparks1c_M")
            .expect("emitter");
        let magics: Vec<_> = emitter
            .subsections
            .iter()
            .map(|s| s.magic.clone())
            .collect();
        assert_eq!(magics, vec!["FCLN".to_string(), "EATR".to_string()]);
    }

    #[test]
    fn ef_matchup_matches_csharp_rebuild() {
        let folder = "/tmp/matchup_fix/cs/ef_matchup";
        let cs_path = "/tmp/matchup_fix/cs/ef_matchup_NEW.eff";
        if !Path::new(folder).is_dir() || !Path::new(cs_path).is_file() {
            return;
        }
        let namco = Creator::create_namco_from_folder(folder)
            .expect("create")
            .expect("some");
        let eff = namco.save().expect("save");
        let cs = fs::read(cs_path).expect("cs");
        if eff != cs {
            let diff = eff
                .iter()
                .zip(cs.iter())
                .position(|(left, right)| left != right)
                .unwrap_or(0);
            panic!("byte mismatch with C# rebuild at offset {diff}");
        }
    }
}
