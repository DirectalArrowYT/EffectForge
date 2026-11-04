use crate::enums::{ColorType, WrapMode};
use crate::reader::ReaderExt;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Read;

mod emitter_write;

fn serialize_base64<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn deserialize_base64<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(serde::de::Error::custom)
}

/// Check if a field should be read for the given VFX version.
/// Mirrors the C# [VersionCheck] attribute behavior.
pub fn version_check(check: Option<(VersionCompare, u16)>, version: u16) -> bool {
    match check {
        None => true,
        Some((cmp, threshold)) => match cmp {
            VersionCompare::Less => version < threshold,
            VersionCompare::Greater => version > threshold,
            VersionCompare::LessOrEqual => version <= threshold,
            VersionCompare::GreaterOrEqual => version >= threshold,
            VersionCompare::Equals => version == threshold,
        },
    }
}

#[allow(dead_code)]
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[allow(dead_code)]
fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Copy)]
pub enum VersionCompare {
    Less,
    Greater,
    LessOrEqual,
    GreaterOrEqual,
    Equals,
}

// ─── TextureSampler ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TextureSampler {
    #[serde(rename = "TextureID")]
    pub texture_id: u64,
    #[serde(rename = "WrapU")]
    pub wrap_u: WrapMode,
    #[serde(rename = "WrapV")]
    pub wrap_v: WrapMode,
    pub filter: u8,
    #[serde(rename = "IsSphereMap")]
    pub is_sphere_map: u8,
    #[serde(rename = "MaxLOD")]
    pub max_lod: f32,
    #[serde(rename = "LODBias")]
    pub lod_bias: f32,
    pub mip_level_limit: u8,
    #[serde(rename = "IsDensityFixedU")]
    pub is_density_fixed_u: u8,
    #[serde(rename = "IsDensityFixedV")]
    pub is_density_fixed_v: u8,
    #[serde(rename = "IsSquareRgb")]
    pub is_square_rgb: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "IsOnAnotherBinary")]
    pub is_on_another_binary: Option<u8>,
    #[serde(rename = "padding1")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding1: Option<u8>,
    #[serde(rename = "padding2")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding2: Option<u8>,
    #[serde(rename = "padding3")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding3: Option<u8>,
    #[serde(rename = "padding4")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding4: Option<u32>,
}

impl TextureSampler {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let texture_id = reader.read_u64_le()?;
        let wrap_u = WrapMode::from_u8(reader.read_u8()?);
        let wrap_v = WrapMode::from_u8(reader.read_u8()?);
        let filter = reader.read_u8()?;
        let is_sphere_map = reader.read_u8()?;
        let max_lod = reader.read_f32_le()?;
        let lod_bias = reader.read_f32_le()?;
        let mip_level_limit = reader.read_u8()?;
        let is_density_fixed_u = reader.read_u8()?;
        let is_density_fixed_v = reader.read_u8()?;
        let is_square_rgb = reader.read_u8()?;

        let (is_on_another_binary, padding1, padding2, padding3, padding4) =
            if version_check(Some((VersionCompare::Less, 50)), version) {
                (
                    Some(reader.read_u8()?),
                    Some(reader.read_u8()?),
                    Some(reader.read_u8()?),
                    Some(reader.read_u8()?),
                    Some(reader.read_u32_le()?),
                )
            } else {
                (None, None, None, None, None)
            };

        Ok(TextureSampler {
            texture_id,
            wrap_u,
            wrap_v,
            filter,
            is_sphere_map,
            max_lod,
            lod_bias,
            mip_level_limit,
            is_density_fixed_u,
            is_density_fixed_v,
            is_square_rgb,
            is_on_another_binary,
            padding1,
            padding2,
            padding3,
            padding4,
        })
    }
}

// ─── TextureAnim ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TextureAnim {
    pub pattern_anim_type: u8,
    pub is_scroll: bool,
    pub is_rotate: bool,
    pub is_scale: bool,
    pub repeat: u8,
    pub inv_rand_u: u8,
    pub inv_rand_v: u8,
    pub is_pat_anim_loop_random: u8,
    pub uv_channel: u8,
    pub is_crossfade: u8,
    #[serde(rename = "padding1")]
    pub padding1: u8,
    #[serde(rename = "padding2")]
    pub padding2: u8,
    #[serde(rename = "padding3")]
    pub padding3: u32,
}

impl TextureAnim {
    pub fn read<R: Read>(reader: &mut R, _version: u16) -> std::io::Result<Self> {
        Ok(TextureAnim {
            pattern_anim_type: reader.read_u8()?,
            is_scroll: reader.read_u8()? != 0,
            is_rotate: reader.read_u8()? != 0,
            is_scale: reader.read_u8()? != 0,
            repeat: reader.read_u8()?,
            inv_rand_u: reader.read_u8()?,
            inv_rand_v: reader.read_u8()?,
            is_pat_anim_loop_random: reader.read_u8()?,
            uv_channel: reader.read_u8()?,
            is_crossfade: reader.read_u8()?,
            padding1: reader.read_u8()?,
            padding2: reader.read_u8()?,
            padding3: reader.read_u32_le()?,
        })
    }
}

// ─── AnimationKey / AnimationKeyTable ──────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AnimationKey {
    #[serde(rename = "X")]
    pub x: f32,
    #[serde(rename = "Y")]
    pub y: f32,
    #[serde(rename = "Z")]
    pub z: f32,
    #[serde(rename = "Time")]
    pub time: f32,
}

impl AnimationKey {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(AnimationKey {
            x: reader.read_f32_le()?,
            y: reader.read_f32_le()?,
            z: reader.read_f32_le()?,
            time: reader.read_f32_le()?,
        })
    }

    const fn default_const() -> Self {
        AnimationKey {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            time: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AnimationKeyTable {
    pub keys: [AnimationKey; 8],
}

impl AnimationKeyTable {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut keys = [AnimationKey::default_const(); 8];
        for key in &mut keys {
            *key = AnimationKey::read(reader)?;
        }
        Ok(AnimationKeyTable { keys })
    }
}

// ─── TexPatAnim ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TexPatAnim {
    pub num: f32,
    pub frequency: f32,
    pub num_random: f32,
    pub pad: f32,
    pub table: [i32; 32],
}

impl TexPatAnim {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let num = reader.read_f32_le()?;
        let frequency = reader.read_f32_le()?;
        let num_random = reader.read_f32_le()?;
        let pad = reader.read_f32_le()?;
        let mut table = [0i32; 32];
        for val in &mut table {
            *val = reader.read_i32_le()?;
        }
        Ok(TexPatAnim {
            num,
            frequency,
            num_random,
            pad,
            table,
        })
    }
}

// ─── TexScrollAnim ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TexScrollAnim {
    pub scroll_add_x: f32,
    pub scroll_add_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub scroll_random_x: f32,
    pub scroll_random_y: f32,
    pub scale_add_x: f32,
    pub scale_add_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub scale_random_x: f32,
    pub scale_random_y: f32,
    pub rotation_add: f32,
    pub rotation: f32,
    pub rotation_random: f32,
    pub rotation_type: f32,
    #[serde(rename = "UVScaleX")]
    pub uv_scale_x: f32,
    #[serde(rename = "UVScaleY")]
    pub uv_scale_y: f32,
    #[serde(rename = "UVDivX")]
    pub uv_div_x: f32,
    #[serde(rename = "UVDivY")]
    pub uv_div_y: f32,
}

impl TexScrollAnim {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(TexScrollAnim {
            scroll_add_x: reader.read_f32_le()?,
            scroll_add_y: reader.read_f32_le()?,
            scroll_x: reader.read_f32_le()?,
            scroll_y: reader.read_f32_le()?,
            scroll_random_x: reader.read_f32_le()?,
            scroll_random_y: reader.read_f32_le()?,
            scale_add_x: reader.read_f32_le()?,
            scale_add_y: reader.read_f32_le()?,
            scale_x: reader.read_f32_le()?,
            scale_y: reader.read_f32_le()?,
            scale_random_x: reader.read_f32_le()?,
            scale_random_y: reader.read_f32_le()?,
            rotation_add: reader.read_f32_le()?,
            rotation: reader.read_f32_le()?,
            rotation_random: reader.read_f32_le()?,
            rotation_type: reader.read_f32_le()?,
            uv_scale_x: reader.read_f32_le()?,
            uv_scale_y: reader.read_f32_le()?,
            uv_div_x: reader.read_f32_le()?,
            uv_div_y: reader.read_f32_le()?,
        })
    }
}

