use crate::emitter::EmitterData;
use crate::namco_file::NamcoEffectFile;
use crate::ptcl_file::PtclFile;
use crate::reader::ReaderExt;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, to_string_pretty};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

type BnshCacheKey = (usize, usize, usize);
type BnshSourceKey = (usize, usize);

struct ResourceCache {
    bfres: HashMap<u64, Vec<u8>>,
    bnsh: HashMap<BnshCacheKey, Vec<u8>>,
    bntx: HashMap<u64, Vec<u8>>,
    texture_id_to_index: HashMap<u64, usize>,
    bnsh_parsed: HashMap<BnshSourceKey, crate::bnsh::BnshFile>,
}

fn bnsh_cache_key(data: &[u8], variation_idx: usize) -> BnshCacheKey {
    (data.as_ptr() as usize, data.len(), variation_idx)
}

fn bnsh_source_key(data: &[u8]) -> BnshSourceKey {
    (data.as_ptr() as usize, data.len())
}

impl ResourceCache {
    fn new(ptcl: &PtclFile) -> Self {
        let texture_id_to_index = ptcl
            .texture_info
            .as_ref()
            .map(|texture_info| {
                texture_info
                    .descriptors
                    .iter()
                    .enumerate()
                    .map(|(index, descriptor)| (descriptor.id, index))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            bfres: HashMap::new(),
            bnsh: HashMap::new(),
            bntx: HashMap::new(),
            texture_id_to_index,
            bnsh_parsed: HashMap::new(),
        }
    }

    fn texture_index(&self, texture_id: u64) -> Option<usize> {
        self.texture_id_to_index.get(&texture_id).copied()
    }

    fn bfres_export<'a>(
        &mut self,
        bfres_session: &mut Option<crate::bfres::load::ResExportSession<'a>>,
        primitive_info: &'a crate::ptcl_file::PrimitiveInfo,
        id: u64,
    ) -> Option<&Vec<u8>> {
        if let std::collections::hash_map::Entry::Vacant(e) = self.bfres.entry(id) {
            let model_index =
                crate::bfres::descriptor_index_for_id(&primitive_info.descriptors, id)?;
            let source = primitive_info.binary_data.as_ref()?;
            let data =
                crate::bfres::export_single_model_with_session(bfres_session, source, model_index)
                    .ok()?;
            e.insert(data);
        }
        self.bfres.get(&id)
    }

    fn bnsh_export(
        &mut self,
        whole_data: &[u8],
        variation_idx: usize,
    ) -> std::io::Result<&Vec<u8>> {
        let key = bnsh_cache_key(whole_data, variation_idx);
        if !self.bnsh.contains_key(&key) {
            let source_key = bnsh_source_key(whole_data);
            if let std::collections::hash_map::Entry::Vacant(e) = self.bnsh_parsed.entry(source_key)
            {
                if let Ok(whole) = crate::bnsh::BnshFile::read(whole_data) {
                    e.insert(whole);
                }
            }
            let bytes = if let Some(whole) = self.bnsh_parsed.get(&source_key) {
                build_single_variation_from_parsed(whole, variation_idx)?
            } else {
                whole_data.to_vec()
            };
            self.bnsh.insert(key, bytes);
        }
        Ok(self.bnsh.get(&key).expect("bnsh cache populated"))
    }

    fn bntx_export(
        &mut self,
        global_bntx: &[u8],
        texture_index: usize,
        texture_name: &str,
        texture_id: u64,
    ) -> std::io::Result<&Vec<u8>> {
        if let std::collections::hash_map::Entry::Vacant(e) = self.bntx.entry(texture_id) {
            let bytes =
                crate::bntx::build_single_texture_bntx(global_bntx, texture_index, texture_name)?;
            e.insert(bytes);
        }
        Ok(self.bntx.get(&texture_id).expect("bntx cache populated"))
    }
}

