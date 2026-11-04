//! Auto-generated write implementations mirroring emitter read order.
//! Regenerate: python3 crate/scripts/generate_emitter_write.py

use super::*;
use byteorder::{LittleEndian, WriteBytesExt};
use std::io::{self, Write};

pub trait WriterExt: Write {
    fn write_u8(&mut self, v: u8) -> io::Result<()> {
        self.write_all(&[v])
    }
    fn write_i16_le(&mut self, v: i16) -> io::Result<()> {
        self.write_i16::<LittleEndian>(v).map(|_| ())
    }
    fn write_u16_le(&mut self, v: u16) -> io::Result<()> {
        self.write_u16::<LittleEndian>(v).map(|_| ())
    }
    fn write_i32_le(&mut self, v: i32) -> io::Result<()> {
        self.write_i32::<LittleEndian>(v).map(|_| ())
    }
    fn write_u32_le(&mut self, v: u32) -> io::Result<()> {
        self.write_u32::<LittleEndian>(v).map(|_| ())
    }
    fn write_i64_le(&mut self, v: i64) -> io::Result<()> {
        self.write_i64::<LittleEndian>(v).map(|_| ())
    }
    fn write_u64_le(&mut self, v: u64) -> io::Result<()> {
        self.write_u64::<LittleEndian>(v).map(|_| ())
    }
    fn write_f32_le(&mut self, v: f32) -> io::Result<()> {
        self.write_f32::<LittleEndian>(v).map(|_| ())
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        self.write_all(data)
    }
    fn write_fixed_string(&mut self, s: &str, len: usize) -> io::Result<()> {
        let mut buf = vec![0u8; len];
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(len);
        buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.write_all(&buf)
    }
}

impl<W: Write> WriterExt for W {}

fn write_bool_u8<W: WriterExt>(writer: &mut W, v: bool) -> io::Result<()> {
    writer.write_u8(v as u8)
}

impl TextureAnim {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.pattern_anim_type)?;
        write_bool_u8(writer, self.is_scroll)?;
        write_bool_u8(writer, self.is_rotate)?;
        write_bool_u8(writer, self.is_scale)?;
        writer.write_u8(self.repeat)?;
        writer.write_u8(self.inv_rand_u)?;
        writer.write_u8(self.inv_rand_v)?;
        writer.write_u8(self.is_pat_anim_loop_random)?;
        writer.write_u8(self.uv_channel)?;
        writer.write_u8(self.is_crossfade)?;
        writer.write_u8(self.padding1)?;
        writer.write_u8(self.padding2)?;
        writer.write_u32_le(self.padding3)?;
        Ok(())
    }
}

impl AnimationKey {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_f32_le(self.x)?;
        writer.write_f32_le(self.y)?;
        writer.write_f32_le(self.z)?;
        writer.write_f32_le(self.time)?;
        Ok(())
    }
}

impl AnimationKeyTable {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        for key in &self.keys {
            key.write(writer)?;
        }
        Ok(())
    }
}

impl TexPatAnim {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_f32_le(self.num)?;
        writer.write_f32_le(self.frequency)?;
        writer.write_f32_le(self.num_random)?;
        writer.write_f32_le(self.pad)?;
        for val in &self.table {
            writer.write_i32_le(*val)?;
        }
        Ok(())
    }
}

impl TextureSampler {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        writer.write_u64_le(self.texture_id)?;
        writer.write_u8(self.wrap_u.as_u8())?;
        writer.write_u8(self.wrap_v.as_u8())?;
        writer.write_u8(self.filter)?;
        writer.write_u8(self.is_sphere_map)?;
        writer.write_f32_le(self.max_lod)?;
        writer.write_f32_le(self.lod_bias)?;
        writer.write_u8(self.mip_level_limit)?;
        writer.write_u8(self.is_density_fixed_u)?;
        writer.write_u8(self.is_density_fixed_v)?;
        writer.write_u8(self.is_square_rgb)?;
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_u8(self.is_on_another_binary.unwrap_or(0))?;
            writer.write_u8(self.padding1.unwrap_or(0))?;
            writer.write_u8(self.padding2.unwrap_or(0))?;
            writer.write_u8(self.padding3.unwrap_or(0))?;
            writer.write_u32_le(self.padding4.unwrap_or(0))?;
        }
        Ok(())
    }
}