// ─── EmitterStatic ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterStatic {
    pub flags1: u32,
    pub flags2: u32,
    pub flags3: u32,
    pub flags4: u32,
    pub num_color0_keys: u32,
    pub num_alpha0_keys: u32,
    pub num_color1_keys: u32,
    pub num_alpha1_keys: u32,
    pub num_scale_keys: u32,
    pub num_param_keys: u32,
    pub unknown1: u32,
    pub unknown2: u32,
    pub num_anim2_keys: u32,
    pub num_anim3_keys: u32,
    pub num_anim4_keys: u32,
    pub num_anim5_keys: u32,
    pub color0_loop_rate: f32,
    pub alpha0_loop_rate: f32,
    pub color1_loop_rate: f32,
    pub alpha1_loop_rate: f32,
    pub scale_loop_rate: f32,
    pub color0_loop_random: f32,
    pub alpha0_loop_random: f32,
    pub color1_loop_random: f32,
    pub alpha1_loop_random: f32,
    pub scale_loop_random: f32,
    pub unknown3: f32,
    pub unknown4: f32,
    #[serde(rename = "GravityDirX")]
    pub gravity_dir_x: f32,
    #[serde(rename = "GravityDirY")]
    pub gravity_dir_y: f32,
    #[serde(rename = "GravityDirZ")]
    pub gravity_dir_z: f32,
    #[serde(rename = "GravityScale")]
    pub gravity_scale: f32,
    pub air_res: f32,
    #[serde(rename = "val_0x74")]
    pub val_0x74: f32,
    #[serde(rename = "val_0x78")]
    pub val_0x78: f32,
    #[serde(rename = "val_0x82")]
    pub val_0x82: f32,
    #[serde(rename = "CenterX")]
    pub center_x: f32,
    #[serde(rename = "CenterY")]
    pub center_y: f32,
    pub offset: f32,
    #[serde(rename = "Padding")]
    pub padding: f32,
    #[serde(rename = "AmplitudeX")]
    pub amplitude_x: f32,
    #[serde(rename = "AmplitudeY")]
    pub amplitude_y: f32,
    #[serde(rename = "CycleX")]
    pub cycle_x: f32,
    #[serde(rename = "CycleY")]
    pub cycle_y: f32,
    #[serde(rename = "PhaseRndX")]
    pub phase_rnd_x: f32,
    #[serde(rename = "PhaseRndY")]
    pub phase_rnd_y: f32,
    #[serde(rename = "PhaseInitX")]
    pub phase_init_x: f32,
    #[serde(rename = "PhaseInitY")]
    pub phase_init_y: f32,
    #[serde(rename = "Coefficient0")]
    pub coefficient0: f32,
    #[serde(rename = "Coefficient1")]
    pub coefficient1: f32,
    #[serde(rename = "val_0xB8")]
    pub val_0xb8: f32,
    #[serde(rename = "val_0xBC")]
    pub val_0xbc: f32,
    #[serde(rename = "TexPatternAnim0")]
    pub tex_pattern_anim0: TexPatAnim,
    #[serde(rename = "TexPatternAnim1")]
    pub tex_pattern_anim1: TexPatAnim,
    #[serde(rename = "TexPatternAnim2")]
    pub tex_pattern_anim2: TexPatAnim,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "TexPatternAnim3")]
    pub tex_pattern_anim3: Option<TexPatAnim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "TexPatternAnim4")]
    pub tex_pattern_anim4: Option<TexPatAnim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "TexPatternAnim5")]
    pub tex_pattern_anim5: Option<TexPatAnim>,
    #[serde(rename = "TexScrollAnim0")]
    pub tex_scroll_anim0: TexScrollAnim,
    #[serde(rename = "TexScrollAnim1")]
    pub tex_scroll_anim1: TexScrollAnim,
    #[serde(rename = "TexScrollAnim2")]
    pub tex_scroll_anim2: TexScrollAnim,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "TexScrollAnim3")]
    pub tex_scroll_anim3: Option<TexScrollAnim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "TexScrollAnim4")]
    pub tex_scroll_anim4: Option<TexScrollAnim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "TexScrollAnim5")]
    pub tex_scroll_anim5: Option<TexScrollAnim>,
    #[serde(rename = "ColorScale")]
    pub color_scale: f32,
    #[serde(rename = "val_0x364")]
    pub val_0x364: f32,
    #[serde(rename = "val_0x368")]
    pub val_0x368: f32,
    #[serde(rename = "val_0x36A")]
    pub val_0x36a: f32,
    #[serde(rename = "Color0")]
    pub color0: AnimationKeyTable,
    #[serde(rename = "Alpha0")]
    pub alpha0: AnimationKeyTable,
    #[serde(rename = "Color1")]
    pub color1: AnimationKeyTable,
    #[serde(rename = "Alpha1")]
    pub alpha1: AnimationKeyTable,
    #[serde(rename = "SoftEdgeParam1")]
    pub soft_edge_param1: f32,
    #[serde(rename = "SoftEdgeParam2")]
    pub soft_edge_param2: f32,
    #[serde(rename = "FresnelAlphaParam1")]
    pub fresnel_alpha_param1: f32,
    #[serde(rename = "FresnelAlphaParam2")]
    pub fresnel_alpha_param2: f32,
    #[serde(rename = "NearDistAlphaParam1")]
    pub near_dist_alpha_param1: f32,
    #[serde(rename = "NearDistAlphaParam2")]
    pub near_dist_alpha_param2: f32,
    #[serde(rename = "FarDistAlphaParam1")]
    pub far_dist_alpha_param1: f32,
    #[serde(rename = "FarDistAlphaParam2")]
    pub far_dist_alpha_param2: f32,
    #[serde(rename = "DecalParam1")]
    pub decal_param1: f32,
    #[serde(rename = "DecalParam2")]
    pub decal_param2: f32,
    #[serde(rename = "AlphaThreshold")]
    pub alpha_threshold: f32,
    #[serde(rename = "Padding2")]
    pub padding2: f32,
    #[serde(rename = "AddVelToScale")]
    pub add_vel_to_scale: f32,
    #[serde(rename = "SoftPartcileDist")]
    pub soft_partcile_dist: f32,
    #[serde(rename = "SoftParticleVolume")]
    pub soft_particle_volume: f32,
    #[serde(rename = "Padding3")]
    pub padding3: f32,
    #[serde(rename = "ScaleAnim")]
    pub scale_anim: AnimationKeyTable,
    #[serde(rename = "ParamAnim")]
    pub param_anim: AnimationKeyTable,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "Anim1Keys")]
    pub anim1_keys: Option<AnimationKeyTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "Anim2Keys")]
    pub anim2_keys: Option<AnimationKeyTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "Anim3Keys")]
    pub anim3_keys: Option<AnimationKeyTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "Anim4Keys")]
    pub anim4_keys: Option<AnimationKeyTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "Unknown6")]
    pub unknown6: Option<[f32; 16]>,
    #[serde(rename = "RotateInitX")]
    pub rotate_init_x: f32,
    #[serde(rename = "RotateInitY")]
    pub rotate_init_y: f32,
    #[serde(rename = "RotateInitZ")]
    pub rotate_init_z: f32,
    #[serde(rename = "RotateInitEmpty")]
    pub rotate_init_empty: f32,
    #[serde(rename = "RotateInitRandX")]
    pub rotate_init_rand_x: f32,
    #[serde(rename = "RotateInitRandY")]
    pub rotate_init_rand_y: f32,
    #[serde(rename = "RotateInitRandZ")]
    pub rotate_init_rand_z: f32,
    #[serde(rename = "RotateInitRandEmpty")]
    pub rotate_init_rand_empty: f32,
    #[serde(rename = "RotateAddX")]
    pub rotate_add_x: f32,
    #[serde(rename = "RotateAddY")]
    pub rotate_add_y: f32,
    #[serde(rename = "RotateAddZ")]
    pub rotate_add_z: f32,
    #[serde(rename = "RotateRegist")]
    pub rotate_regist: f32,
    #[serde(rename = "RotateAddRandX")]
    pub rotate_add_rand_x: f32,
    #[serde(rename = "RotateAddRandY")]
    pub rotate_add_rand_y: f32,
    #[serde(rename = "RotateAddRandZ")]
    pub rotate_add_rand_z: f32,
    #[serde(rename = "Padding4")]
    pub padding4: f32,
    #[serde(rename = "ScaleLimitDistNear")]
    pub scale_limit_dist_near: f32,
    #[serde(rename = "ScaleLimitDistFar")]
    pub scale_limit_dist_far: f32,
    #[serde(rename = "Padding5")]
    pub padding5: f32,
    #[serde(rename = "Padding6")]
    pub padding6: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "Unknown7")]
    pub unknown7: Option<[f32; 16]>,
}

impl EmitterStatic {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_ge_50 = |v: u16| version_check(Some((VersionCompare::GreaterOrEqual, 50)), v);
        let has_gt_40 = |v: u16| version_check(Some((VersionCompare::Greater, 40)), v);

        Ok(EmitterStatic {
            flags1: reader.read_u32_le()?,
            flags2: reader.read_u32_le()?,
            flags3: reader.read_u32_le()?,
            flags4: reader.read_u32_le()?,
            num_color0_keys: reader.read_u32_le()?,
            num_alpha0_keys: reader.read_u32_le()?,
            num_color1_keys: reader.read_u32_le()?,
            num_alpha1_keys: reader.read_u32_le()?,
            num_scale_keys: reader.read_u32_le()?,
            num_param_keys: reader.read_u32_le()?,
            unknown1: reader.read_u32_le()?,
            unknown2: reader.read_u32_le()?,
            num_anim2_keys: if has_ge_50(version) {
                reader.read_u32_le()?
            } else {
                0
            },
            num_anim3_keys: if has_ge_50(version) {
                reader.read_u32_le()?
            } else {
                0
            },
            num_anim4_keys: if has_ge_50(version) {
                reader.read_u32_le()?
            } else {
                0
            },
            num_anim5_keys: if has_ge_50(version) {
                reader.read_u32_le()?
            } else {
                0
            },
            color0_loop_rate: reader.read_f32_le()?,
            alpha0_loop_rate: reader.read_f32_le()?,
            color1_loop_rate: reader.read_f32_le()?,
            alpha1_loop_rate: reader.read_f32_le()?,
            scale_loop_rate: reader.read_f32_le()?,
            color0_loop_random: reader.read_f32_le()?,
            alpha0_loop_random: reader.read_f32_le()?,
            color1_loop_random: reader.read_f32_le()?,
            alpha1_loop_random: reader.read_f32_le()?,
            scale_loop_random: reader.read_f32_le()?,
            unknown3: reader.read_f32_le()?,
            unknown4: reader.read_f32_le()?,
            gravity_dir_x: reader.read_f32_le()?,
            gravity_dir_y: reader.read_f32_le()?,
            gravity_dir_z: reader.read_f32_le()?,
            gravity_scale: reader.read_f32_le()?,
            air_res: reader.read_f32_le()?,
            val_0x74: reader.read_f32_le()?,
            val_0x78: reader.read_f32_le()?,
            val_0x82: reader.read_f32_le()?,
            center_x: reader.read_f32_le()?,
            center_y: reader.read_f32_le()?,
            offset: reader.read_f32_le()?,
            padding: reader.read_f32_le()?,
            amplitude_x: reader.read_f32_le()?,
            amplitude_y: reader.read_f32_le()?,
            cycle_x: reader.read_f32_le()?,
            cycle_y: reader.read_f32_le()?,
            phase_rnd_x: reader.read_f32_le()?,
            phase_rnd_y: reader.read_f32_le()?,
            phase_init_x: reader.read_f32_le()?,
            phase_init_y: reader.read_f32_le()?,
            coefficient0: reader.read_f32_le()?,
            coefficient1: reader.read_f32_le()?,
            val_0xb8: reader.read_f32_le()?,
            val_0xbc: reader.read_f32_le()?,
            tex_pattern_anim0: TexPatAnim::read(reader)?,
            tex_pattern_anim1: TexPatAnim::read(reader)?,
            tex_pattern_anim2: TexPatAnim::read(reader)?,
            tex_pattern_anim3: has_gt_40(version)
                .then(|| TexPatAnim::read(reader))
                .transpose()?,
            tex_pattern_anim4: has_gt_40(version)
                .then(|| TexPatAnim::read(reader))
                .transpose()?,
            tex_pattern_anim5: has_gt_40(version)
                .then(|| TexPatAnim::read(reader))
                .transpose()?,
            tex_scroll_anim0: TexScrollAnim::read(reader)?,
            tex_scroll_anim1: TexScrollAnim::read(reader)?,
            tex_scroll_anim2: TexScrollAnim::read(reader)?,
            tex_scroll_anim3: has_gt_40(version)
                .then(|| TexScrollAnim::read(reader))
                .transpose()?,
            tex_scroll_anim4: has_gt_40(version)
                .then(|| TexScrollAnim::read(reader))
                .transpose()?,
            tex_scroll_anim5: has_gt_40(version)
                .then(|| TexScrollAnim::read(reader))
                .transpose()?,
            color_scale: reader.read_f32_le()?,
            val_0x364: reader.read_f32_le()?,
            val_0x368: reader.read_f32_le()?,
            val_0x36a: reader.read_f32_le()?,
            color0: AnimationKeyTable::read(reader)?,
            alpha0: AnimationKeyTable::read(reader)?,
            color1: AnimationKeyTable::read(reader)?,
            alpha1: AnimationKeyTable::read(reader)?,
            soft_edge_param1: reader.read_f32_le()?,
            soft_edge_param2: reader.read_f32_le()?,
            fresnel_alpha_param1: reader.read_f32_le()?,
            fresnel_alpha_param2: reader.read_f32_le()?,
            near_dist_alpha_param1: reader.read_f32_le()?,
            near_dist_alpha_param2: reader.read_f32_le()?,
            far_dist_alpha_param1: reader.read_f32_le()?,
            far_dist_alpha_param2: reader.read_f32_le()?,
            decal_param1: reader.read_f32_le()?,
            decal_param2: reader.read_f32_le()?,
            alpha_threshold: reader.read_f32_le()?,
            padding2: reader.read_f32_le()?,
            add_vel_to_scale: reader.read_f32_le()?,
            soft_partcile_dist: reader.read_f32_le()?,
            soft_particle_volume: reader.read_f32_le()?,
            padding3: reader.read_f32_le()?,
            scale_anim: AnimationKeyTable::read(reader)?,
            param_anim: AnimationKeyTable::read(reader)?,
            anim1_keys: has_ge_50(version)
                .then(|| AnimationKeyTable::read(reader))
                .transpose()?,
            anim2_keys: has_ge_50(version)
                .then(|| AnimationKeyTable::read(reader))
                .transpose()?,
            anim3_keys: has_ge_50(version)
                .then(|| AnimationKeyTable::read(reader))
                .transpose()?,
            anim4_keys: has_ge_50(version)
                .then(|| AnimationKeyTable::read(reader))
                .transpose()?,
            unknown6: if has_gt_40(version) {
                let mut arr = [0.0f32; 16];
                for v in &mut arr {
                    *v = reader.read_f32_le().unwrap_or(0.0);
                }
                Some(arr)
            } else {
                None
            },
            rotate_init_x: reader.read_f32_le()?,
            rotate_init_y: reader.read_f32_le()?,
            rotate_init_z: reader.read_f32_le()?,
            rotate_init_empty: reader.read_f32_le()?,
            rotate_init_rand_x: reader.read_f32_le()?,
            rotate_init_rand_y: reader.read_f32_le()?,
            rotate_init_rand_z: reader.read_f32_le()?,
            rotate_init_rand_empty: reader.read_f32_le()?,
            rotate_add_x: reader.read_f32_le()?,
            rotate_add_y: reader.read_f32_le()?,
            rotate_add_z: reader.read_f32_le()?,
            rotate_regist: reader.read_f32_le()?,
            rotate_add_rand_x: reader.read_f32_le()?,
            rotate_add_rand_y: reader.read_f32_le()?,
            rotate_add_rand_z: reader.read_f32_le()?,
            padding4: reader.read_f32_le()?,
            scale_limit_dist_near: reader.read_f32_le()?,
            scale_limit_dist_far: reader.read_f32_le()?,
            padding5: reader.read_f32_le()?,
            padding6: reader.read_f32_le()?,
            unknown7: if has_gt_40(version) {
                let mut arr = [0.0f32; 16];
                for v in &mut arr {
                    *v = reader.read_f32_le().unwrap_or(0.0);
                }
                Some(arr)
            } else {
                None
            },
        })
    }
}