fn build_single_variation_from_parsed(
    whole: &crate::bnsh::BnshFile,
    variation_idx: usize,
) -> std::io::Result<Vec<u8>> {
    use crate::bnsh;

    if variation_idx >= whole.variations.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "variation index {} out of range (max {})",
                variation_idx,
                whole.variations.len()
            ),
        ));
    }

    let bin_header = bnsh::BinaryHeader {
        magic: 0x48534E42,
        version_micro: 0,
        version_minor: 0,
        version_major: 0,
        byte_order: 0,
        alignment: 0,
        target_address_size: 0,
        flag: 0,
        block_offset: 96,
        relocation_table_offset: 0,
        file_size: 0,
        name_offset: 0,
    };

    let header = bnsh::BnshHeader {
        magic: 0x63737267,
        block_offset: 0,
        block_size: 0,
        padding: 0,
        api_type: 0,
        api_version: 0,
        code_target: 0,
        compiler_version: 0,
        num_variation: 1,
        variation_start_offset: 0,
        memory_pool_offset: 0,
        unknown2: 0,
    };

    let single = bnsh::BnshFile {
        bin_header,
        header,
        name: "dummy".to_string(),
        variations: vec![whole.variations[variation_idx].clone()],
    };
    Ok(single.write())
}

/// Normalize JSON formatting to match C# output
/// - Convert small non-zero floats to uppercase scientific notation used by Newtonsoft
/// - Pad exponents to at least two digits
fn normalize_json(json_str: &str) -> String {
    let bytes = json_str.as_bytes();
    let mut out = String::with_capacity(json_str.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let ch = bytes[i];
        if in_string {
            out.push(ch as char);
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }

        if ch == b'-' || ch.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-') {
                    i += 1;
                } else {
                    break;
                }
            }

            write_normalized_json_number(&mut out, &json_str[start..i]);
            continue;
        }

        out.push(ch as char);
        i += 1;
    }

    out
}

fn write_normalized_json_number(out: &mut String, token: &str) {
    let parsed = match token.parse::<f64>() {
        Ok(value) => value,
        Err(_) => {
            out.push_str(token);
            return;
        }
    };

    if parsed != 0.0 && parsed.abs() < 1e-4 {
        out.push_str(&format_scientific_csharp(parsed));
        return;
    }

    if token.as_bytes().iter().any(|&b| b == b'e' || b == b'E') {
        out.push_str(&format_scientific_csharp(parsed));
        return;
    }

    out.push_str(token);
}

fn format_scientific_csharp(value: f64) -> String {
    let raw = format!("{:E}", value);
    let (mantissa, exponent) = raw.split_once('E').unwrap_or((&raw, "0"));
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');

    let sign = if exponent.starts_with('-') { '-' } else { '+' };
    let digits = exponent
        .trim_start_matches(['+', '-'])
        .parse::<i32>()
        .unwrap_or(0);

    format!("{mantissa}E{sign}{:02}", digits)
}

#[derive(Serialize, Deserialize)]
struct EmitterAnimationSection {
    #[serde(rename = "Enable")]
    enable: bool,
    #[serde(rename = "Loop")]
    loop_: bool,
    #[serde(rename = "RandomizeStartFrame")]
    randomize_start_frame: bool,
    #[serde(rename = "Reserved")]
    reserved: u8,
    #[serde(rename = "LoopCount")]
    loop_count: u32,
    #[serde(rename = "KeyFrames")]
    key_frames: Vec<EmitterAnimationKeyFrame>,
}

#[derive(Serialize, Deserialize)]
struct EmitterAnimationKeyFrame {
    #[serde(rename = "X")]
    x: f32,
    #[serde(rename = "Y")]
    y: f32,
    #[serde(rename = "Z")]
    z: f32,
    #[serde(rename = "Time")]
    time: f32,
}

fn parse_ea_section(data: &[u8]) -> std::io::Result<EmitterAnimationSection> {
    let mut cursor = Cursor::new(data);
    let enable = cursor.read_u8()? != 0;
    let loop_ = cursor.read_u8()? != 0;
    let randomize_start_frame = cursor.read_u8()? != 0;
    let reserved = cursor.read_u8()?;
    let num_keys = cursor.read_u32_le()? as usize;
    let loop_count = cursor.read_u32_le()?;

    let mut key_frames = Vec::with_capacity(num_keys);
    for _ in 0..num_keys {
        key_frames.push(EmitterAnimationKeyFrame {
            x: cursor.read_f32_le()?,
            y: cursor.read_f32_le()?,
            z: cursor.read_f32_le()?,
            time: cursor.read_f32_le()?,
        });
    }

    Ok(EmitterAnimationSection {
        enable,
        loop_,
        randomize_start_frame,
        reserved,
        loop_count,
        key_frames,
    })
}