impl TexScrollAnim {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_f32_le(self.scroll_add_x)?;
        writer.write_f32_le(self.scroll_add_y)?;
        writer.write_f32_le(self.scroll_x)?;
        writer.write_f32_le(self.scroll_y)?;
        writer.write_f32_le(self.scroll_random_x)?;
        writer.write_f32_le(self.scroll_random_y)?;
        writer.write_f32_le(self.scale_add_x)?;
        writer.write_f32_le(self.scale_add_y)?;
        writer.write_f32_le(self.scale_x)?;
        writer.write_f32_le(self.scale_y)?;
        writer.write_f32_le(self.scale_random_x)?;
        writer.write_f32_le(self.scale_random_y)?;
        writer.write_f32_le(self.rotation_add)?;
        writer.write_f32_le(self.rotation)?;
        writer.write_f32_le(self.rotation_random)?;
        writer.write_f32_le(self.rotation_type)?;
        writer.write_f32_le(self.uv_scale_x)?;
        writer.write_f32_le(self.uv_scale_y)?;
        writer.write_f32_le(self.uv_div_x)?;
        writer.write_f32_le(self.uv_div_y)?;
        Ok(())
    }
}

impl EmitterStatic {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        writer.write_u32_le(self.flags1)?;
        writer.write_u32_le(self.flags2)?;
        writer.write_u32_le(self.flags3)?;
        writer.write_u32_le(self.flags4)?;
        writer.write_u32_le(self.num_color0_keys)?;
        writer.write_u32_le(self.num_alpha0_keys)?;
        writer.write_u32_le(self.num_color1_keys)?;
        writer.write_u32_le(self.num_alpha1_keys)?;
        writer.write_u32_le(self.num_scale_keys)?;
        writer.write_u32_le(self.num_param_keys)?;
        writer.write_u32_le(self.unknown1)?;
        writer.write_u32_le(self.unknown2)?;
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_u32_le(self.num_anim2_keys)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_u32_le(self.num_anim3_keys)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_u32_le(self.num_anim4_keys)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_u32_le(self.num_anim5_keys)?;
        }
        writer.write_f32_le(self.color0_loop_rate)?;
        writer.write_f32_le(self.alpha0_loop_rate)?;
        writer.write_f32_le(self.color1_loop_rate)?;
        writer.write_f32_le(self.alpha1_loop_rate)?;
        writer.write_f32_le(self.scale_loop_rate)?;
        writer.write_f32_le(self.color0_loop_random)?;
        writer.write_f32_le(self.alpha0_loop_random)?;
        writer.write_f32_le(self.color1_loop_random)?;
        writer.write_f32_le(self.alpha1_loop_random)?;
        writer.write_f32_le(self.scale_loop_random)?;
        writer.write_f32_le(self.unknown3)?;
        writer.write_f32_le(self.unknown4)?;
        writer.write_f32_le(self.gravity_dir_x)?;
        writer.write_f32_le(self.gravity_dir_y)?;
        writer.write_f32_le(self.gravity_dir_z)?;
        writer.write_f32_le(self.gravity_scale)?;
        writer.write_f32_le(self.air_res)?;
        writer.write_f32_le(self.val_0x74)?;
        writer.write_f32_le(self.val_0x78)?;
        writer.write_f32_le(self.val_0x82)?;
        writer.write_f32_le(self.center_x)?;
        writer.write_f32_le(self.center_y)?;
        writer.write_f32_le(self.offset)?;
        writer.write_f32_le(self.padding)?;
        writer.write_f32_le(self.amplitude_x)?;
        writer.write_f32_le(self.amplitude_y)?;
        writer.write_f32_le(self.cycle_x)?;
        writer.write_f32_le(self.cycle_y)?;
        writer.write_f32_le(self.phase_rnd_x)?;
        writer.write_f32_le(self.phase_rnd_y)?;
        writer.write_f32_le(self.phase_init_x)?;
        writer.write_f32_le(self.phase_init_y)?;
        writer.write_f32_le(self.coefficient0)?;
        writer.write_f32_le(self.coefficient1)?;
        writer.write_f32_le(self.val_0xb8)?;
        writer.write_f32_le(self.val_0xbc)?;
        self.tex_pattern_anim0.write(writer)?;
        self.tex_pattern_anim1.write(writer)?;
        self.tex_pattern_anim2.write(writer)?;
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(v) = &self.tex_pattern_anim3 {
                v.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(v) = &self.tex_pattern_anim4 {
                v.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(v) = &self.tex_pattern_anim5 {
                v.write(writer)?;
            }
        }
        self.tex_scroll_anim0.write(writer)?;
        self.tex_scroll_anim1.write(writer)?;
        self.tex_scroll_anim2.write(writer)?;
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(v) = &self.tex_scroll_anim3 {
                v.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(v) = &self.tex_scroll_anim4 {
                v.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(v) = &self.tex_scroll_anim5 {
                v.write(writer)?;
            }
        }
        writer.write_f32_le(self.color_scale)?;
        writer.write_f32_le(self.val_0x364)?;
        writer.write_f32_le(self.val_0x368)?;
        writer.write_f32_le(self.val_0x36a)?;
        self.color0.write(writer)?;
        self.alpha0.write(writer)?;
        self.color1.write(writer)?;
        self.alpha1.write(writer)?;
        writer.write_f32_le(self.soft_edge_param1)?;
        writer.write_f32_le(self.soft_edge_param2)?;
        writer.write_f32_le(self.fresnel_alpha_param1)?;
        writer.write_f32_le(self.fresnel_alpha_param2)?;
        writer.write_f32_le(self.near_dist_alpha_param1)?;
        writer.write_f32_le(self.near_dist_alpha_param2)?;
        writer.write_f32_le(self.far_dist_alpha_param1)?;
        writer.write_f32_le(self.far_dist_alpha_param2)?;
        writer.write_f32_le(self.decal_param1)?;
        writer.write_f32_le(self.decal_param2)?;
        writer.write_f32_le(self.alpha_threshold)?;
        writer.write_f32_le(self.padding2)?;
        writer.write_f32_le(self.add_vel_to_scale)?;
        writer.write_f32_le(self.soft_partcile_dist)?;
        writer.write_f32_le(self.soft_particle_volume)?;
        writer.write_f32_le(self.padding3)?;
        self.scale_anim.write(writer)?;
        self.param_anim.write(writer)?;
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            if let Some(keys) = &self.anim1_keys {
                keys.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            if let Some(keys) = &self.anim2_keys {
                keys.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            if let Some(keys) = &self.anim3_keys {
                keys.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            if let Some(keys) = &self.anim4_keys {
                keys.write(writer)?;
            }
        }
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(arr) = &self.unknown6 {
                for v in arr {
                    writer.write_f32_le(*v)?;
                }
            } else {
                for _ in 0..16 {
                    writer.write_f32_le(0.0)?;
                }
            }
        }
        writer.write_f32_le(self.rotate_init_x)?;
        writer.write_f32_le(self.rotate_init_y)?;
        writer.write_f32_le(self.rotate_init_z)?;
        writer.write_f32_le(self.rotate_init_empty)?;
        writer.write_f32_le(self.rotate_init_rand_x)?;
        writer.write_f32_le(self.rotate_init_rand_y)?;
        writer.write_f32_le(self.rotate_init_rand_z)?;
        writer.write_f32_le(self.rotate_init_rand_empty)?;
        writer.write_f32_le(self.rotate_add_x)?;
        writer.write_f32_le(self.rotate_add_y)?;
        writer.write_f32_le(self.rotate_add_z)?;
        writer.write_f32_le(self.rotate_regist)?;
        writer.write_f32_le(self.rotate_add_rand_x)?;
        writer.write_f32_le(self.rotate_add_rand_y)?;
        writer.write_f32_le(self.rotate_add_rand_z)?;
        writer.write_f32_le(self.padding4)?;
        writer.write_f32_le(self.scale_limit_dist_near)?;
        writer.write_f32_le(self.scale_limit_dist_far)?;
        writer.write_f32_le(self.padding5)?;
        writer.write_f32_le(self.padding6)?;
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(arr) = &self.unknown7 {
                for v in arr {
                    writer.write_f32_le(*v)?;
                }
            } else {
                for _ in 0..16 {
                    writer.write_f32_le(0.0)?;
                }
            }
        }
        Ok(())
    }
}