// ─── EmitterInfo ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterInfo {
    #[serde(rename = "IsParticleDraw")]
    pub is_particle_draw: u8,
    #[serde(rename = "SortType")]
    pub sort_type: u8,
    #[serde(rename = "CalcType")]
    pub calc_type: u8,
    #[serde(rename = "FollowType")]
    pub follow_type: u8,
    #[serde(rename = "IsFadeEmit")]
    pub is_fade_emit: u8,
    #[serde(rename = "IsFadeAlphaFade")]
    pub is_fade_alpha_fade: u8,
    #[serde(rename = "IsScaleFade")]
    pub is_scale_fade: u8,
    #[serde(rename = "RandomSeedType")]
    pub random_seed_type: u8,
    #[serde(rename = "IsUpdateMatrixByEmit")]
    pub is_update_matrix_by_emit: u8,
    #[serde(rename = "TestAlways")]
    pub test_always: u8,
    #[serde(rename = "InterpolateEmissionAmount")]
    pub interpolate_emission_amount: u8,
    #[serde(rename = "IsAlphaFadeIn")]
    pub is_alpha_fade_in: u8,
    #[serde(rename = "IsScaleFadeIn")]
    pub is_scale_fade_in: u8,
    #[serde(rename = "padding1")]
    pub padding1: u8,
    #[serde(rename = "padding2")]
    pub padding2: u8,
    #[serde(rename = "padding3")]
    pub padding3: u8,
    #[serde(rename = "RandomSeed")]
    pub random_seed: u32,
    #[serde(rename = "DrawPath")]
    pub draw_path: u32,
    #[serde(rename = "AlphaFadeTime")]
    pub alpha_fade_time: i32,
    #[serde(rename = "FadeInTime")]
    pub fade_in_time: i32,
    #[serde(rename = "TransX")]
    pub trans_x: f32,
    #[serde(rename = "TransY")]
    pub trans_y: f32,
    #[serde(rename = "TransZ")]
    pub trans_z: f32,
    #[serde(rename = "TransRandX")]
    pub trans_rand_x: f32,
    #[serde(rename = "TransRandY")]
    pub trans_rand_y: f32,
    #[serde(rename = "TransRandZ")]
    pub trans_rand_z: f32,
    #[serde(rename = "RotateX")]
    pub rotate_x: f32,
    #[serde(rename = "RotateY")]
    pub rotate_y: f32,
    #[serde(rename = "RotateZ")]
    pub rotate_z: f32,
    #[serde(rename = "RotateRandX")]
    pub rotate_rand_x: f32,
    #[serde(rename = "RotateRandY")]
    pub rotate_rand_y: f32,
    #[serde(rename = "RotateRandZ")]
    pub rotate_rand_z: f32,
    #[serde(rename = "ScaleX")]
    pub scale_x: f32,
    #[serde(rename = "ScaleY")]
    pub scale_y: f32,
    #[serde(rename = "ScaleZ")]
    pub scale_z: f32,
    #[serde(rename = "Color0R")]
    pub color0_r: f32,
    #[serde(rename = "Color0G")]
    pub color0_g: f32,
    #[serde(rename = "Color0B")]
    pub color0_b: f32,
    #[serde(rename = "Color0A")]
    pub color0_a: f32,
    #[serde(rename = "Color1R")]
    pub color1_r: f32,
    #[serde(rename = "Color1G")]
    pub color1_g: f32,
    #[serde(rename = "Color1B")]
    pub color1_b: f32,
    #[serde(rename = "Color1A")]
    pub color1_a: f32,
    #[serde(rename = "EmissionRangeNear")]
    pub emission_range_near: f32,
    #[serde(rename = "EmissionRangeFar")]
    pub emission_range_far: f32,
    #[serde(rename = "EmissionRatioFar")]
    pub emission_ratio_far: f32,
}

impl EmitterInfo {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EmitterInfo {
            is_particle_draw: reader.read_u8()?,
            sort_type: reader.read_u8()?,
            calc_type: reader.read_u8()?,
            follow_type: reader.read_u8()?,
            is_fade_emit: reader.read_u8()?,
            is_fade_alpha_fade: reader.read_u8()?,
            is_scale_fade: reader.read_u8()?,
            random_seed_type: reader.read_u8()?,
            is_update_matrix_by_emit: reader.read_u8()?,
            test_always: reader.read_u8()?,
            interpolate_emission_amount: reader.read_u8()?,
            is_alpha_fade_in: reader.read_u8()?,
            is_scale_fade_in: reader.read_u8()?,
            padding1: reader.read_u8()?,
            padding2: reader.read_u8()?,
            padding3: reader.read_u8()?,
            random_seed: reader.read_u32_le()?,
            draw_path: reader.read_u32_le()?,
            alpha_fade_time: reader.read_i32_le()?,
            fade_in_time: reader.read_i32_le()?,
            trans_x: reader.read_f32_le()?,
            trans_y: reader.read_f32_le()?,
            trans_z: reader.read_f32_le()?,
            trans_rand_x: reader.read_f32_le()?,
            trans_rand_y: reader.read_f32_le()?,
            trans_rand_z: reader.read_f32_le()?,
            rotate_x: reader.read_f32_le()?,
            rotate_y: reader.read_f32_le()?,
            rotate_z: reader.read_f32_le()?,
            rotate_rand_x: reader.read_f32_le()?,
            rotate_rand_y: reader.read_f32_le()?,
            rotate_rand_z: reader.read_f32_le()?,
            scale_x: reader.read_f32_le()?,
            scale_y: reader.read_f32_le()?,
            scale_z: reader.read_f32_le()?,
            color0_r: reader.read_f32_le()?,
            color0_g: reader.read_f32_le()?,
            color0_b: reader.read_f32_le()?,
            color0_a: reader.read_f32_le()?,
            color1_r: reader.read_f32_le()?,
            color1_g: reader.read_f32_le()?,
            color1_b: reader.read_f32_le()?,
            color1_a: reader.read_f32_le()?,
            emission_range_near: reader.read_f32_le()?,
            emission_range_far: reader.read_f32_le()?,
            emission_ratio_far: reader.read_f32_le()?,
        })
    }
}

// ─── EmitterInheritance ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterInheritance {
    #[serde(rename = "Velocity")]
    pub velocity: u8,
    #[serde(rename = "Scale")]
    pub scale: u8,
    #[serde(rename = "Rotate")]
    pub rotate: u8,
    #[serde(rename = "ColorScale")]
    pub color_scale: u8,
    #[serde(rename = "Color0")]
    pub color0: u8,
    #[serde(rename = "Color1")]
    pub color1: u8,
    #[serde(rename = "Alpha0")]
    pub alpha0: u8,
    #[serde(rename = "Alpha1")]
    pub alpha1: u8,
    #[serde(rename = "DrawPath")]
    pub draw_path: u8,
    #[serde(rename = "PreDraw")]
    pub pre_draw: u8,
    #[serde(rename = "Alpha0EachFrame")]
    pub alpha0_each_frame: u8,
    #[serde(rename = "Alpha1EachFrame")]
    pub alpha1_each_frame: u8,
    #[serde(rename = "EnableEmitterParticle")]
    pub enable_emitter_particle: u8,
    #[serde(rename = "padding1")]
    pub padding1: u8,
    #[serde(rename = "padding2")]
    pub padding2: u8,
    #[serde(rename = "padding3")]
    pub padding3: u8,
    #[serde(rename = "UnknownV40")]
    pub unknown_v40: u64,
    #[serde(rename = "VelocityRate")]
    pub velocity_rate: f32,
    #[serde(rename = "ScaleRate")]
    pub scale_rate: f32,
}

impl EmitterInheritance {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_gt_40 = version_check(Some((VersionCompare::Greater, 40)), version);
        Ok(EmitterInheritance {
            velocity: reader.read_u8()?,
            scale: reader.read_u8()?,
            rotate: reader.read_u8()?,
            color_scale: reader.read_u8()?,
            color0: reader.read_u8()?,
            color1: reader.read_u8()?,
            alpha0: reader.read_u8()?,
            alpha1: reader.read_u8()?,
            draw_path: reader.read_u8()?,
            pre_draw: reader.read_u8()?,
            alpha0_each_frame: reader.read_u8()?,
            alpha1_each_frame: reader.read_u8()?,
            enable_emitter_particle: reader.read_u8()?,
            padding1: reader.read_u8()?,
            padding2: reader.read_u8()?,
            padding3: reader.read_u8()?,
            unknown_v40: if has_gt_40 {
                reader.read_u64_le()?
            } else {
                0u64
            },
            velocity_rate: reader.read_f32_le()?,
            scale_rate: reader.read_f32_le()?,
        })
    }
}

// ─── Emission ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Emission {
    #[serde(rename = "isOneTime")]
    pub is_one_time: bool,
    #[serde(rename = "IsWorldGravity")]
    pub is_world_gravity: bool,
    #[serde(rename = "IsEmitDistEnabled")]
    pub is_emit_dist_enabled: bool,
    #[serde(rename = "IsWorldOrientedVelocity")]
    pub is_world_oriented_velocity: bool,
    #[serde(rename = "Start")]
    pub start: u32,
    #[serde(rename = "Timing")]
    pub timing: u32,
    #[serde(rename = "Duration")]
    pub duration: u32,
    #[serde(rename = "Rate")]
    pub rate: f32,
    #[serde(rename = "RateRandom")]
    pub rate_random: f32,
    #[serde(rename = "Interval")]
    pub interval: i32,
    #[serde(rename = "IntervalRandom")]
    pub interval_random: f32,
    #[serde(rename = "PositionRandom")]
    pub position_random: f32,
    #[serde(rename = "GravityScale")]
    pub gravity_scale: f32,
    #[serde(rename = "GravityDirX")]
    pub gravity_dir_x: f32,
    #[serde(rename = "GravityDirY")]
    pub gravity_dir_y: f32,
    #[serde(rename = "GravityDirZ")]
    pub gravity_dir_z: f32,
    #[serde(rename = "EmitterDistUnit")]
    pub emitter_dist_unit: f32,
    #[serde(rename = "EmitterDistMin")]
    pub emitter_dist_min: f32,
    #[serde(rename = "EmitterDistMax")]
    pub emitter_dist_max: f32,
    #[serde(rename = "EmitterDistMarg")]
    pub emitter_dist_marg: f32,
    #[serde(rename = "EmitterDistParticlesMax")]
    pub emitter_dist_particles_max: i32,
}

impl Emission {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(Emission {
            is_one_time: reader.read_u8()? != 0,
            is_world_gravity: reader.read_u8()? != 0,
            is_emit_dist_enabled: reader.read_u8()? != 0,
            is_world_oriented_velocity: reader.read_u8()? != 0,
            start: reader.read_u32_le()?,
            timing: reader.read_u32_le()?,
            duration: reader.read_u32_le()?,
            rate: reader.read_f32_le()?,
            rate_random: reader.read_f32_le()?,
            interval: reader.read_i32_le()?,
            interval_random: reader.read_f32_le()?,
            position_random: reader.read_f32_le()?,
            gravity_scale: reader.read_f32_le()?,
            gravity_dir_x: reader.read_f32_le()?,
            gravity_dir_y: reader.read_f32_le()?,
            gravity_dir_z: reader.read_f32_le()?,
            emitter_dist_unit: reader.read_f32_le()?,
            emitter_dist_min: reader.read_f32_le()?,
            emitter_dist_max: reader.read_f32_le()?,
            emitter_dist_marg: reader.read_f32_le()?,
            emitter_dist_particles_max: reader.read_i32_le()?,
        })
    }
}