/// Serialize an EA emitter animation subsection from dumped JSON.
pub fn serialize_ea_section_from_json(json: &str) -> std::io::Result<Vec<u8>> {
    let section: EmitterAnimationSection = serde_json::from_str(json)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

    let mut data = vec![
        section.enable as u8,
        section.loop_ as u8,
        section.randomize_start_frame as u8,
        section.reserved,
    ];
    data.write_all(&(section.key_frames.len() as u32).to_le_bytes())?;
    data.write_all(&section.loop_count.to_le_bytes())?;
    for key in &section.key_frames {
        data.write_all(&key.x.to_le_bytes())?;
        data.write_all(&key.y.to_le_bytes())?;
        data.write_all(&key.z.to_le_bytes())?;
        data.write_all(&key.time.to_le_bytes())?;
    }
    Ok(data)
}

/// Converts SCREAMING_SNAKE_CASE to PascalCase with common effect-name abbreviations expanded.
fn snake_to_pascal(s: &str) -> String {
    let mut result = String::new();
    let parts: Vec<&str> = s.split('_').collect();

    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        let prev = if idx == 0 {
            None
        } else {
            parts.get(idx - 1).copied()
        };
        result.push_str(&snake_token_to_pascal(part, prev));
    }

    result
}

fn snake_token_to_pascal(token: &str, prev_token: Option<&str>) -> String {
    let token_upper = token.to_ascii_uppercase();

    // Context-sensitive expansions for generic effect naming conventions
    if let Some(prev) = prev_token {
        let prev_upper = prev.to_ascii_uppercase();
        if prev_upper == "STONE" {
            return match token_upper.as_str() {
                "S" => "Start".to_string(),
                "E" => "End".to_string(),
                _ => pascalize(token),
            };
        }
    }

    if token_upper == "FINAL" {
        return "Final".to_string();
    }

    if token_upper.starts_with("FIN") {
        let tail = &token[3..];
        if tail.is_empty() {
            return "Final".to_string();
        }
        return format!("{}Final", pascalize(tail));
    }

    match token_upper.as_str() {
        "ATK" => "Attack".to_string(),
        "ATK100" => "Attack100".to_string(),
        "FCUT" => "Finalcutter".to_string(),
        "CUT" => "Cut".to_string(),
        "HIT" => "Hit".to_string(),
        "BOMB" => "Bomb".to_string(),
        "ARC" => "Arc".to_string(),
        "BG" => "Bg".to_string(),
        "AURA" => "Aura".to_string(),
        "STAR" => "Star".to_string(),
        "LIGHT" => "Light".to_string(),
        "DASH" => "Dash".to_string(),
        "ENTRY" => "Entry".to_string(),
        "BODY" => "Body".to_string(),
        "HOLD" => "Hold".to_string(),
        "IMPACT" => "Impact".to_string(),
        "ONIGOROSHI" => "Onigoroshi".to_string(),
        "RISE" => "Rise".to_string(),
        "SMASH" => "Smash".to_string(),
        "SMOKE" => "Smoke".to_string(),
        "STONE" => "Stone".to_string(),
        "SWORD" => "Sword".to_string(),
        "THUNDER" => "Thunder".to_string(),
        "TRACE" => "Trace".to_string(),
        "VACUUM" => "Vacuum".to_string(),
        "WIND" => "Wind".to_string(),
        _ => pascalize(token),
    }
}