impl EmitterInfo {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.is_particle_draw)?;
        writer.write_u8(self.sort_type)?;
        writer.write_u8(self.calc_type)?;
        writer.write_u8(self.follow_type)?;
        writer.write_u8(self.is_fade_emit)?;
        writer.write_u8(self.is_fade_alpha_fade)?;
        writer.write_u8(self.is_scale_fade)?;
        writer.write_u8(self.random_seed_type)?;
        writer.write_u8(self.is_update_matrix_by_emit)?;
        writer.write_u8(self.test_always)?;
        writer.write_u8(self.interpolate_emission_amount)?;
        writer.write_u8(self.is_alpha_fade_in)?;
        writer.write_u8(self.is_scale_fade_in)?;
        writer.write_u8(self.padding1)?;
        writer.write_u8(self.padding2)?;
        writer.write_u8(self.padding3)?;
        writer.write_u32_le(self.random_seed)?;
        writer.write_u32_le(self.draw_path)?;
        writer.write_i32_le(self.alpha_fade_time)?;
        writer.write_i32_le(self.fade_in_time)?;
        writer.write_f32_le(self.trans_x)?;
        writer.write_f32_le(self.trans_y)?;
        writer.write_f32_le(self.trans_z)?;
        writer.write_f32_le(self.trans_rand_x)?;
        writer.write_f32_le(self.trans_rand_y)?;
        writer.write_f32_le(self.trans_rand_z)?;
        writer.write_f32_le(self.rotate_x)?;
        writer.write_f32_le(self.rotate_y)?;
        writer.write_f32_le(self.rotate_z)?;
        writer.write_f32_le(self.rotate_rand_x)?;
        writer.write_f32_le(self.rotate_rand_y)?;
        writer.write_f32_le(self.rotate_rand_z)?;
        writer.write_f32_le(self.scale_x)?;
        writer.write_f32_le(self.scale_y)?;
        writer.write_f32_le(self.scale_z)?;
        writer.write_f32_le(self.color0_r)?;
        writer.write_f32_le(self.color0_g)?;
        writer.write_f32_le(self.color0_b)?;
        writer.write_f32_le(self.color0_a)?;
        writer.write_f32_le(self.color1_r)?;
        writer.write_f32_le(self.color1_g)?;
        writer.write_f32_le(self.color1_b)?;
        writer.write_f32_le(self.color1_a)?;
        writer.write_f32_le(self.emission_range_near)?;
        writer.write_f32_le(self.emission_range_far)?;
        writer.write_f32_le(self.emission_ratio_far)?;
        Ok(())
    }
}