// ─── EmitterShapeInfo ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterShapeInfo {
    #[serde(rename = "VolumeType")]
    pub volume_type: u8,
    #[serde(rename = "SweepStartRandom")]
    pub sweep_start_random: u8,
    #[serde(rename = "ArcType")]
    pub arc_type: u8,
    #[serde(rename = "IsVolumeLatitudeEnabled")]
    pub is_volume_latitude_enabled: u8,
    #[serde(rename = "VolumeTblIndex")]
    pub volume_tbl_index: u8,
    #[serde(rename = "VolumeTblIndex64")]
    pub volume_tbl_index64: u8,
    #[serde(rename = "VolumeLatitudeDir")]
    pub volume_latitude_dir: u8,
    #[serde(rename = "IsGpuEmitter")]
    pub is_gpu_emitter: u8,
    #[serde(rename = "SweepLongitude")]
    pub sweep_longitude: f32,
    #[serde(rename = "SweepLatitude")]
    pub sweep_latitude: f32,
    #[serde(rename = "SweepStart")]
    pub sweep_start: f32,
    #[serde(rename = "VolumeSurfacePosRand")]
    pub volume_surface_pos_rand: f32,
    #[serde(rename = "CaliberRatio")]
    pub caliber_ratio: f32,
    #[serde(rename = "LineCenter")]
    pub line_center: f32,
    #[serde(rename = "LineLength")]
    pub line_length: f32,
    #[serde(rename = "VolumeRadiusX")]
    pub volume_radius_x: f32,
    #[serde(rename = "VolumeRadiusY")]
    pub volume_radius_y: f32,
    #[serde(rename = "VolumeRadiusZ")]
    pub volume_radius_z: f32,
    #[serde(rename = "VolumeFormScaleX")]
    pub volume_form_scale_x: f32,
    #[serde(rename = "VolumeFormScaleY")]
    pub volume_form_scale_y: f32,
    #[serde(rename = "VolumeFormScaleZ")]
    pub volume_form_scale_z: f32,
    #[serde(rename = "PrimEmitType")]
    pub prim_emit_type: i32,
    #[serde(rename = "PrimitiveIndex")]
    pub primitive_index: u64,
    #[serde(rename = "NumDivideCircle")]
    pub num_divide_circle: i32,
    #[serde(rename = "NumDivideCircleRandom")]
    pub num_divide_circle_random: i32,
    #[serde(rename = "NumDivideLine")]
    pub num_divide_line: i32,
    #[serde(rename = "NumDivideLineRandom")]
    pub num_divide_line_random: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "IsOnAnotherBinaryVolumePrimitive")]
    pub is_on_another_binary_volume_primitive: Option<u8>,
    #[serde(rename = "padding1")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding1: Option<u8>,
    #[serde(rename = "padding2")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding2: Option<u8>,
    #[serde(rename = "padding3")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding3: Option<u8>,
    #[serde(rename = "padding4")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding4: Option<u32>,
}

impl EmitterShapeInfo {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_lt_40 = version_check(Some((VersionCompare::Less, 40)), version);
        Ok(EmitterShapeInfo {
            volume_type: reader.read_u8()?,
            sweep_start_random: reader.read_u8()?,
            arc_type: reader.read_u8()?,
            is_volume_latitude_enabled: reader.read_u8()?,
            volume_tbl_index: reader.read_u8()?,
            volume_tbl_index64: reader.read_u8()?,
            volume_latitude_dir: reader.read_u8()?,
            is_gpu_emitter: reader.read_u8()?,
            sweep_longitude: reader.read_f32_le()?,
            sweep_latitude: reader.read_f32_le()?,
            sweep_start: reader.read_f32_le()?,
            volume_surface_pos_rand: reader.read_f32_le()?,
            caliber_ratio: reader.read_f32_le()?,
            line_center: reader.read_f32_le()?,
            line_length: reader.read_f32_le()?,
            volume_radius_x: reader.read_f32_le()?,
            volume_radius_y: reader.read_f32_le()?,
            volume_radius_z: reader.read_f32_le()?,
            volume_form_scale_x: reader.read_f32_le()?,
            volume_form_scale_y: reader.read_f32_le()?,
            volume_form_scale_z: reader.read_f32_le()?,
            prim_emit_type: reader.read_i32_le()?,
            primitive_index: reader.read_u64_le()?,
            num_divide_circle: reader.read_i32_le()?,
            num_divide_circle_random: reader.read_i32_le()?,
            num_divide_line: reader.read_i32_le()?,
            num_divide_line_random: reader.read_i32_le()?,
            is_on_another_binary_volume_primitive: has_lt_40
                .then(|| reader.read_u8())
                .transpose()?,
            padding1: has_lt_40.then(|| reader.read_u8()).transpose()?,
            padding2: has_lt_40.then(|| reader.read_u8()).transpose()?,
            padding3: has_lt_40.then(|| reader.read_u8()).transpose()?,
            padding4: has_lt_40.then(|| reader.read_u32_le()).transpose()?,
        })
    }
}

// ─── EmitterRenderState ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterRenderState {
    #[serde(rename = "IsBlendEnable")]
    pub is_blend_enable: bool,
    #[serde(rename = "IsDepthTest")]
    pub is_depth_test: bool,
    pub depth_func: u8,
    #[serde(rename = "IsDepthMask")]
    pub is_depth_mask: bool,
    pub is_alpha_test: bool,
    #[serde(rename = "AlphaFunc")]
    pub alpha_func: u8,
    #[serde(rename = "BlendType")]
    pub blend_type: u8,
    #[serde(rename = "DisplaySide")]
    pub display_side: u8,
    #[serde(rename = "AlphaThreshold")]
    pub alpha_threshold: f32,
    #[serde(rename = "padding")]
    pub padding: u32,
}

impl EmitterRenderState {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EmitterRenderState {
            is_blend_enable: reader.read_u8()? != 0,
            is_depth_test: reader.read_u8()? != 0,
            depth_func: reader.read_u8()?,
            is_depth_mask: reader.read_u8()? != 0,
            is_alpha_test: reader.read_u8()? != 0,
            alpha_func: reader.read_u8()?,
            blend_type: reader.read_u8()?,
            display_side: reader.read_u8()?,
            alpha_threshold: reader.read_f32_le()?,
            padding: reader.read_u32_le()?,
        })
    }
}

// ─── ParticleData ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParticleData {
    #[serde(rename = "InfiniteLife")]
    pub infinite_life: bool,
    #[serde(rename = "IsTriming")]
    pub is_triming: bool,
    #[serde(rename = "BillboardType")]
    pub billboard_type: u8,
    #[serde(rename = "RotType")]
    pub rot_type: u8,
    #[serde(rename = "OffsetType")]
    pub offset_type: u8,
    #[serde(rename = "RotRevRandX")]
    pub rot_rev_rand_x: bool,
    #[serde(rename = "RotRevRandY")]
    pub rot_rev_rand_y: bool,
    #[serde(rename = "RotRevRandZ")]
    pub rot_rev_rand_z: bool,
    #[serde(rename = "IsRotateX")]
    pub is_rotate_x: bool,
    #[serde(rename = "IsRotateY")]
    pub is_rotate_y: bool,
    #[serde(rename = "IsRotateZ")]
    pub is_rotate_z: u8,
    #[serde(rename = "PrimitiveScaleType")]
    pub primitive_scale_type: u8,
    #[serde(rename = "IsTextureCommonRandom")]
    pub is_texture_common_random: u8,
    #[serde(rename = "ConnectPtclScaleAndZOffset")]
    pub connect_ptcl_scale_and_z_offset: u8,
    #[serde(rename = "EnableAvoidZFighting")]
    pub enable_avoid_z_fighting: u8,
    #[serde(rename = "val_0xF")]
    pub val_0xf: u8,
    #[serde(rename = "Life")]
    pub life: i32,
    #[serde(rename = "LifeRandom")]
    pub life_random: i32,
    #[serde(rename = "MomentumRandom")]
    pub momentum_random: f32,
    #[serde(rename = "PrimitiveVertexInfoFlags")]
    pub primitive_vertex_info_flags: u32,
    #[serde(rename = "PrimitiveID")]
    pub primitive_id: u64,
    #[serde(rename = "PrimitiveExID")]
    pub primitive_ex_id: u64,
    #[serde(rename = "LoopColor0")]
    pub loop_color0: bool,
    #[serde(rename = "LoopAlpha0")]
    pub loop_alpha0: bool,
    #[serde(rename = "LoopColor1")]
    pub loop_color1: bool,
    #[serde(rename = "LoopAlpha1")]
    pub loop_alpha1: bool,
    #[serde(rename = "ScaleLoop")]
    pub scale_loop: bool,
    #[serde(rename = "LoopRandomColor0")]
    pub loop_random_color0: bool,
    #[serde(rename = "LoopRandomAlpha0")]
    pub loop_random_alpha0: bool,
    #[serde(rename = "LoopRandomColor1")]
    pub loop_random_color1: bool,
    #[serde(rename = "LoopRandomAlpha1")]
    pub loop_random_alpha1: bool,
    #[serde(rename = "ScaleLoopRandom")]
    pub scale_loop_random: bool,
    #[serde(rename = "PrimFlag1")]
    pub prim_flag1: u8,
    #[serde(rename = "PrimFlag2")]
    pub prim_flag2: u8,
    // Version < 50: int loop rates
    #[serde(rename = "Color0LoopRate")]
    pub color0_loop_rate: i32,
    #[serde(rename = "Alpha0LoopRate")]
    pub alpha0_loop_rate: i32,
    #[serde(rename = "Color1LoopRate")]
    pub color1_loop_rate: i32,
    #[serde(rename = "Alpha1LoopRate")]
    pub alpha1_loop_rate: i32,
    #[serde(rename = "ScaleLoopRate")]
    pub scale_loop_rate: i32,
    // Version >= 50: short loop rates
    #[serde(rename = "Color0LoopRate16")]
    pub color0_loop_rate16: i16,
    #[serde(rename = "Alpha0LoopRate16")]
    pub alpha0_loop_rate16: i16,
    #[serde(rename = "Color1LoopRate16")]
    pub color1_loop_rate16: i16,
    #[serde(rename = "Alpha1LoopRate16")]
    pub alpha1_loop_rate16: i16,
    #[serde(rename = "ScaleLoopRate16")]
    pub scale_loop_rate16: i16,
}

impl ParticleData {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_lt_50 = version_check(Some((VersionCompare::Less, 50)), version);
        let has_ge_50 = version_check(Some((VersionCompare::GreaterOrEqual, 50)), version);

        Ok(ParticleData {
            infinite_life: reader.read_u8()? != 0,
            is_triming: reader.read_u8()? != 0,
            billboard_type: reader.read_u8()?,
            rot_type: reader.read_u8()?,
            offset_type: reader.read_u8()?,
            rot_rev_rand_x: reader.read_u8()? != 0,
            rot_rev_rand_y: reader.read_u8()? != 0,
            rot_rev_rand_z: reader.read_u8()? != 0,
            is_rotate_x: reader.read_u8()? != 0,
            is_rotate_y: reader.read_u8()? != 0,
            is_rotate_z: reader.read_u8()?,
            primitive_scale_type: reader.read_u8()?,
            is_texture_common_random: reader.read_u8()?,
            connect_ptcl_scale_and_z_offset: reader.read_u8()?,
            enable_avoid_z_fighting: reader.read_u8()?,
            val_0xf: reader.read_u8()?,
            life: reader.read_i32_le()?,
            life_random: reader.read_i32_le()?,
            momentum_random: reader.read_f32_le()?,
            primitive_vertex_info_flags: reader.read_u32_le()?,
            primitive_id: reader.read_u64_le()?,
            primitive_ex_id: reader.read_u64_le()?,
            loop_color0: reader.read_u8()? != 0,
            loop_alpha0: reader.read_u8()? != 0,
            loop_color1: reader.read_u8()? != 0,
            loop_alpha1: reader.read_u8()? != 0,
            scale_loop: reader.read_u8()? != 0,
            loop_random_color0: reader.read_u8()? != 0,
            loop_random_alpha0: reader.read_u8()? != 0,
            loop_random_color1: reader.read_u8()? != 0,
            loop_random_alpha1: reader.read_u8()? != 0,
            scale_loop_random: reader.read_u8()? != 0,
            prim_flag1: reader.read_u8()?,
            prim_flag2: reader.read_u8()?,
            color0_loop_rate: if has_lt_50 { reader.read_i32_le()? } else { 0 },
            alpha0_loop_rate: if has_lt_50 { reader.read_i32_le()? } else { 0 },
            color1_loop_rate: if has_lt_50 { reader.read_i32_le()? } else { 0 },
            alpha1_loop_rate: if has_lt_50 { reader.read_i32_le()? } else { 0 },
            scale_loop_rate: if has_lt_50 { reader.read_i32_le()? } else { 0 },
            color0_loop_rate16: if has_ge_50 { reader.read_i16_le()? } else { 0 },
            alpha0_loop_rate16: if has_ge_50 { reader.read_i16_le()? } else { 0 },
            color1_loop_rate16: if has_ge_50 { reader.read_i16_le()? } else { 0 },
            alpha1_loop_rate16: if has_ge_50 { reader.read_i16_le()? } else { 0 },
            scale_loop_rate16: if has_ge_50 { reader.read_i16_le()? } else { 0 },
        })
    }
}