fn pascalize(token: &str) -> String {
    let mut chars = token.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

/// Converts entry names like "KIRBY_ATTACK_LINE" to emitter set names like "P_KirbyAttackLine"
fn entry_name_to_emitter_set_name(entry_name: &str) -> String {
    format!("P_{}", snake_to_pascal(entry_name))
}

pub struct Dumper;

impl Dumper {
    /// Dump a PTCL file to a directory structure
    pub fn dump_ptcl(ptcl: &PtclFile, output_dir: &str) -> std::io::Result<()> {
        fs::create_dir_all(output_dir)?;

        let header_json = format!("{{\n  \"Header\": {{\n    \"Magic\": 2314885531392558678,\n    \"GraphicsAPIVersion\": {},\n    \"VFXVersion\": {},\n    \"ByteOrder\": {},\n    \"Alignment\": 12,\n    \"TargetAddressSize\": {},\n    \"NameOffset\": 32,\n    \"Flag\": 0,\n    \"BlockOffset\": 64,\n    \"RelocationTableOffset\": 0,\n    \"FileSize\": {}\n  }},\n  \"Name\": \"\"\n}}",
            0x400,
            ptcl.vfx_version,
            0xFEFF,
            if ptcl.is_version_64_bit { 64 } else { 32 },
            ptcl.file_size
        );
        let header_path = Path::new(output_dir).join("PtclHeader.txt");
        fs::write(header_path, header_json)?;
        if !ptcl.base_bytes.is_empty() {
            fs::write(Path::new(output_dir).join("Base.ptcl"), ptcl.save())?;
        }

        let emitter_set_names: Vec<String> = ptcl
            .emitter_list
            .emitter_sets
            .iter()
            .map(|set| set.name.clone())
            .collect();

        let set_info_path = Path::new(output_dir).join("EmitterSetInfo.txt");
        fs::write(
            set_info_path,
            to_string_pretty(&json!({"Order": emitter_set_names}))?,
        )?;

        let mut cache = ResourceCache::new(ptcl);
        let mut bfres_session = None;
        for emitter_set in &ptcl.emitter_list.emitter_sets {
            Self::dump_emitter_set(
                emitter_set,
                Path::new(output_dir),
                Some(ptcl),
                &emitter_set.name,
                &mut cache,
                &mut bfres_session,
            )?;
        }

        Ok(())
    }

    /// Dump a NAMCO (EFFN) file to a directory structure
    pub fn dump_namco(namco: &NamcoEffectFile, output_dir: &str) -> std::io::Result<()> {
        fs::create_dir_all(output_dir)?;

        let json_export = namco.export_to_json();
        let namco_path = Path::new(output_dir).join("NamcoFile.json");
        fs::write(namco_path, to_string_pretty(&json_export)?)?;

        if let Some(ptcl) = &namco.ptcl_file {
            if !ptcl.emitter_list.emitter_sets.is_empty() {
                let emitter_set_names: Vec<String> = ptcl
                    .emitter_list
                    .emitter_sets
                    .iter()
                    .map(|set| set.name.clone())
                    .collect();

                let set_info_path = Path::new(output_dir).join("EmitterSetInfo.txt");
                fs::write(
                    set_info_path,
                    to_string_pretty(&json!({"Order": emitter_set_names}))?,
                )?;

                let mut cache = ResourceCache::new(ptcl);
                let mut bfres_session = None;
                for (idx, emitter_set) in ptcl.emitter_list.emitter_sets.iter().enumerate() {
                    if idx >= emitter_set_names.len() {
                        continue;
                    }
                    let set_name = emitter_set_names
                        .get(idx)
                        .unwrap_or(&emitter_set.name)
                        .clone();
                    Self::dump_emitter_set(
                        emitter_set,
                        Path::new(output_dir),
                        Some(ptcl),
                        &set_name,
                        &mut cache,
                        &mut bfres_session,
                    )?;
                }
            }
        } else {
            for (entry_idx, entry) in namco.entries.iter().enumerate() {
                if entry.emitter_set_id == 0 {
                    continue;
                }
                let entry_name = namco
                    .entry_names
                    .get(entry_idx)
                    .unwrap_or(&format!("Entry_{}", entry_idx))
                    .clone();
                let set_name = entry_name_to_emitter_set_name(&entry_name);
                let set_dir = Path::new(output_dir).join(&set_name);
                fs::create_dir_all(&set_dir)?;
                fs::write(
                    set_dir.join("EmitterOrder.txt"),
                    to_string_pretty(&json!({"Order": []}))?,
                )?;
            }
        }

        if let Some(ptcl) = namco.ptcl_file.as_ref() {
            if !ptcl.base_bytes.is_empty() {
                fs::write(Path::new(output_dir).join("Base.ptcl"), ptcl.save())?;
            }
            let header_json = format!("{{\n  \"Header\": {{\n    \"Magic\": 2314885531392558678,\n    \"GraphicsAPIVersion\": {},\n    \"VFXVersion\": {},\n    \"ByteOrder\": {},\n    \"Alignment\": 12,\n    \"TargetAddressSize\": {},\n    \"NameOffset\": 32,\n    \"Flag\": 0,\n    \"BlockOffset\": 64,\n    \"RelocationTableOffset\": 0,\n    \"FileSize\": {}\n  }},\n  \"Name\": \"\"\n}}",
                0x400,
                ptcl.vfx_version,
                ptcl.byte_order,
                if ptcl.is_version_64_bit { 64 } else { 32 },
                ptcl.file_size
            );
            fs::write(Path::new(output_dir).join("PtclHeader.txt"), header_json)?;
        }

        Ok(())
    }

    fn dump_emitter_set<'a>(
        emitter_set: &crate::structs::EmitterSet,
        output_dir: &Path,
        ptcl: Option<&'a PtclFile>,
        set_name: &str,
        cache: &mut ResourceCache,
        bfres_session: &mut Option<crate::bfres::load::ResExportSession<'a>>,
    ) -> std::io::Result<()> {
        let set_dir = output_dir.join(set_name);
        fs::create_dir_all(&set_dir)?;

        let emitter_order: Vec<String> = emitter_set
            .emitters
            .iter()
            .map(|emitter| {
                let name = emitter.data.display_name();
                if name.is_empty() {
                    format!("emitter_{}", emitter.data.order)
                } else {
                    name
                }
            })
            .collect();

        fs::write(
            set_dir.join("EmitterOrder.txt"),
            to_string_pretty(&json!({"Order": emitter_order}))?,
        )?;

        for emitter in &emitter_set.emitters {
            Self::dump_emitter(emitter, &set_dir, ptcl, cache, bfres_session)?;
        }

        Ok(())
    }

    fn dump_emitter<'a>(
        emitter: &crate::structs::Emitter,
        output_dir: &Path,
        ptcl: Option<&'a PtclFile>,
        cache: &mut ResourceCache,
        bfres_session: &mut Option<crate::bfres::load::ResExportSession<'a>>,
    ) -> std::io::Result<()> {
        let emitter_name = emitter.data.display_name();
        let emitter_name = if emitter_name.is_empty() {
            format!("emitter_{}", emitter.data.order)
        } else {
            emitter_name
        };

        let emitter_dir = output_dir.join(&emitter_name);
        fs::create_dir_all(&emitter_dir)?;

        if let Some(binary_data) = &emitter.binary_data {
            fs::write(emitter_dir.join("EmitterData.bin"), binary_data)?;
        }

        let json_string = to_string_pretty(&emitter.data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let normalized_json = normalize_json(&json_string);
        fs::write(emitter_dir.join("EmitterData.json"), normalized_json)?;

        if let Some(ptcl) = ptcl {
            Self::dump_emitter_resources(emitter, &emitter_dir, ptcl, cache, bfres_session)?;
        }

        for subsection in &emitter.subsections {
            if subsection.magic.starts_with("EA") {
                let animation = parse_ea_section(&subsection.data)?;
                let json_string = to_string_pretty(&animation)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                let normalized_json = normalize_json(&json_string);
                fs::write(
                    emitter_dir.join(format!("{}.json", subsection.magic)),
                    normalized_json,
                )?;
            } else {
                let filename = format!("{}.bin", subsection.magic);
                fs::write(emitter_dir.join(filename), &subsection.data)?;
            }
        }

        for child in &emitter.children {
            Self::dump_emitter(child, &emitter_dir, ptcl, cache, bfres_session)?;
        }

        Ok(())
    }

    fn dump_emitter_resources<'a>(
        emitter: &crate::structs::Emitter,
        emitter_dir: &Path,
        ptcl: &'a PtclFile,
        cache: &mut ResourceCache,
        bfres_session: &mut Option<crate::bfres::load::ResExportSession<'a>>,
    ) -> std::io::Result<()> {
        Self::dump_emitter_shaders(emitter, emitter_dir, ptcl, cache)?;
        Self::dump_emitter_primitives(emitter, emitter_dir, ptcl, cache, bfres_session)?;

        if let Some(texture_info) = &ptcl.texture_info {
            if let Some(global_bntx) = &texture_info.binary_data {
                let mut written_textures = HashSet::new();
                for sampler in emitter.data.get_samplers() {
                    if !written_textures.insert(sampler.texture_id) {
                        continue;
                    }
                    let Some(texture_index) = cache.texture_index(sampler.texture_id) else {
                        continue;
                    };
                    let descriptor = &texture_info.descriptors[texture_index];
                    let path = emitter_dir.join(format!("{}.bntx", sampler.texture_id));
                    match cache.bntx_export(
                        global_bntx,
                        texture_index,
                        &descriptor.name,
                        sampler.texture_id,
                    ) {
                        Ok(bytes) => {
                            if let Err(err) = fs::write(&path, bytes) {
                                eprintln!(
                                    "failed to export texture {} ({}) for {:?}: {}",
                                    sampler.texture_id, descriptor.name, emitter.data.name, err
                                );
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "failed to export texture {} ({}) for {:?}: {}",
                                sampler.texture_id, descriptor.name, emitter.data.name, err
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn dump_emitter_shaders(
        emitter: &crate::structs::Emitter,
        emitter_dir: &Path,
        ptcl: &PtclFile,
        cache: &mut ResourceCache,
    ) -> std::io::Result<()> {
        let write_variation_bnsh = |cache: &mut ResourceCache,
                                    path: &Path,
                                    whole_data: &[u8],
                                    variation_idx: usize|
         -> std::io::Result<()> {
            let bytes = cache.bnsh_export(whole_data, variation_idx)?;
            fs::write(path, bytes)
        };

        if let Some(shader_info) = &ptcl.shader_info {
            if emitter.data.shader_references.shader_index >= 0 {
                if let Some(binary) = &shader_info.binary_data {
                    let idx = emitter.data.shader_references.shader_index as usize;
                    write_variation_bnsh(cache, &emitter_dir.join("Shader.bnsh"), binary, idx)?;
                } else {
                    let idx = emitter.data.shader_references.shader_index as usize;
                    if idx < shader_info.variations.len() {
                        if let Some(binary_data) = &shader_info.variations[idx].binary_data {
                            write_variation_bnsh(
                                cache,
                                &emitter_dir.join("Shader.bnsh"),
                                binary_data,
                                0,
                            )?;
                        }
                    }
                }
            }

            if emitter.data.shader_references.compute_shader_index >= 0 {
                if let Some(binary) = &shader_info.compute_binary {
                    let idx = emitter.data.shader_references.compute_shader_index as usize;
                    write_variation_bnsh(
                        cache,
                        &emitter_dir.join("ComputeShader.bnsh"),
                        binary,
                        idx,
                    )?;
                } else {
                    let idx = emitter.data.shader_references.compute_shader_index as usize;
                    if idx < shader_info.variations.len() {
                        if let Some(binary_data) = &shader_info.variations[idx].binary_data {
                            write_variation_bnsh(
                                cache,
                                &emitter_dir.join("ComputeShader.bnsh"),
                                binary_data,
                                0,
                            )?;
                        }
                    }
                }
            }

            if emitter.data.shader_references.user_shader_index1 >= 0 {
                if let Some(binary) = &shader_info.binary_data {
                    let idx = emitter.data.shader_references.user_shader_index1 as usize;
                    write_variation_bnsh(
                        cache,
                        &emitter_dir.join("UserShader1.bnsh"),
                        binary,
                        idx,
                    )?;
                } else {
                    let idx = emitter.data.shader_references.user_shader_index1 as usize;
                    if idx < shader_info.variations.len() {
                        if let Some(binary_data) = &shader_info.variations[idx].binary_data {
                            write_variation_bnsh(
                                cache,
                                &emitter_dir.join("UserShader1.bnsh"),
                                binary_data,
                                0,
                            )?;
                        }
                    }
                }
            }

            if emitter.data.shader_references.user_shader_index2 >= 0 {
                if let Some(binary) = &shader_info.binary_data {
                    let idx = emitter.data.shader_references.user_shader_index2 as usize;
                    write_variation_bnsh(
                        cache,
                        &emitter_dir.join("UserShader2.bnsh"),
                        binary,
                        idx,
                    )?;
                } else {
                    let idx = emitter.data.shader_references.user_shader_index2 as usize;
                    if idx < shader_info.variations.len() {
                        if let Some(binary_data) = &shader_info.variations[idx].binary_data {
                            write_variation_bnsh(
                                cache,
                                &emitter_dir.join("UserShader2.bnsh"),
                                binary_data,
                                0,
                            )?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn dump_emitter_primitives<'a>(
        emitter: &crate::structs::Emitter,
        emitter_dir: &Path,
        ptcl: &'a PtclFile,
        cache: &mut ResourceCache,
        bfres_session: &mut Option<crate::bfres::load::ResExportSession<'a>>,
    ) -> std::io::Result<()> {
        let Some(primitive_info) = &ptcl.primitive_info else {
            return Ok(());
        };

        for prim_id in [
            emitter.data.particle_data.primitive_id,
            emitter.data.particle_data.primitive_ex_id,
            emitter.data.shape_info.primitive_index,
        ] {
            if prim_id == 0 || prim_id == u64::MAX {
                continue;
            }
            if let Some(model_data) = cache.bfres_export(bfres_session, primitive_info, prim_id) {
                fs::write(emitter_dir.join(format!("{prim_id}.bfres")), model_data)?;
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn file_extension_for_shader(binary_data: &[u8]) -> &'static str {
        if binary_data.len() >= 4 {
            match &binary_data[0..4] {
                b"BNSH" => "bnsh",
                b"FSHA" => "bfsha",
                _ => "bin",
            }
        } else {
            "bin"
        }
    }

    /// Group emitters by some heuristic (name patterns, order, etc.)
    #[allow(dead_code)]
    fn group_emitters(emitters: &[EmitterData]) -> BTreeMap<String, Vec<usize>> {
        let mut groups = BTreeMap::new();

        // For now, create a single group with all emitters
        // In a real implementation, we'd parse emitter names to group them
        let indices: Vec<usize> = (0..emitters.len()).collect();
        groups.insert("Emitters".to_string(), indices);

        groups
    }

    #[allow(dead_code)]
    fn group_emitters_by_namco(
        emitters: &[EmitterData],
        namco: &NamcoEffectFile,
    ) -> BTreeMap<String, Vec<usize>> {
        let mut groups = BTreeMap::new();

        // Create groups based on NAMCO entries and emitter names
        // Try to match emitters to emitter sets using name patterns or other heuristics

        // For now, use a simple approach: group by emitter name prefix or pattern
        let _emitter_by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();

        for _idx in 0..emitters.len() {
            // Try to find the emitter set this belongs to by parsing the name
            // For now, just create groups as we find them
        }

        // Now map back to emitter set names from NAMCO
        // For simplicity, we'll group all emitters under the entry names
        for entry_name in namco.entry_names.iter() {
            let set_name = entry_name_to_emitter_set_name(entry_name);

            // For now, just assign emitters in order
            // This is a simplified approach
            groups.entry(set_name).or_insert_with(Vec::new);
        }

        // If we couldn't figure out the grouping, just put all emitters in the first group
        if groups.is_empty() {
            let indices: Vec<usize> = (0..emitters.len()).collect();
            if let Some(first_name) = namco.entry_names.first() {
                let set_name = entry_name_to_emitter_set_name(first_name);
                groups.insert(set_name, indices);
            }
        }

        groups
    }
}

#[cfg(test)]
mod tests {
    use super::entry_name_to_emitter_set_name;

    #[test]
    fn test_entry_name_to_emitter_set_name_special_cases() {
        let cases = [
            ("KIRBY_ATK100", "P_KirbyAttack100"),
            ("KIRBY_FCUT", "P_KirbyFinalcutter"),
            ("KIRBY_FCUT_ARC", "P_KirbyFinalcutterArc"),
            ("KIRBY_FCUT_RISE", "P_KirbyFinalcutterRise"),
            ("KIRBY_STONE_S", "P_KirbyStoneStart"),
            ("KIRBY_STONE_S_GROUND", "P_KirbyStoneStartGround"),
            ("KIRBY_STONE_E", "P_KirbyStoneEnd"),
            ("FINKIRBY_HIT_CUT_L", "P_KirbyFinalHitCutL"),
        ];

        for (input, expected) in cases {
            assert_eq!(entry_name_to_emitter_set_name(input), expected);
        }
    }

    #[test]
    fn test_compute_single_entry_dict_ref_bit() {
        use crate::bntx::compute_single_entry_dict_ref_bit;

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