impl EmitterInheritance {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        writer.write_u8(self.velocity)?;
        writer.write_u8(self.scale)?;
        writer.write_u8(self.rotate)?;
        writer.write_u8(self.color_scale)?;
        writer.write_u8(self.color0)?;
        writer.write_u8(self.color1)?;
        writer.write_u8(self.alpha0)?;
        writer.write_u8(self.alpha1)?;
        writer.write_u8(self.draw_path)?;
        writer.write_u8(self.pre_draw)?;
        writer.write_u8(self.alpha0_each_frame)?;
        writer.write_u8(self.alpha1_each_frame)?;
        writer.write_u8(self.enable_emitter_particle)?;
        writer.write_u8(self.padding1)?;
        writer.write_u8(self.padding2)?;
        writer.write_u8(self.padding3)?;
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            writer.write_u64_le(self.unknown_v40)?;
        }
        writer.write_f32_le(self.velocity_rate)?;
        writer.write_f32_le(self.scale_rate)?;
        Ok(())
    }
}

impl Emission {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        write_bool_u8(writer, self.is_one_time)?;
        write_bool_u8(writer, self.is_world_gravity)?;
        write_bool_u8(writer, self.is_emit_dist_enabled)?;
        write_bool_u8(writer, self.is_world_oriented_velocity)?;
        writer.write_u32_le(self.start)?;
        writer.write_u32_le(self.timing)?;
        writer.write_u32_le(self.duration)?;
        writer.write_f32_le(self.rate)?;
        writer.write_f32_le(self.rate_random)?;
        writer.write_i32_le(self.interval)?;
        writer.write_f32_le(self.interval_random)?;
        writer.write_f32_le(self.position_random)?;
        writer.write_f32_le(self.gravity_scale)?;
        writer.write_f32_le(self.gravity_dir_x)?;
        writer.write_f32_le(self.gravity_dir_y)?;
        writer.write_f32_le(self.gravity_dir_z)?;
        writer.write_f32_le(self.emitter_dist_unit)?;
        writer.write_f32_le(self.emitter_dist_min)?;
        writer.write_f32_le(self.emitter_dist_max)?;
        writer.write_f32_le(self.emitter_dist_marg)?;
        writer.write_i32_le(self.emitter_dist_particles_max)?;
        Ok(())
    }
}