// ─── EmitterCombiner (version < 40) ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterCombiner {
    #[serde(rename = "ColorCombinerProcess")]
    pub color_combiner_process: u8,
    #[serde(rename = "AlphaCombinerProcess")]
    pub alpha_combiner_process: u8,
    #[serde(rename = "Texture1ColorBlend")]
    pub texture1_color_blend: u8,
    #[serde(rename = "Texture2ColorBlend")]
    pub texture2_color_blend: u8,
    #[serde(rename = "PrimitiveColorBlend")]
    pub primitive_color_blend: u8,
    #[serde(rename = "Texture1AlphaBlend")]
    pub texture1_alpha_blend: u8,
    #[serde(rename = "Texture2AlphaBlend")]
    pub texture2_alpha_blend: u8,
    #[serde(rename = "PrimitiveAlphaBlend")]
    pub primitive_alpha_blend: u8,
    #[serde(rename = "TexColor0InputType")]
    pub tex_color0_input_type: u8,
    #[serde(rename = "TexColor1InputType")]
    pub tex_color1_input_type: u8,
    #[serde(rename = "TexColor2InputType")]
    pub tex_color2_input_type: u8,
    #[serde(rename = "TexAlpha0InputType")]
    pub tex_alpha0_input_type: u8,
    #[serde(rename = "TexAlpha1InputType")]
    pub tex_alpha1_input_type: u8,
    #[serde(rename = "TexAlpha2InputType")]
    pub tex_alpha2_input_type: u8,
    #[serde(rename = "PrimitiveColorInputType")]
    pub primitive_color_input_type: u8,
    #[serde(rename = "PrimitiveAlphaInputType")]
    pub primitive_alpha_input_type: u8,
    #[serde(rename = "ShaderType")]
    pub shader_type: u8,
    #[serde(rename = "ApplyAlpha")]
    pub apply_alpha: u8,
    #[serde(rename = "IsDistortionByCameraDistance")]
    pub is_distortion_by_camera_distance: u8,
    #[serde(rename = "padding1")]
    pub padding1: u8,
    #[serde(rename = "padding2")]
    pub padding2: u32,
}

impl EmitterCombiner {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EmitterCombiner {
            color_combiner_process: reader.read_u8()?,
            alpha_combiner_process: reader.read_u8()?,
            texture1_color_blend: reader.read_u8()?,
            texture2_color_blend: reader.read_u8()?,
            primitive_color_blend: reader.read_u8()?,
            texture1_alpha_blend: reader.read_u8()?,
            texture2_alpha_blend: reader.read_u8()?,
            primitive_alpha_blend: reader.read_u8()?,
            tex_color0_input_type: reader.read_u8()?,
            tex_color1_input_type: reader.read_u8()?,
            tex_color2_input_type: reader.read_u8()?,
            tex_alpha0_input_type: reader.read_u8()?,
            tex_alpha1_input_type: reader.read_u8()?,
            tex_alpha2_input_type: reader.read_u8()?,
            primitive_color_input_type: reader.read_u8()?,
            primitive_alpha_input_type: reader.read_u8()?,
            shader_type: reader.read_u8()?,
            apply_alpha: reader.read_u8()?,
            is_distortion_by_camera_distance: reader.read_u8()?,
            padding1: reader.read_u8()?,
            padding2: reader.read_u32_le()?,
        })
    }
}

// ─── EmitterCombinerV36 (version == 36) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterCombinerV36 {
    #[serde(rename = "ColorCombinerProcess")]
    pub color_combiner_process: u8,
    #[serde(rename = "AlphaCombinerProcess")]
    pub alpha_combiner_process: u8,
    #[serde(rename = "Texture1ColorBlend")]
    pub texture1_color_blend: u8,
    #[serde(rename = "Texture2ColorBlend")]
    pub texture2_color_blend: u8,
    #[serde(rename = "PrimitiveColorBlend")]
    pub primitive_color_blend: u8,
    #[serde(rename = "Texture1AlphaBlend")]
    pub texture1_alpha_blend: u8,
    #[serde(rename = "Texture2AlphaBlend")]
    pub texture2_alpha_blend: u8,
    #[serde(rename = "PrimitiveAlphaBlend")]
    pub primitive_alpha_blend: u8,
}

impl EmitterCombinerV36 {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(EmitterCombinerV36 {
            color_combiner_process: reader.read_u8()?,
            alpha_combiner_process: reader.read_u8()?,
            texture1_color_blend: reader.read_u8()?,
            texture2_color_blend: reader.read_u8()?,
            primitive_color_blend: reader.read_u8()?,
            texture1_alpha_blend: reader.read_u8()?,
            texture2_alpha_blend: reader.read_u8()?,
            primitive_alpha_blend: reader.read_u8()?,
        })
    }
}

// ─── CombinedEmitterCombinerV40 (version > 40) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CombinedEmitterCombinerV40 {
    #[serde(rename = "ColorCombinerProcess")]
    pub color_combiner_process: u8,
    #[serde(rename = "AlphaCombinerProcess")]
    pub alpha_combiner_process: u8,
    #[serde(rename = "Texture1ColorBlend")]
    pub texture1_color_blend: u8,
    #[serde(rename = "Texture2ColorBlend")]
    pub texture2_color_blend: u8,
    #[serde(rename = "PrimitiveColorBlend")]
    pub primitive_color_blend: u8,
    #[serde(rename = "Texture1AlphaBlend")]
    pub texture1_alpha_blend: u8,
    #[serde(rename = "Texture2AlphaBlend")]
    pub texture2_alpha_blend: u8,
    #[serde(rename = "PrimitiveAlphaBlend")]
    pub primitive_alpha_blend: u8,
    #[serde(rename = "TexColor0InputType")]
    pub tex_color0_input_type: u8,
    #[serde(rename = "TexColor1InputType")]
    pub tex_color1_input_type: u8,
    #[serde(rename = "TexColor2InputType")]
    pub tex_color2_input_type: u8,
    #[serde(rename = "TexAlpha0InputType")]
    pub tex_alpha0_input_type: u8,
    #[serde(rename = "TexAlpha1InputType")]
    pub tex_alpha1_input_type: u8,
    #[serde(rename = "TexAlpha2InputType")]
    pub tex_alpha2_input_type: u8,
    #[serde(rename = "PrimitiveColorInputType")]
    pub primitive_color_input_type: u8,
    #[serde(rename = "PrimitiveAlphaInputType")]
    pub primitive_alpha_input_type: u8,
    #[serde(rename = "ShaderType")]
    pub shader_type: u8,
    #[serde(rename = "ApplyAlpha")]
    pub apply_alpha: u8,
    #[serde(rename = "IsDistortionByCameraDistance")]
    pub is_distortion_by_camera_distance: u8,
    #[serde(rename = "padding1")]
    pub padding1: u8,
    #[serde(rename = "padding2")]
    pub padding2: u8,
    // These trailing v50+ fields come from the C# `EmitterCombinerV40` class, which is separate
    // from `EmitterCombiner` and so carries its own capitalised `Padding*` names. Merging both
    // classes into this one struct means those names have to stay distinct from the lowercase
    // `padding1`/`padding2` above: sharing a name makes serde emit the same JSON key twice and
    // the dump then fails to reload with a duplicate-field error.
    #[serde(rename = "Padding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<i16>,
    #[serde(rename = "Padding2")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding2_opt: Option<u32>,
    #[serde(rename = "Padding3")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding3: Option<u32>,
}

impl CombinedEmitterCombinerV40 {
    pub fn read<R: Read>(
        reader: &mut R,
        version: u16,
        shader: &ShaderRefInfo,
    ) -> std::io::Result<Self> {
        let has_ge_50 = version_check(Some((VersionCompare::GreaterOrEqual, 50)), version);
        Ok(CombinedEmitterCombinerV40 {
            color_combiner_process: reader.read_u8()?,
            alpha_combiner_process: reader.read_u8()?,
            texture1_color_blend: reader.read_u8()?,
            texture2_color_blend: reader.read_u8()?,
            primitive_color_blend: reader.read_u8()?,
            texture1_alpha_blend: reader.read_u8()?,
            texture2_alpha_blend: reader.read_u8()?,
            primitive_alpha_blend: reader.read_u8()?,
            tex_color0_input_type: reader.read_u8()?,
            tex_color1_input_type: reader.read_u8()?,
            tex_color2_input_type: reader.read_u8()?,
            tex_alpha0_input_type: reader.read_u8()?,
            tex_alpha1_input_type: reader.read_u8()?,
            tex_alpha2_input_type: reader.read_u8()?,
            primitive_color_input_type: reader.read_u8()?,
            primitive_alpha_input_type: reader.read_u8()?,
            shader_type: shader.type_,
            apply_alpha: shader.val_0x2,
            is_distortion_by_camera_distance: shader.val_0x3,
            padding1: shader.val_0x4,
            padding2: 0, // assuming
            padding: has_ge_50.then(|| reader.read_i16_le()).transpose()?,
            padding2_opt: has_ge_50.then(|| reader.read_u32_le()).transpose()?,
            padding3: has_ge_50.then(|| reader.read_u32_le()).transpose()?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmitterCombinerVariant {
    Legacy(EmitterCombiner),
    V36(EmitterCombinerV36),
    V40(CombinedEmitterCombinerV40),
}

impl EmitterCombinerVariant {
    /// Rebuild a combiner from its dumped JSON, picking the variant from the vfx version.
    ///
    /// The derived `#[serde(untagged)]` `Deserialize` cannot do this on its own: every field
    /// `EmitterCombiner` declares also exists on `CombinedEmitterCombinerV40` under the same
    /// name, unknown fields are ignored, and untagged tries the variants in declaration order,
    /// so a v40 combiner always comes back as `Legacy` (or `V36` once the v50-only `Padding*`
    /// fields are present). `EmitterData::write` then matches the variant against the version
    /// and writes *nothing* when the two disagree, silently truncating the emitter. Selecting
    /// on the version uses the same predicates as `EmitterData::read`/`write`, so the parsed
    /// variant is the one the writer expects.
    ///
    /// Returns `Ok(None)` for versions that carry no combiner section (37..=40), leaving the
    /// caller free to keep whatever it already had.
    pub fn from_json_value(
        value: &serde_json::Value,
        version: u16,
    ) -> std::io::Result<Option<Self>> {
        fn parse<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> std::io::Result<T> {
            T::deserialize(value).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
            })
        }

        match Self::expected_kind(version) {
            Some(CombinerKind::Legacy) => Ok(Some(EmitterCombinerVariant::Legacy(parse(value)?))),
            Some(CombinerKind::V36) => Ok(Some(EmitterCombinerVariant::V36(parse(value)?))),
            Some(CombinerKind::V40) => Ok(Some(EmitterCombinerVariant::V40(parse(value)?))),
            None => Ok(None),
        }
    }

    /// Which variant this version's binary layout uses, by the same predicates as
    /// `EmitterData::read`/`write`. `None` for the 37..=40 gap, which carries no combiner.
    fn expected_kind(version: u16) -> Option<CombinerKind> {
        let has_lt_40 = version_check(Some((VersionCompare::Less, 40)), version);
        let has_eq_36 = version_check(Some((VersionCompare::Equals, 36)), version);
        let has_gt_40 = version_check(Some((VersionCompare::Greater, 40)), version);
        let has_ge_36 = version_check(Some((VersionCompare::GreaterOrEqual, 36)), version);

        if has_lt_40 && !has_ge_36 {
            Some(CombinerKind::Legacy)
        } else if has_eq_36 {
            Some(CombinerKind::V36)
        } else if has_gt_40 {
            Some(CombinerKind::V40)
        } else {
            None
        }
    }

    fn kind(&self) -> CombinerKind {
        match self {
            EmitterCombinerVariant::Legacy(_) => CombinerKind::Legacy,
            EmitterCombinerVariant::V36(_) => CombinerKind::V36,
            EmitterCombinerVariant::V40(_) => CombinerKind::V40,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombinerKind {
    Legacy,
    V36,
    V40,
}

// ─── ShaderRefInfo ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ShaderRefInfo {
    #[serde(rename = "Type")]
    pub type_: u8,
    #[serde(rename = "val_0x2")]
    pub val_0x2: u8,
    #[serde(rename = "val_0x3")]
    pub val_0x3: u8,
    #[serde(rename = "val_0x4")]
    pub val_0x4: u8,
    #[serde(rename = "ShaderIndex")]
    pub shader_index: i32,
    #[serde(rename = "ComputeShaderIndex")]
    pub compute_shader_index: i32,
    #[serde(rename = "UserShaderIndex1")]
    pub user_shader_index1: i32,
    #[serde(rename = "UserShaderIndex2")]
    pub user_shader_index2: i32,
    #[serde(rename = "CustomShaderIndex")]
    pub custom_shader_index: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "CustomShaderFlag")]
    pub custom_shader_flag: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "CustomShaderSwitch")]
    pub custom_shader_switch: Option<u64>,
    #[serde(rename = "Unknown1")]
    pub unknown1: u64,
    #[serde(rename = "ExtraShaderIndex2")]
    pub extra_shader_index2: i32,
    #[serde(rename = "val_0x34")]
    pub val_0x34: i32,
    #[serde(rename = "Unknown2")]
    pub unknown2: u64,
    #[serde(rename = "UserShaderDefine1")]
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub user_shader_define1: Vec<u8>,
    #[serde(rename = "UserShaderDefine2")]
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub user_shader_define2: Vec<u8>,
}