impl EmitterShapeInfo {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        writer.write_u8(self.volume_type)?;
        writer.write_u8(self.sweep_start_random)?;
        writer.write_u8(self.arc_type)?;
        writer.write_u8(self.is_volume_latitude_enabled)?;
        writer.write_u8(self.volume_tbl_index)?;
        writer.write_u8(self.volume_tbl_index64)?;
        writer.write_u8(self.volume_latitude_dir)?;
        writer.write_u8(self.is_gpu_emitter)?;
        writer.write_f32_le(self.sweep_longitude)?;
        writer.write_f32_le(self.sweep_latitude)?;
        writer.write_f32_le(self.sweep_start)?;
        writer.write_f32_le(self.volume_surface_pos_rand)?;
        writer.write_f32_le(self.caliber_ratio)?;
        writer.write_f32_le(self.line_center)?;
        writer.write_f32_le(self.line_length)?;
        writer.write_f32_le(self.volume_radius_x)?;
        writer.write_f32_le(self.volume_radius_y)?;
        writer.write_f32_le(self.volume_radius_z)?;
        writer.write_f32_le(self.volume_form_scale_x)?;
        writer.write_f32_le(self.volume_form_scale_y)?;
        writer.write_f32_le(self.volume_form_scale_z)?;
        writer.write_i32_le(self.prim_emit_type)?;
        writer.write_u64_le(self.primitive_index)?;
        writer.write_i32_le(self.num_divide_circle)?;
        writer.write_i32_le(self.num_divide_circle_random)?;
        writer.write_i32_le(self.num_divide_line)?;
        writer.write_i32_le(self.num_divide_line_random)?;
        if version_check(Some((VersionCompare::Less, 40)), version) {
            if let Some(v) = self.is_on_another_binary_volume_primitive {
                writer.write_u8(v)?;
            }
            if let Some(v) = self.padding1 {
                writer.write_u8(v)?;
            }
            if let Some(v) = self.padding2 {
                writer.write_u8(v)?;
            }
            if let Some(v) = self.padding3 {
                writer.write_u8(v)?;
            }
            if let Some(v) = self.padding4 {
                writer.write_u32_le(v)?;
            }
        }
        Ok(())
    }
}

impl EmitterRenderState {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        write_bool_u8(writer, self.is_blend_enable)?;
        write_bool_u8(writer, self.is_depth_test)?;
        writer.write_u8(self.depth_func)?;
        write_bool_u8(writer, self.is_depth_mask)?;
        write_bool_u8(writer, self.is_alpha_test)?;
        writer.write_u8(self.alpha_func)?;
        writer.write_u8(self.blend_type)?;
        writer.write_u8(self.display_side)?;
        writer.write_f32_le(self.alpha_threshold)?;
        writer.write_u32_le(self.padding)?;
        Ok(())
    }
}

impl ParticleData {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        write_bool_u8(writer, self.infinite_life)?;
        write_bool_u8(writer, self.is_triming)?;
        writer.write_u8(self.billboard_type)?;
        writer.write_u8(self.rot_type)?;
        writer.write_u8(self.offset_type)?;
        write_bool_u8(writer, self.rot_rev_rand_x)?;
        write_bool_u8(writer, self.rot_rev_rand_y)?;
        write_bool_u8(writer, self.rot_rev_rand_z)?;
        write_bool_u8(writer, self.is_rotate_x)?;
        write_bool_u8(writer, self.is_rotate_y)?;
        writer.write_u8(self.is_rotate_z)?;
        writer.write_u8(self.primitive_scale_type)?;
        writer.write_u8(self.is_texture_common_random)?;
        writer.write_u8(self.connect_ptcl_scale_and_z_offset)?;
        writer.write_u8(self.enable_avoid_z_fighting)?;
        writer.write_u8(self.val_0xf)?;
        writer.write_i32_le(self.life)?;
        writer.write_i32_le(self.life_random)?;
        writer.write_f32_le(self.momentum_random)?;
        writer.write_u32_le(self.primitive_vertex_info_flags)?;
        writer.write_u64_le(self.primitive_id)?;
        writer.write_u64_le(self.primitive_ex_id)?;
        write_bool_u8(writer, self.loop_color0)?;
        write_bool_u8(writer, self.loop_alpha0)?;
        write_bool_u8(writer, self.loop_color1)?;
        write_bool_u8(writer, self.loop_alpha1)?;
        write_bool_u8(writer, self.scale_loop)?;
        write_bool_u8(writer, self.loop_random_color0)?;
        write_bool_u8(writer, self.loop_random_alpha0)?;
        write_bool_u8(writer, self.loop_random_color1)?;
        write_bool_u8(writer, self.loop_random_alpha1)?;
        write_bool_u8(writer, self.scale_loop_random)?;
        writer.write_u8(self.prim_flag1)?;
        writer.write_u8(self.prim_flag2)?;
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_i32_le(self.color0_loop_rate)?;
        }
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_i32_le(self.alpha0_loop_rate)?;
        }
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_i32_le(self.color1_loop_rate)?;
        }
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_i32_le(self.alpha1_loop_rate)?;
        }
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_i32_le(self.scale_loop_rate)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_i16_le(self.color0_loop_rate16)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_i16_le(self.alpha0_loop_rate16)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_i16_le(self.color1_loop_rate16)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_i16_le(self.alpha1_loop_rate16)?;
        }
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_i16_le(self.scale_loop_rate16)?;
        }
        Ok(())
    }
}