impl ShaderRefInfo {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_lt_50 = version_check(Some((VersionCompare::Less, 50)), version);
        let has_lt_22 = version_check(Some((VersionCompare::Less, 22)), version);
        let has_ge_50 = version_check(Some((VersionCompare::GreaterOrEqual, 50)), version);

        Ok(ShaderRefInfo {
            type_: reader.read_u8()?,
            val_0x2: reader.read_u8()?,
            val_0x3: reader.read_u8()?,
            val_0x4: reader.read_u8()?,
            shader_index: reader.read_i32_le()?,
            compute_shader_index: reader.read_i32_le()?,
            user_shader_index1: reader.read_i32_le()?,
            user_shader_index2: reader.read_i32_le()?,
            custom_shader_index: reader.read_i32_le()?,
            custom_shader_flag: has_lt_50.then(|| reader.read_u64_le()).transpose()?,
            custom_shader_switch: has_lt_50.then(|| reader.read_u64_le()).transpose()?,
            unknown1: if has_lt_22 { reader.read_u64_le()? } else { 0 },
            extra_shader_index2: reader.read_i32_le()?,
            val_0x34: reader.read_i32_le()?,
            unknown2: if has_ge_50 { reader.read_u64_le()? } else { 0 },
            user_shader_define1: reader.read_bytes(16)?,
            user_shader_define2: reader.read_bytes(16)?,
        })
    }
}

// ─── ActionInfo ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ActionInfo {
    #[serde(rename = "ActionIndex")]
    pub action_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown: Option<Vec<u32>>,
}

impl ActionInfo {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_gt_40 = version_check(Some((VersionCompare::Greater, 40)), version);
        Ok(ActionInfo {
            action_index: reader.read_u32_le()?,
            unknown: has_gt_40.then(|| {
                let mut arr = Vec::with_capacity(5);
                for _ in 0..5 {
                    arr.push(reader.read_u32_le().unwrap_or(0));
                }
                arr
            }),
        })
    }
}

// ─── ParticleVelocityInfo ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParticleVelocityInfo {
    #[serde(rename = "AllDirection")]
    pub all_direction: f32,
    #[serde(rename = "DesignatedDirScale")]
    pub designated_dir_scale: f32,
    #[serde(rename = "DesignatedDirX")]
    pub designated_dir_x: f32,
    #[serde(rename = "DesignatedDirY")]
    pub designated_dir_y: f32,
    #[serde(rename = "DesignatedDirZ")]
    pub designated_dir_z: f32,
    #[serde(rename = "DiffusionDirAngle")]
    pub diffusion_dir_angle: f32,
    #[serde(rename = "XZDiffusion")]
    pub xz_diffusion: f32,
    #[serde(rename = "DiffusionX")]
    pub diffusion_x: f32,
    #[serde(rename = "DiffusionY")]
    pub diffusion_y: f32,
    #[serde(rename = "DiffusionZ")]
    pub diffusion_z: f32,
    #[serde(rename = "VelRandom")]
    pub vel_random: f32,
    #[serde(rename = "EmVelInherit")]
    pub em_vel_inherit: f32,
}

impl ParticleVelocityInfo {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(ParticleVelocityInfo {
            all_direction: reader.read_f32_le()?,
            designated_dir_scale: reader.read_f32_le()?,
            designated_dir_x: reader.read_f32_le()?,
            designated_dir_y: reader.read_f32_le()?,
            designated_dir_z: reader.read_f32_le()?,
            diffusion_dir_angle: reader.read_f32_le()?,
            xz_diffusion: reader.read_f32_le()?,
            diffusion_x: reader.read_f32_le()?,
            diffusion_y: reader.read_f32_le()?,
            diffusion_z: reader.read_f32_le()?,
            vel_random: reader.read_f32_le()?,
            em_vel_inherit: reader.read_f32_le()?,
        })
    }
}

// ─── ParticleColor ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParticleColor {
    #[serde(rename = "IsSoftParticle")]
    pub is_soft_particle: u8,
    #[serde(rename = "IsFresnelAlpha")]
    pub is_fresnel_alpha: u8,
    #[serde(rename = "IsNearDistAlpha")]
    pub is_near_dist_alpha: u8,
    #[serde(rename = "IsFarDistAlpha")]
    pub is_far_dist_alpha: u8,
    #[serde(rename = "IsDecal")]
    pub is_decal: u8,
    #[serde(rename = "val_0x5")]
    pub val_0x5: u8,
    #[serde(rename = "val_0x6")]
    pub val_0x6: u8,
    #[serde(rename = "val_0x7")]
    pub val_0x7: u8,
    #[serde(rename = "Color0Type")]
    pub color0_type: ColorType,
    #[serde(rename = "Color1Type")]
    pub color1_type: ColorType,
    #[serde(rename = "Alpha0Type")]
    pub alpha0_type: ColorType,
    #[serde(rename = "Alpha1Type")]
    pub alpha1_type: ColorType,
    #[serde(rename = "Color0R")]
    pub color0_r: f32,
    #[serde(rename = "Color0G")]
    pub color0_g: f32,
    #[serde(rename = "Color0B")]
    pub color0_b: f32,
    #[serde(rename = "Alpha0")]
    pub alpha0: f32,
    #[serde(rename = "Color1R")]
    pub color1_r: f32,
    #[serde(rename = "Color1G")]
    pub color1_g: f32,
    #[serde(rename = "Color1B")]
    pub color1_b: f32,
    #[serde(rename = "Alpha1")]
    pub alpha1: f32,
}

impl ParticleColor {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(ParticleColor {
            is_soft_particle: reader.read_u8()?,
            is_fresnel_alpha: reader.read_u8()?,
            is_near_dist_alpha: reader.read_u8()?,
            is_far_dist_alpha: reader.read_u8()?,
            is_decal: reader.read_u8()?,
            val_0x5: reader.read_u8()?,
            val_0x6: reader.read_u8()?,
            val_0x7: reader.read_u8()?,
            color0_type: ColorType::from_u8(reader.read_u8()?),
            color1_type: ColorType::from_u8(reader.read_u8()?),
            alpha0_type: ColorType::from_u8(reader.read_u8()?),
            alpha1_type: ColorType::from_u8(reader.read_u8()?),
            color0_r: reader.read_f32_le()?,
            color0_g: reader.read_f32_le()?,
            color0_b: reader.read_f32_le()?,
            alpha0: reader.read_f32_le()?,
            color1_r: reader.read_f32_le()?,
            color1_g: reader.read_f32_le()?,
            color1_b: reader.read_f32_le()?,
            alpha1: reader.read_f32_le()?,
        })
    }
}

// ─── ParticleScale ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParticleScale {
    #[serde(rename = "ScaleX")]
    pub scale_x: f32,
    #[serde(rename = "ScaleY")]
    pub scale_y: f32,
    #[serde(rename = "ScaleZ")]
    pub scale_z: f32,
    #[serde(rename = "ScaleRandomX")]
    pub scale_random_x: f32,
    #[serde(rename = "ScaleRandomY")]
    pub scale_random_y: f32,
    #[serde(rename = "ScaleRandomZ")]
    pub scale_random_z: f32,
    #[serde(rename = "EnableScalingByCameraDistNear")]
    pub enable_scaling_by_camera_dist_near: u8,
    #[serde(rename = "EnableScalingByCameraDistFar")]
    pub enable_scaling_by_camera_dist_far: u8,
    #[serde(rename = "EnableAddScaleY")]
    pub enable_add_scale_y: u8,
    #[serde(rename = "EnableLinkFovyToScaleValue")]
    pub enable_link_fovy_to_scale_value: u8,
    #[serde(rename = "ScaleMin")]
    pub scale_min: f32,
    #[serde(rename = "ScaleMax")]
    pub scale_max: f32,
}

impl ParticleScale {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(ParticleScale {
            scale_x: reader.read_f32_le()?,
            scale_y: reader.read_f32_le()?,
            scale_z: reader.read_f32_le()?,
            scale_random_x: reader.read_f32_le()?,
            scale_random_y: reader.read_f32_le()?,
            scale_random_z: reader.read_f32_le()?,
            enable_scaling_by_camera_dist_near: reader.read_u8()?,
            enable_scaling_by_camera_dist_far: reader.read_u8()?,
            enable_add_scale_y: reader.read_u8()?,
            enable_link_fovy_to_scale_value: reader.read_u8()?,
            scale_min: reader.read_f32_le()?,
            scale_max: reader.read_f32_le()?,
        })
    }
}

// ─── ParticleFlucInfo ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ParticleFlucInfo {
    #[serde(rename = "IsApplyAlpha")]
    pub is_apply_alpha: u8,
    #[serde(rename = "IsApplayScale")]
    pub is_applay_scale: u8,
    #[serde(rename = "IsApplayScaleY")]
    pub is_applay_scale_y: u8,
    #[serde(rename = "IsWaveType")]
    pub is_wave_type: u8,
    #[serde(rename = "IsPhaseRandomX")]
    pub is_phase_random_x: u8,
    #[serde(rename = "IsPhaseRandomY")]
    pub is_phase_random_y: u8,
    #[serde(rename = "padding1")]
    pub padding1: u8,
    #[serde(rename = "padding2")]
    pub padding2: u8,
    #[serde(rename = "padding3")]
    pub padding3: u32,
}

impl ParticleFlucInfo {
    pub fn read<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        Ok(ParticleFlucInfo {
            is_apply_alpha: reader.read_u8()?,
            is_applay_scale: reader.read_u8()?,
            is_applay_scale_y: reader.read_u8()?,
            is_wave_type: reader.read_u8()?,
            is_phase_random_x: reader.read_u8()?,
            is_phase_random_y: reader.read_u8()?,
            padding1: reader.read_u8()?,
            padding2: reader.read_u8()?,
            padding3: reader.read_u32_le()?,
        })
    }
}

// ─── EmitterData (top-level) ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmitterData {
    #[serde(rename = "Flag")]
    pub flag: u32,
    #[serde(rename = "RandomSeed")]
    pub random_seed: u32,
    #[serde(rename = "Padding1")]
    pub padding1: u32,
    #[serde(rename = "Padding2")]
    pub padding2: u32,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "EmitterStatic")]
    pub emitter_static: EmitterStatic,
    #[serde(rename = "EmitterInfo")]
    pub emitter_info: EmitterInfo,
    #[serde(rename = "ChildInheritance")]
    pub child_inheritance: EmitterInheritance,
    #[serde(rename = "Emission")]
    pub emission: Emission,
    #[serde(rename = "ShapeInfo")]
    pub shape_info: EmitterShapeInfo,
    #[serde(rename = "RenderState")]
    pub render_state: EmitterRenderState,
    #[serde(rename = "ParticleData")]
    pub particle_data: ParticleData,
    #[serde(rename = "Combiner")]
    pub combiner: Option<EmitterCombinerVariant>,
    #[serde(rename = "ShaderReferences")]
    pub shader_references: ShaderRefInfo,
    #[serde(rename = "Action")]
    pub action: ActionInfo,
    #[serde(rename = "ParticleVelocity")]
    pub particle_velocity: ParticleVelocityInfo,
    #[serde(rename = "ParticleColor")]
    pub particle_color: ParticleColor,
    #[serde(rename = "ParticleScale")]
    pub particle_scale: ParticleScale,
    #[serde(rename = "ParticleFluctuation")]
    pub particle_fluctuation: Option<ParticleFlucInfo>,
    #[serde(rename = "Sampler0")]
    pub sampler0: Option<TextureSampler>,
    #[serde(rename = "Sampler1")]
    pub sampler1: Option<TextureSampler>,
    #[serde(rename = "Sampler2")]
    pub sampler2: Option<TextureSampler>,
    #[serde(rename = "TextureAnim0")]
    pub texture_anim0: Option<TextureAnim>,
    #[serde(rename = "TextureAnim1")]
    pub texture_anim1: Option<TextureAnim>,
    #[serde(rename = "TextureAnim2")]
    pub texture_anim2: Option<TextureAnim>,
    #[serde(rename = "Reserved")]
    #[serde(
        serialize_with = "serialize_base64",
        deserialize_with = "deserialize_base64"
    )]
    pub reserved: Vec<u8>,
    // Version-gated fields, each read from the binary but only present at certain versions.
    // C# declares all four and dumps with `NullValueHandling.Ignore`, so omitting them while
    // null is what matches the reference exporter — skipping them outright does not: it drops
    // them from the dump entirely, and the rebuild then writes empty strings and zero floats
    // over whatever the file actually held. `namev40` carries the emitter's *name* above v40,
    // where `name` is left None, so skipping it lost the name outright.
    #[serde(rename = "Namev40")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namev40: Option<String>,
    #[serde(rename = "DepthMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_mode: Option<String>,
    #[serde(rename = "PassInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass_info: Option<String>,
    #[serde(rename = "UnknownV36")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_v36: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampler3: Option<TextureSampler>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampler4: Option<TextureSampler>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampler5: Option<TextureSampler>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture_anim3: Option<TextureAnim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture_anim4: Option<TextureAnim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture_anim5: Option<TextureAnim>,
    #[serde(skip)]
    pub order: usize,
}

// ─── CombinerTemp ─────────────────────────────────────────────────

type CombinerV40Fields = (
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    u8,
    Option<i16>,
    Option<u32>,
    Option<u32>,
);

enum CombinerTemp {
    Legacy(EmitterCombiner),
    V36(EmitterCombinerV36),
    V40(CombinerV40Fields),
}

impl EmitterData {
    pub fn read<R: Read>(reader: &mut R, version: u16) -> std::io::Result<Self> {
        let has_lt_40 = version_check(Some((VersionCompare::Less, 40)), version);
        let has_eq_36 = version_check(Some((VersionCompare::Equals, 36)), version);
        let has_gt_40 = version_check(Some((VersionCompare::Greater, 40)), version);
        let has_ge_36 = version_check(Some((VersionCompare::GreaterOrEqual, 36)), version);
        let has_ge_22 = version_check(Some((VersionCompare::GreaterOrEqual, 22)), version);

        let flag = reader.read_u32_le()?;
        let random_seed = reader.read_u32_le()?;
        let padding1 = reader.read_u32_le()?;
        let padding2 = reader.read_u32_le()?;

        let (name, namev40) = if has_lt_40 {
            (
                Some(reader.read_string(64)?).map(|s| s.replace('\0', "")),
                None,
            )
        } else {
            (
                None,
                Some(reader.read_string(96)?).map(|s| s.replace('\0', "")),
            )
        };

        let emitter_static = EmitterStatic::read(reader, version)?;
        let emitter_info = EmitterInfo::read(reader)?;
        let child_inheritance = EmitterInheritance::read(reader, version)?;
        let emission = Emission::read(reader)?;
        let shape_info = EmitterShapeInfo::read(reader, version)?;
        let render_state = EmitterRenderState::read(reader)?;
        let particle_data = ParticleData::read(reader, version)?;

        let combiner_temp = if has_lt_40 && !has_ge_36 {
            Some(CombinerTemp::Legacy(EmitterCombiner::read(reader)?))
        } else if has_eq_36 {
            Some(CombinerTemp::V36(EmitterCombinerV36::read(reader)?))
        } else if has_gt_40 {
            // Read combiner fields
            let color_combiner_process = reader.read_u8()?;
            let alpha_combiner_process = reader.read_u8()?;
            let texture1_color_blend = reader.read_u8()?;
            let texture2_color_blend = reader.read_u8()?;
            let primitive_color_blend = reader.read_u8()?;
            let texture1_alpha_blend = reader.read_u8()?;
            let texture2_alpha_blend = reader.read_u8()?;
            let primitive_alpha_blend = reader.read_u8()?;
            let tex_color0_input_type = reader.read_u8()?;
            let tex_color1_input_type = reader.read_u8()?;
            let tex_color2_input_type = reader.read_u8()?;
            let tex_alpha0_input_type = reader.read_u8()?;
            let tex_alpha1_input_type = reader.read_u8()?;
            let tex_alpha2_input_type = reader.read_u8()?;
            let primitive_color_input_type = reader.read_u8()?;
            let primitive_alpha_input_type = reader.read_u8()?;
            let padding = if version >= 50 {
                Some(reader.read_i16_le()?)
            } else {
                None
            };
            let padding2_opt = if version >= 50 {
                Some(reader.read_u32_le()?)
            } else {
                None
            };
            let padding3 = if version >= 50 {
                Some(reader.read_u32_le()?)
            } else {
                None
            };
            Some(CombinerTemp::V40((
                color_combiner_process,
                alpha_combiner_process,
                texture1_color_blend,
                texture2_color_blend,
                primitive_color_blend,
                texture1_alpha_blend,
                texture2_alpha_blend,
                primitive_alpha_blend,
                tex_color0_input_type,
                tex_color1_input_type,
                tex_color2_input_type,
                tex_alpha0_input_type,
                tex_alpha1_input_type,
                tex_alpha2_input_type,
                primitive_color_input_type,
                primitive_alpha_input_type,
                padding,
                padding2_opt,
                padding3,
            )))
        } else {
            None
        };

        let shader_references = ShaderRefInfo::read(reader, version)?;

        let combiner = match combiner_temp {
            Some(CombinerTemp::Legacy(c)) => Some(EmitterCombinerVariant::Legacy(c)),
            Some(CombinerTemp::V36(c)) => Some(EmitterCombinerVariant::V36(c)),
            Some(CombinerTemp::V40((
                color_combiner_process,
                alpha_combiner_process,
                texture1_color_blend,
                texture2_color_blend,
                primitive_color_blend,
                texture1_alpha_blend,
                texture2_alpha_blend,
                primitive_alpha_blend,
                tex_color0_input_type,
                tex_color1_input_type,
                tex_color2_input_type,
                tex_alpha0_input_type,
                tex_alpha1_input_type,
                tex_alpha2_input_type,
                primitive_color_input_type,
                primitive_alpha_input_type,
                padding,
                padding2_opt,
                padding3,
            ))) => Some(EmitterCombinerVariant::V40(CombinedEmitterCombinerV40 {
                color_combiner_process,
                alpha_combiner_process,
                texture1_color_blend,
                texture2_color_blend,
                primitive_color_blend,
                texture1_alpha_blend,
                texture2_alpha_blend,
                primitive_alpha_blend,
                tex_color0_input_type,
                tex_color1_input_type,
                tex_color2_input_type,
                tex_alpha0_input_type,
                tex_alpha1_input_type,
                tex_alpha2_input_type,
                primitive_color_input_type,
                primitive_alpha_input_type,
                shader_type: shader_references.type_,
                apply_alpha: shader_references.val_0x2,
                is_distortion_by_camera_distance: shader_references.val_0x3,
                padding1: shader_references.val_0x4,
                padding2: 0,
                padding,
                padding2_opt,
                padding3,
            })),
            None => None,
        };
        let action = ActionInfo::read(reader, version)?;

        let (depth_mode, pass_info) = if has_gt_40 {
            (
                Some(reader.read_string(16)?.replace('\0', "")),
                Some(reader.read_string(52)?.replace('\0', "")),
            )
        } else {
            (None, None)
        };

        let particle_velocity = ParticleVelocityInfo::read(reader)?;

        let unknown_v36 = if has_ge_36 {
            let mut arr = Vec::with_capacity(4);
            for _ in 0..4 {
                arr.push(reader.read_f32_le()?);
            }
            Some(arr)
        } else {
            None
        };

        let particle_color = ParticleColor::read(reader)?;
        let particle_scale = ParticleScale::read(reader)?;
        let particle_fluctuation = ParticleFlucInfo::read(reader)?;

        let sampler0 = TextureSampler::read(reader, version)?;
        let sampler1 = TextureSampler::read(reader, version)?;
        let sampler2 = TextureSampler::read(reader, version)?;

        let (sampler3, sampler4, sampler5) = if has_gt_40 {
            (
                Some(TextureSampler::read(reader, version)?),
                Some(TextureSampler::read(reader, version)?),
                Some(TextureSampler::read(reader, version)?),
            )
        } else {
            (None, None, None)
        };

        let texture_anim0 = TextureAnim::read(reader, version)?;
        let texture_anim1 = TextureAnim::read(reader, version)?;
        let texture_anim2 = TextureAnim::read(reader, version)?;

        let (texture_anim3, texture_anim4, texture_anim5) = if has_gt_40 {
            (
                Some(TextureAnim::read(reader, version)?),
                Some(TextureAnim::read(reader, version)?),
                Some(TextureAnim::read(reader, version)?),
            )
        } else {
            (None, None, None)
        };

        let reserved = if has_ge_22 {
            reader.read_bytes(0x40)?
        } else {
            Vec::new()
        };

        Ok(EmitterData {
            action,
            child_inheritance,
            combiner,
            emission,
            emitter_info,
            emitter_static,
            flag,
            name,
            padding1,
            padding2,
            particle_color,
            particle_data,
            particle_scale,
            particle_velocity,
            random_seed,
            render_state,
            reserved,
            shader_references,
            shape_info,
            namev40,
            depth_mode,
            pass_info,
            unknown_v36,
            particle_fluctuation: Some(particle_fluctuation),
            sampler0: Some(sampler0),
            sampler1: Some(sampler1),
            sampler2: Some(sampler2),
            sampler3,
            sampler4,
            sampler5,
            texture_anim0: Some(texture_anim0),
            texture_anim1: Some(texture_anim1),
            texture_anim2: Some(texture_anim2),
            texture_anim3,
            texture_anim4,
            texture_anim5,
            order: 0,
        })
    }

    /// Get the display name for this emitter.
    pub fn display_name(&self) -> String {
        if let Some(ref n) = self.namev40 {
            if !n.is_empty() {
                return n.clone();
            }
        }
        if let Some(ref n) = self.name {
            return n.clone();
        }
        String::new()
    }