impl EmitterCombiner {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.color_combiner_process)?;
        writer.write_u8(self.alpha_combiner_process)?;
        writer.write_u8(self.texture1_color_blend)?;
        writer.write_u8(self.texture2_color_blend)?;
        writer.write_u8(self.primitive_color_blend)?;
        writer.write_u8(self.texture1_alpha_blend)?;
        writer.write_u8(self.texture2_alpha_blend)?;
        writer.write_u8(self.primitive_alpha_blend)?;
        writer.write_u8(self.tex_color0_input_type)?;
        writer.write_u8(self.tex_color1_input_type)?;
        writer.write_u8(self.tex_color2_input_type)?;
        writer.write_u8(self.tex_alpha0_input_type)?;
        writer.write_u8(self.tex_alpha1_input_type)?;
        writer.write_u8(self.tex_alpha2_input_type)?;
        writer.write_u8(self.primitive_color_input_type)?;
        writer.write_u8(self.primitive_alpha_input_type)?;
        writer.write_u8(self.shader_type)?;
        writer.write_u8(self.apply_alpha)?;
        writer.write_u8(self.is_distortion_by_camera_distance)?;
        writer.write_u8(self.padding1)?;
        writer.write_u32_le(self.padding2)?;
        Ok(())
    }
}

impl EmitterCombinerV36 {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.color_combiner_process)?;
        writer.write_u8(self.alpha_combiner_process)?;
        writer.write_u8(self.texture1_color_blend)?;
        writer.write_u8(self.texture2_color_blend)?;
        writer.write_u8(self.primitive_color_blend)?;
        writer.write_u8(self.texture1_alpha_blend)?;
        writer.write_u8(self.texture2_alpha_blend)?;
        writer.write_u8(self.primitive_alpha_blend)?;
        Ok(())
    }
}

impl ShaderRefInfo {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        writer.write_u8(self.type_)?;
        writer.write_u8(self.val_0x2)?;
        writer.write_u8(self.val_0x3)?;
        writer.write_u8(self.val_0x4)?;
        writer.write_i32_le(self.shader_index)?;
        writer.write_i32_le(self.compute_shader_index)?;
        writer.write_i32_le(self.user_shader_index1)?;
        writer.write_i32_le(self.user_shader_index2)?;
        writer.write_i32_le(self.custom_shader_index)?;
        if version_check(Some((VersionCompare::Less, 50)), version) {
            writer.write_u64_le(self.custom_shader_flag.unwrap_or(0))?;
            writer.write_u64_le(self.custom_shader_switch.unwrap_or(0))?;
        }
        if version_check(Some((VersionCompare::Less, 22)), version) {
            writer.write_u64_le(self.unknown1)?;
        }
        writer.write_i32_le(self.extra_shader_index2)?;
        writer.write_i32_le(self.val_0x34)?;
        if version_check(Some((VersionCompare::GreaterOrEqual, 50)), version) {
            writer.write_u64_le(self.unknown2)?;
        }
        writer.write_bytes(&self.user_shader_define1)?;
        writer.write_bytes(&self.user_shader_define2)?;
        Ok(())
    }
}

impl ActionInfo {
    pub fn write<W: WriterExt>(&self, writer: &mut W, version: u16) -> io::Result<()> {
        writer.write_u32_le(self.action_index)?;
        if version_check(Some((VersionCompare::Greater, 40)), version) {
            if let Some(values) = &self.unknown {
                for v in values {
                    writer.write_u32_le(*v)?;
                }
            } else {
                for _ in 0..5 {
                    writer.write_u32_le(0)?;
                }
            }
        }
        Ok(())
    }
}

impl ParticleVelocityInfo {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_f32_le(self.all_direction)?;
        writer.write_f32_le(self.designated_dir_scale)?;
        writer.write_f32_le(self.designated_dir_x)?;
        writer.write_f32_le(self.designated_dir_y)?;
        writer.write_f32_le(self.designated_dir_z)?;
        writer.write_f32_le(self.diffusion_dir_angle)?;
        writer.write_f32_le(self.xz_diffusion)?;
        writer.write_f32_le(self.diffusion_x)?;
        writer.write_f32_le(self.diffusion_y)?;
        writer.write_f32_le(self.diffusion_z)?;
        writer.write_f32_le(self.vel_random)?;
        writer.write_f32_le(self.em_vel_inherit)?;
        Ok(())
    }
}