    /// Load emitter data from dumped JSON, mirroring C# `PtclSerialize.Deserialize`.
    pub fn from_json(json: &str, version: u16) -> std::io::Result<Self> {
        let mut data: EmitterData = serde_json::from_str(json)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;

        // `EmitterCombinerVariant` is untagged, so the derive above cannot tell the variants
        // apart from the JSON alone and always settles on the earliest one that fits. Re-pick it
        // from the version, which is what the writer keys off; see
        // `EmitterCombinerVariant::from_json_value`.
        //
        // Only versions above 40 actually land on the wrong variant: below that the earliest
        // fitting variant is already the right one. Checking first keeps the second parse off
        // the common path instead of paying for it on every emitter.
        let combiner_is_wrong = match (
            data.combiner.as_ref(),
            EmitterCombinerVariant::expected_kind(version),
        ) {
            (Some(current), Some(expected)) => current.kind() != expected,
            _ => false,
        };

        if combiner_is_wrong {
            #[derive(Deserialize)]
            struct CombinerOnly {
                #[serde(rename = "Combiner")]
                combiner: Option<serde_json::Value>,
            }

            let raw: CombinerOnly = serde_json::from_str(json).map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
            })?;
            if let Some(value) = raw.combiner.filter(|value| !value.is_null()) {
                if let Some(combiner) = EmitterCombinerVariant::from_json_value(&value, version)? {
                    data.combiner = Some(combiner);
                }
            }
        }

        if version >= 40 && data.namev40.is_none() {
            data.namev40 = data.name.clone();
        }
        Ok(data)
    }

    /// Get all texture samplers as a flat list.
    pub fn get_samplers(&self) -> Vec<&TextureSampler> {
        let mut s = Vec::new();
        if let Some(ref s0) = self.sampler0 {
            s.push(s0);
        }
        if let Some(ref s1) = self.sampler1 {
            s.push(s1);
        }
        if let Some(ref s2) = self.sampler2 {
            s.push(s2);
        }
        if let Some(ref s3) = self.sampler3 {
            s.push(s3);
        }
        if let Some(ref s4) = self.sampler4 {
            s.push(s4);
        }
        if let Some(ref s5) = self.sampler5 {
            s.push(s5);
        }
        s
    }
}

// ─── EmitterAnimation (sub-section) ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitterAnimation {
    pub magic: String,
    pub data: Vec<u8>,
}

impl EmitterAnimation {
    pub fn read<R: Read>(reader: &mut R, magic: String, data_size: usize) -> std::io::Result<Self> {
        Ok(EmitterAnimation {
            magic,
            data: reader.read_bytes(data_size)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialize_animation_key_matches_baseline() {
        let key = AnimationKey {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            time: 4.0,
        };
        let value = serde_json::to_value(key).unwrap();
        assert_eq!(value, json!({"X": 1.0, "Y": 2.0, "Z": 3.0, "Time": 4.0}));
    }

    #[test]
    fn emitter_data_binary_write_roundtrip() {
        let sample = std::path::Path::new(
            "/home/leap/Workshop/Smash Mod Tools/ArcExplorer_linux_x64/export/effect/pokemon/mijumaru/ef_mijumaru.eff",
        );
        if !sample.is_file() {
            return;
        }

        let data = std::fs::read(sample).expect("read sample eff");
        let namco = crate::NamcoEffectFile::load(&data).expect("load eff");
        let ptcl = namco.ptcl_file.as_ref().expect("ptcl present");
        let version = ptcl.vfx_version;

        for emitter_set in &ptcl.emitter_list.emitter_sets {
            for emitter in &emitter_set.emitters {
                let original = emitter
                    .binary_data
                    .as_ref()
                    .expect("emitter binary present");
                let rewritten = emitter.data.write(version).expect("write emitter data");
                assert_eq!(
                    original.as_slice(),
                    rewritten.as_slice(),
                    "emitter {:?} write mismatch",
                    emitter.data.display_name()
                );
            }
        }
    }

    /// Build an emitter whose every binary field is zero, so the only thing a round-trip can
    /// disturb is what the test deliberately puts in it.
    fn zeroed_emitter_data(version: u16) -> EmitterData {
        let mut cursor = std::io::Cursor::new(vec![0u8; 0x8000]);
        EmitterData::read(&mut cursor, version).expect("read zeroed emitter data")
    }

    fn combiner_variant_name(combiner: &Option<EmitterCombinerVariant>) -> &'static str {
        match combiner {
            Some(EmitterCombinerVariant::Legacy(_)) => "Legacy",
            Some(EmitterCombinerVariant::V36(_)) => "V36",
            Some(EmitterCombinerVariant::V40(_)) => "V40",
            None => "None",
        }
    }

    /// `Namev40`, `DepthMode`, `PassInfo` and `UnknownV36` are read from the binary only at
    /// certain versions. They were `#[serde(skip)]`, so the dump dropped them and the rebuild
    /// wrote empty strings and zero floats over them. Above v40 that included the emitter's
    /// name, which lives in `namev40` rather than `name`.
    #[test]
    fn version_gated_fields_survive_json_roundtrip() {
        for version in [22u16, 36, 41, 50] {
            let mut data = zeroed_emitter_data(version);

            // Only stamp what this version actually carries; the others stay None so the dump
            // keeps omitting them, which is what matches the C# exporter.
            if data.namev40.is_some() {
                data.namev40 = Some("emitter_name_above_v40".to_string());
            }
            if data.depth_mode.is_some() {
                data.depth_mode = Some("depth_mode".to_string());
            }
            if data.pass_info.is_some() {
                data.pass_info = Some("pass_info".to_string());
            }
            if data.unknown_v36.is_some() {
                data.unknown_v36 = Some(vec![1.5, -2.25, 3.75, 4.0]);
            }

            let json = serde_json::to_string(&data).expect("serialize emitter data");
            let restored = EmitterData::from_json(&json, version).expect("from_json");

            assert_eq!(
                data.namev40, restored.namev40,
                "version {version} lost Namev40 through JSON"
            );
            assert_eq!(
                data.depth_mode, restored.depth_mode,
                "version {version} lost DepthMode through JSON"
            );
            assert_eq!(
                data.pass_info, restored.pass_info,
                "version {version} lost PassInfo through JSON"
            );
            assert_eq!(
                data.unknown_v36, restored.unknown_v36,
                "version {version} lost UnknownV36 through JSON"
            );
            assert_eq!(
                data.write(version).expect("write original"),
                restored.write(version).expect("write restored"),
                "version {version} write mismatch after JSON round-trip"
            );
        }
    }

    /// Below v41 these fields are absent, and the C# exporter dumps with
    /// `NullValueHandling.Ignore`, so the keys must stay out of the JSON entirely.
    #[test]
    fn version_gated_fields_are_absent_when_unset() {
        let value = serde_json::to_value(zeroed_emitter_data(22)).unwrap();
        for key in ["Namev40", "DepthMode", "PassInfo", "UnknownV36"] {
            assert!(
                value.get(key).is_none(),
                "v22 dump should not carry {key}, C# omits it"
            );
        }

        let value = serde_json::to_value(zeroed_emitter_data(41)).unwrap();
        for key in ["Namev40", "DepthMode", "PassInfo", "UnknownV36"] {
            assert!(
                value.get(key).is_some(),
                "v41 dump should carry {key}, C# emits it"
            );
        }
    }

    /// `EmitterCombinerVariant` is `#[serde(untagged)]` and its variants share field names, so
    /// serde used to reconstruct the wrong one and `write` then emitted no combiner at all.
    #[test]
    fn combiner_variant_survives_json_roundtrip() {
        for (version, expected) in [(22u16, "Legacy"), (36, "V36"), (41, "V40"), (50, "V40")] {
            let mut data = zeroed_emitter_data(version);

            // Distinct non-zero values so a dropped or mis-parsed combiner shows up in the bytes.
            match &mut data.combiner {
                Some(EmitterCombinerVariant::Legacy(c)) => {
                    c.color_combiner_process = 1;
                    c.primitive_alpha_blend = 7;
                    c.shader_type = 9;
                }
                Some(EmitterCombinerVariant::V36(c)) => {
                    c.color_combiner_process = 1;
                    c.primitive_alpha_blend = 7;
                }
                Some(EmitterCombinerVariant::V40(c)) => {
                    c.color_combiner_process = 1;
                    c.primitive_alpha_blend = 7;
                    c.primitive_alpha_input_type = 15;
                    if version >= 50 {
                        c.padding = Some(-2);
                        c.padding2_opt = Some(3);
                        c.padding3 = Some(4);
                    }
                }
                None => panic!("version {version} produced no combiner"),
            }

            assert_eq!(
                combiner_variant_name(&data.combiner),
                expected,
                "version {version} read the wrong combiner variant"
            );

            let json = serde_json::to_string(&data).expect("serialize emitter data");
            let restored = EmitterData::from_json(&json, version).expect("from_json");

            assert_eq!(
                combiner_variant_name(&restored.combiner),
                expected,
                "version {version} lost its combiner variant through JSON"
            );
            assert_eq!(
                data.write(version).expect("write original"),
                restored.write(version).expect("write restored"),
                "version {version} write mismatch after JSON round-trip"
            );
        }
    }

    /// The serialized shape must not move: the dump has to stay byte-compatible with the
    /// reference C# exporter, so only the deserialize side was allowed to change.
    #[test]
    fn combiner_serialized_shape_is_unchanged() {
        let v40 = CombinedEmitterCombinerV40 {
            color_combiner_process: 1,
            alpha_combiner_process: 2,
            texture1_color_blend: 3,
            texture2_color_blend: 4,
            primitive_color_blend: 5,
            texture1_alpha_blend: 6,
            texture2_alpha_blend: 7,
            primitive_alpha_blend: 8,
            tex_color0_input_type: 9,
            tex_color1_input_type: 10,
            tex_color2_input_type: 11,
            tex_alpha0_input_type: 12,
            tex_alpha1_input_type: 13,
            tex_alpha2_input_type: 14,
            primitive_color_input_type: 15,
            primitive_alpha_input_type: 16,
            shader_type: 17,
            apply_alpha: 18,
            is_distortion_by_camera_distance: 19,
            padding1: 20,
            padding2: 21,
            padding: None,
            padding2_opt: None,
            padding3: None,
        };
        let value = serde_json::to_value(EmitterCombinerVariant::V40(v40)).unwrap();
        assert_eq!(value["ColorCombinerProcess"], json!(1));
        assert_eq!(value["PrimitiveAlphaInputType"], json!(16));
        assert_eq!(value["ShaderType"], json!(17));
        assert_eq!(value["padding1"], json!(20));
        assert_eq!(value["padding2"], json!(21));
        assert!(value.get("Padding").is_none());
        assert!(value.get("Padding2").is_none());
        assert!(value.get("Padding3").is_none());
    }

    #[test]
    fn serialize_shader_ref_info_type_field_matches_baseline() {
        let info = ShaderRefInfo {
            type_: 1,
            val_0x2: 2,
            val_0x3: 3,
            val_0x4: 4,
            shader_index: -1,
            compute_shader_index: -2,
            user_shader_index1: -3,
            user_shader_index2: -4,
            custom_shader_index: -5,
            custom_shader_flag: None,
            custom_shader_switch: None,
            unknown1: 0,
            extra_shader_index2: -6,
            val_0x34: -7,
            unknown2: 0,
            user_shader_define1: vec![],
            user_shader_define2: vec![],
        };
        let value = serde_json::to_value(&info).unwrap();
        assert_eq!(value["Type"], json!(1));
        assert_eq!(value["ShaderIndex"], json!(-1));
        assert_eq!(value["ComputeShaderIndex"], json!(-2));
    }
}

// ─── BNSH Exporter ──────────────────────────────────────────────────────-

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BnshFile {
    pub magic: String,
    pub version: u32,
    pub variations: Vec<ShaderVariation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShaderVariation {
    pub name: String,
    pub binary_data: Vec<u8>,
}

impl BnshFile {
    pub fn new(magic: &str, version: u32) -> Self {
        BnshFile {
            magic: magic.to_string(),
            version,
            variations: Vec::new(),
        }
    }

    pub fn add_variation(&mut self, name: &str, binary_data: Vec<u8>) {
        self.variations.push(ShaderVariation {
            name: name.to_string(),
            binary_data,
        });
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut result = Vec::new();

        // Write magic and version
        result.extend_from_slice(self.magic.as_bytes());
        result.extend_from_slice(&self.version.to_le_bytes());

        // Write number of variations
        let num_variations = self.variations.len() as u32;
        result.extend_from_slice(&num_variations.to_le_bytes());

        // Write each variation
        for variation in &self.variations {
            // Write name length and name
            let name_len = variation.name.len() as u32;
            result.extend_from_slice(&name_len.to_le_bytes());
            result.extend_from_slice(variation.name.as_bytes());

            // Write binary data size and data
            let data_size = variation.binary_data.len() as u32;
            result.extend_from_slice(&data_size.to_le_bytes());
            result.extend_from_slice(&variation.binary_data);
        }

        result
    }
}