impl ParticleColor {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.is_soft_particle)?;
        writer.write_u8(self.is_fresnel_alpha)?;
        writer.write_u8(self.is_near_dist_alpha)?;
        writer.write_u8(self.is_far_dist_alpha)?;
        writer.write_u8(self.is_decal)?;
        writer.write_u8(self.val_0x5)?;
        writer.write_u8(self.val_0x6)?;
        writer.write_u8(self.val_0x7)?;
        writer.write_u8(self.color0_type.as_u8())?;
        writer.write_u8(self.color1_type.as_u8())?;
        writer.write_u8(self.alpha0_type.as_u8())?;
        writer.write_u8(self.alpha1_type.as_u8())?;
        writer.write_f32_le(self.color0_r)?;
        writer.write_f32_le(self.color0_g)?;
        writer.write_f32_le(self.color0_b)?;
        writer.write_f32_le(self.alpha0)?;
        writer.write_f32_le(self.color1_r)?;
        writer.write_f32_le(self.color1_g)?;
        writer.write_f32_le(self.color1_b)?;
        writer.write_f32_le(self.alpha1)?;
        Ok(())
    }
}

impl ParticleScale {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_f32_le(self.scale_x)?;
        writer.write_f32_le(self.scale_y)?;
        writer.write_f32_le(self.scale_z)?;
        writer.write_f32_le(self.scale_random_x)?;
        writer.write_f32_le(self.scale_random_y)?;
        writer.write_f32_le(self.scale_random_z)?;
        writer.write_u8(self.enable_scaling_by_camera_dist_near)?;
        writer.write_u8(self.enable_scaling_by_camera_dist_far)?;
        writer.write_u8(self.enable_add_scale_y)?;
        writer.write_u8(self.enable_link_fovy_to_scale_value)?;
        writer.write_f32_le(self.scale_min)?;
        writer.write_f32_le(self.scale_max)?;
        Ok(())
    }
}

impl ParticleFlucInfo {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.is_apply_alpha)?;
        writer.write_u8(self.is_applay_scale)?;
        writer.write_u8(self.is_applay_scale_y)?;
        writer.write_u8(self.is_wave_type)?;
        writer.write_u8(self.is_phase_random_x)?;
        writer.write_u8(self.is_phase_random_y)?;
        writer.write_u8(self.padding1)?;
        writer.write_u8(self.padding2)?;
        writer.write_u32_le(self.padding3)?;
        Ok(())
    }
}

impl EmitterAnimation {
    pub fn write<W: WriterExt>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_bytes(&self.data)?;
        Ok(())
    }
}

impl EmitterData {
    pub fn write(&self, version: u16) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        let has_lt_40 = version_check(Some((VersionCompare::Less, 40)), version);
        let has_eq_36 = version_check(Some((VersionCompare::Equals, 36)), version);
        let has_gt_40 = version_check(Some((VersionCompare::Greater, 40)), version);
        let has_ge_36 = version_check(Some((VersionCompare::GreaterOrEqual, 36)), version);
        let has_ge_22 = version_check(Some((VersionCompare::GreaterOrEqual, 22)), version);

        buf.write_u32_le(self.flag)?;
        buf.write_u32_le(self.random_seed)?;
        buf.write_u32_le(self.padding1)?;
        buf.write_u32_le(self.padding2)?;

        if has_lt_40 {
            buf.write_fixed_string(self.name.as_deref().unwrap_or(""), 64)?;
        } else {
            buf.write_fixed_string(
                self.namev40
                    .as_deref()
                    .or(self.name.as_deref())
                    .unwrap_or(""),
                96,
            )?;
        }

        self.emitter_static.write(&mut buf, version)?;
        self.emitter_info.write(&mut buf)?;
        self.child_inheritance.write(&mut buf, version)?;
        self.emission.write(&mut buf)?;
        self.shape_info.write(&mut buf, version)?;
        self.render_state.write(&mut buf)?;
        self.particle_data.write(&mut buf, version)?;

        if has_lt_40 && !has_ge_36 {
            if let Some(EmitterCombinerVariant::Legacy(c)) = &self.combiner {
                c.write(&mut buf)?;
            }
        } else if has_eq_36 {
            if let Some(EmitterCombinerVariant::V36(c)) = &self.combiner {
                c.write(&mut buf)?;
            }
        } else if has_gt_40 {
            if let Some(EmitterCombinerVariant::V40(c)) = &self.combiner {
                c.write_combiner_body(&mut buf, version)?;
            }
        }

        self.shader_references.write(&mut buf, version)?;
        self.action.write(&mut buf, version)?;

        if has_gt_40 {
            buf.write_fixed_string(self.depth_mode.as_deref().unwrap_or(""), 16)?;
            buf.write_fixed_string(self.pass_info.as_deref().unwrap_or(""), 52)?;
        }

        self.particle_velocity.write(&mut buf)?;

        if has_ge_36 {
            if let Some(values) = &self.unknown_v36 {
                for v in values {
                    buf.write_f32_le(*v)?;
                }
            } else {
                for _ in 0..4 {
                    buf.write_f32_le(0.0)?;
                }
            }
        }

        self.particle_color.write(&mut buf)?;
        self.particle_scale.write(&mut buf)?;
        if let Some(fluc) = &self.particle_fluctuation {
            fluc.write(&mut buf)?;
        } else {
            ParticleFlucInfo::default_write(&mut buf)?;
        }

        if let Some(s) = &self.sampler0 {
            s.write(&mut buf, version)?;
        }
        if let Some(s) = &self.sampler1 {
            s.write(&mut buf, version)?;
        }
        if let Some(s) = &self.sampler2 {
            s.write(&mut buf, version)?;
        }
        if has_gt_40 {
            if let Some(s) = &self.sampler3 {
                s.write(&mut buf, version)?;
            }
            if let Some(s) = &self.sampler4 {
                s.write(&mut buf, version)?;
            }
            if let Some(s) = &self.sampler5 {
                s.write(&mut buf, version)?;
            }
        }

        if let Some(a) = &self.texture_anim0 {
            a.write(&mut buf)?;
        }
        if let Some(a) = &self.texture_anim1 {
            a.write(&mut buf)?;
        }
        if let Some(a) = &self.texture_anim2 {
            a.write(&mut buf)?;
        }
        if has_gt_40 {
            if let Some(a) = &self.texture_anim3 {
                a.write(&mut buf)?;
            }
            if let Some(a) = &self.texture_anim4 {
                a.write(&mut buf)?;
            }
            if let Some(a) = &self.texture_anim5 {
                a.write(&mut buf)?;
            }
        }

        if has_ge_22 {
            let reserved = if self.reserved.len() >= 0x40 {
                &self.reserved[..0x40]
            } else {
                &self.reserved
            };
            if reserved.len() < 0x40 {
                buf.write_bytes(reserved)?;
                buf.write_bytes(&vec![0u8; 0x40 - reserved.len()])?;
            } else {
                buf.write_bytes(reserved)?;
            }
        }

        Ok(buf)
    }
}

impl CombinedEmitterCombinerV40 {
    pub fn write_combiner_body<W: WriterExt>(
        &self,
        writer: &mut W,
        version: u16,
    ) -> io::Result<()> {
        writer.write_u8(self.color_combiner_process)?;
        writer.write_u8(self.alpha_combiner_process)?;
        writer.write_u8(self.texture1_color_blend)?;
        writer.write_u8(self.texture2_color_blend)?;
        writer.write_u8(self.primitive_color_blend)?;
        writer.write_u8(self.texture1_alpha_blend)?;
        writer.write_u8(self.texture2_alpha_blend)?;
        writer.write_u8(self.primitive_alpha_blend)?;
        writer.write_u8(self.tex_color0_input_type)?;
        writer.write_u8(self.tex_color1_input_type)?;
        writer.write_u8(self.tex_color2_input_type)?;
        writer.write_u8(self.tex_alpha0_input_type)?;
        writer.write_u8(self.tex_alpha1_input_type)?;
        writer.write_u8(self.tex_alpha2_input_type)?;
        writer.write_u8(self.primitive_color_input_type)?;
        writer.write_u8(self.primitive_alpha_input_type)?;
        if version >= 50 {
            writer.write_i16_le(self.padding.unwrap_or(0))?;
            writer.write_u32_le(self.padding2_opt.unwrap_or(0))?;
            writer.write_u32_le(self.padding3.unwrap_or(0))?;
        }
        Ok(())
    }
}

impl ParticleFlucInfo {
    pub fn default_write<W: WriterExt>(writer: &mut W) -> io::Result<()> {
        for _ in 0..8 {
            writer.write_u8(0)?;
        }
        writer.write_u32_le(0)?;
        Ok(())
    }
}
