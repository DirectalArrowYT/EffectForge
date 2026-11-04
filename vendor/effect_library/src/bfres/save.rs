use super::common::{
    BinWriter, RelocationTable, StringTable, SECTION1, SECTION2, SECTION3, SECTION4, SECTION5,
};
use super::dict::save_dict;
use super::types::*;

struct ModelHeaderSlots {
    skel: usize,
    vtx: usize,
    shape: usize,
    shape_dict: usize,
    mat: usize,
    mat_dict: usize,
}

impl Clone for ModelHeaderSlots {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for ModelHeaderSlots {}

struct MaterialHeaderSlots {
    render: usize,
    render_dict: usize,
    shader_assign: usize,
    tex_unk1: usize,
    tex_refs: usize,
    tex_unk2: usize,
    samplers: usize,
    sampler_dict: usize,
    shader_params: usize,
    shader_param_dict: usize,
    shader_data: usize,
    volatile: usize,
    sampler_slot: usize,
    texture_slot: usize,
}

impl Copy for MaterialHeaderSlots {}
impl Clone for MaterialHeaderSlots {
    fn clone(&self) -> Self {
        *self
    }
}

struct SkeletonHeaderSlots {
    bone_dict: usize,
    bone_array: usize,
    matrix: usize,
    inverse: usize,
    mirror: usize,
}

impl Copy for SkeletonHeaderSlots {}
impl Clone for SkeletonHeaderSlots {
    fn clone(&self) -> Self {
        *self
    }
}

struct VertexBufferHeaderSlots {
    attr: usize,
    attr_dict: usize,
    unk: usize,
    sizes: usize,
    strides: usize,
    unk2: usize,
}

impl Copy for VertexBufferHeaderSlots {}
impl Clone for VertexBufferHeaderSlots {
    fn clone(&self) -> Self {
        *self
    }
}

pub struct SaveCtx {
    pub writer: BinWriter,
    pub rlt: RelocationTable,
    pub strings: StringTable,
    pub current_index: usize,
    pub buffer_info_offset: usize,
    pub inline_buffer_info_pos: usize,
    pub index_buffer_offset_pos: usize,
    pub vertex_buffer_offset_pos: usize,
    pub total_buffer_size_pos: usize,
    model_dict_pos: usize,
    model_offset_pos: usize,
    pub memory_pool_offset: usize,
    pub saved_memory_pool_pointers: Vec<usize>,
    pub end_of_string_table: usize,
    pub file_name_header_pos: usize,
    pub file_size_pos: usize,
    pub string_pool_pos: usize,
    pub buffer_data_start: usize,
    vertex_data_end: usize,
    pub buffer_info_header_pos: usize,
    pub buffer_offset_field_pos: usize,
    pub shape_mesh_slots: Vec<usize>,
    pub pending_skin_slots: Vec<usize>,
    pub pending_bounds_slots: Vec<usize>,
    pub pending_radius_slots: Vec<usize>,
    pending_shape_vtx_runtime_slots: Vec<usize>,
    pub mesh_buffer_size_slots: Vec<usize>,
    pub mesh_submesh_slots: Vec<usize>,
    pub mesh_unk_slots: Vec<usize>,
    model_header_slots: Vec<ModelHeaderSlots>,
    material_header_slots: Vec<MaterialHeaderSlots>,
    skeleton_header_slots: Vec<SkeletonHeaderSlots>,
    vertex_buffer_header_slots: Vec<VertexBufferHeaderSlots>,
    render_info_data_slots: Vec<usize>,
    current_model_index: usize,
    preserve_shader_param_headers: bool,
    preserve_file_name_offset: bool,
    source_file_name_offset: u32,
}

impl SaveCtx {
    pub fn new(file: &ResFileData) -> Self {
        let mut strings = StringTable::default();
        strings.collect_keys(&file.name, &file.collect_string_keys());
        Self {
            writer: BinWriter::default(),
            rlt: RelocationTable::new(5),
            strings,
            current_index: 0,
            buffer_info_offset: 0,
            inline_buffer_info_pos: 0,
            index_buffer_offset_pos: 0,
            vertex_buffer_offset_pos: 0,
            total_buffer_size_pos: 0,
            model_dict_pos: 0,
            model_offset_pos: 0,
            memory_pool_offset: 0,
            saved_memory_pool_pointers: Vec::new(),
            end_of_string_table: 0,
            file_name_header_pos: 0,
            file_size_pos: 0,
            string_pool_pos: 0,
            buffer_data_start: 0,
            vertex_data_end: 0,
            buffer_info_header_pos: 0,
            buffer_offset_field_pos: 0,
            shape_mesh_slots: Vec::new(),
            pending_skin_slots: Vec::new(),
            pending_bounds_slots: Vec::new(),
            pending_radius_slots: Vec::new(),
            pending_shape_vtx_runtime_slots: Vec::new(),
            mesh_buffer_size_slots: Vec::new(),
            mesh_submesh_slots: Vec::new(),
            mesh_unk_slots: Vec::new(),
            model_header_slots: Vec::new(),
            material_header_slots: Vec::new(),
            skeleton_header_slots: Vec::new(),
            vertex_buffer_header_slots: Vec::new(),
            render_info_data_slots: Vec::new(),
            current_model_index: 0,
            preserve_shader_param_headers: file.preserve_shader_param_headers,
            preserve_file_name_offset: file.has_file_name_offset_field,
            source_file_name_offset: file.file_name_offset_field,
        }
    }

    fn model_slots(&self) -> &ModelHeaderSlots {
        &self.model_header_slots[self.current_model_index]
    }

    fn save_string_ref(&mut self, value: &str) {
        let pos = self.writer.position();
        self.strings.add_entry(pos, value);
        self.writer.write_u64(0);
    }

    fn save_offset_slot(&mut self) -> usize {
        self.writer.save_offset()
    }

    fn write_offset(&mut self, pos: usize) {
        self.writer.write_offset(pos);
    }

    pub fn save_res_file(&mut self, file: &ResFileData) {
        // Set EFFECT_LIBRARY_TRACE_SECTIONS=1 to print where each phase ends. Comparing those
        // boundaries against a game container's landmarks localizes a layout divergence to one
        // phase in a single run, which hex-reading a shifted file does not.
        let trace = std::env::var_os("EFFECT_LIBRARY_TRACE_SECTIONS").is_some();
        macro_rules! phase {
            ($name:literal, $call:expr) => {{
                $call;
                if trace {
                    eprintln!("  {:<22} ends at {:#x}", $name, self.writer.position());
                }
            }};
        }
        phase!("header", self.write_header(file));
        phase!("buffer_info_inline", self.write_buffer_info_inline(file));
        phase!("dicts", self.write_dicts(file));
        phase!("models", self.write_models(file));
        phase!("strings", self.write_strings());
        phase!("index_buffer", self.write_index_buffer(file));
        phase!("vertex_buffer", self.write_vertex_buffer(file));
        phase!("memory_pool", self.write_memory_pool(file));
        phase!("finalize_buffer", self.finalize_buffer_section());
        self.setup_relocation_table(file);
        phase!("relocation_table", self.rlt.write(&mut self.writer));
        phase!("header_blocks", self.writer.write_header_blocks());
        self.finalize_header(file);
    }

    fn write_header(&mut self, file: &ResFileData) {
        self.writer.write_signature("FRES");
        self.writer.write_u32(0x20202020);
        self.writer.write_u32(file.encode_version());
        self.writer.write_u16(0xFEFF);
        self.writer.write_u8(file.alignment);
        self.writer.write_u8(file.target_address_size);
        self.file_name_header_pos = self.writer.position();
        self.writer.write_u32(0);
        self.writer.write_u16(file.flag);
        self.writer.save_header_block(true);
        self.rlt.save_header_offset(&mut self.writer);
        self.file_size_pos = self.writer.position();
        self.writer.write_u32(0);

        // From version 9 the header carries a 32-byte hole between the model dict and the
        // animation arrays, and it is NOT relocated. Covering the whole run with one entry marked
        // those four words as pointers and stopped short of the external-file and string-pool
        // slots at the end — so the game rebased four words of padding and left five real
        // pointers unrebased. `ef_return.eff` splits it as 3 slots, gap, then 10.
        if file.version_major >= 9 {
            self.rlt
                .save_entry(self.writer.position(), 3, 1, 0, SECTION1);
        } else {
            self.rlt
                .save_entry(self.writer.position(), 13, 1, 0, SECTION1);
        }

        self.save_string_ref(&file.name);
        let model_offset_pos = self.save_offset_slot();
        self.model_offset_pos = model_offset_pos;
        let model_dict_pos = self.save_offset_slot();

        if file.version_major >= 9 {
            self.writer.write_zeroes(32);
            self.rlt
                .save_entry(self.writer.position(), 10, 1, 0, SECTION1);
        }

        let ska_pos = self.save_offset_slot();
        let ska_dict_pos = self.save_offset_slot();
        let mat_anim_pos = self.save_offset_slot();
        let mat_anim_dict_pos = self.save_offset_slot();
        let bone_vis_pos = self.save_offset_slot();
        let bone_vis_dict_pos = self.save_offset_slot();
        let shape_anim_pos = self.save_offset_slot();
        let shape_anim_dict_pos = self.save_offset_slot();
        let scene_anim_pos = self.save_offset_slot();
        let scene_anim_dict_pos = self.save_offset_slot();

        if !file.models.is_empty() {
            self.rlt
                .save_entry(self.writer.position(), 1, 1, 0, SECTION4);
        }
        self.saved_memory_pool_pointers.push(self.writer.position());
        self.writer.write_u64(0);

        // One run of five: buffer info, external array, external dict, the reserved word, and the
        // string pool. Marking only the first and last left the three between them unrebased.
        self.rlt
            .save_entry(self.writer.position(), 5, 1, 0, SECTION1);
        self.buffer_info_header_pos = self.save_offset_slot();

        let ext_pos = self.save_offset_slot();
        let ext_dict_pos = self.save_offset_slot();
        self.writer.write_i64(0);
        self.string_pool_pos = self.writer.position();
        self.writer.write_u64(0);
        self.writer.write_u32(0);
        self.writer.write_u16(file.models.len() as u16);

        if file.version_major >= 9 {
            self.writer.write_u16(0);
            self.writer.write_u16(0);
        }

        self.writer.write_u16(0);
        self.writer.write_u16(0);
        self.writer.write_u16(0);
        self.writer.write_u16(0);
        self.writer.write_u16(0);
        self.writer.write_u16(file.external_files.len() as u16);
        if file.version_major >= 9 {
            // Write what the SOURCE had. These two bytes used to be hardcoded `0, 1`, which
            // invented a value: Nintendo's own effect containers carry 0 here, and the loader
            // treats `reserve10 == 1` as a signal to force a 0x1000 data alignment — so every
            // file we wrote came back claiming something the original never said.
            self.writer.write_u8(file.external_flag);
            self.writer.write_u8(file.reserve10);
        } else {
            self.writer.write_u8(file.external_flag);
            self.writer.write_u8(file.reserve10);
            self.writer.write_u32(0);
        }

        let _ = (
            ska_pos,
            ska_dict_pos,
            mat_anim_pos,
            mat_anim_dict_pos,
            bone_vis_pos,
            bone_vis_dict_pos,
            shape_anim_pos,
            shape_anim_dict_pos,
            scene_anim_pos,
            scene_anim_dict_pos,
            ext_pos,
            ext_dict_pos,
        );

        if !file.models.is_empty() {
            self.write_offset(self.model_offset_pos);
            for (_, model) in &file.models {
                self.save_model_header(model, file.version_major);
            }
            self.model_dict_pos = model_dict_pos;
        }
    }

    /// Write each model's subheaders immediately followed by its own block.
    ///
    /// Game containers interleave per model — `ef_bomberman.eff` repeats FVTX/FMAT/FSHP/FSKL on a
    /// fixed 1328-byte stride, one group per model. Emitting all six headers and then all six
    /// blocks produced the same total size but a different order, so every inter-model pointer
    /// landed somewhere the original never pointed.
    fn write_models(&mut self, file: &ResFileData) {
        for (i, (_, model)) in file.models.iter().enumerate() {
            self.current_model_index = i;
            self.write_model_subheaders(model, file.version_major);
            self.write_model_block(model, file.version_major);
        }
    }

    fn write_dicts(&mut self, file: &ResFileData) {
        if !file.models.is_empty() {
            self.write_offset(self.model_dict_pos);
            let keys: Vec<String> = file.models.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        let _ = file;
    }

    fn write_buffer_info_inline(&mut self, file: &ResFileData) {
        if file.models.is_empty() {
            return;
        }
        self.write_offset(self.buffer_info_header_pos);
        self.inline_buffer_info_pos = self.writer.position();
        self.total_buffer_size_pos = self.writer.position();
        // Echo the source's value; 34 is only a fallback for containers we synthesised.
        self.writer.write_u32(if file.has_buffer_info_unk {
            file.buffer_info_unk
        } else {
            34
        });
        let total_buffer_size = self.compute_buffer_total_size(file);
        self.writer.write_u32(total_buffer_size as u32);
        self.rlt
            .save_entry(self.writer.position(), 1, 1, 0, SECTION2);
        self.index_buffer_offset_pos = self.writer.position();
        self.buffer_offset_field_pos = self.writer.position();
        self.writer.write_i64(0);
        self.writer.write_zeroes(16);
    }

    fn save_model_header(&mut self, model: &Model, version_major: u32) {
        self.writer.write_signature("FMDL");
        if version_major >= 9 {
            self.writer.write_u32(model.flags);
        } else {
            self.writer.save_header_block(false);
        }

        // Like the file header, this run is broken by a padding word that must NOT be relocated,
        // and the run continues past it to the trailing reserved slot. One entry of 10 straddled
        // the padding at +0x40 and stopped two slots early.
        let first_run = match version_major {
            0..=8 => 10,
            // v9 pads between the material array and its dict; v10 pads after the dict.
            9 => 7,
            _ => 8,
        };
        self.rlt
            .save_entry(self.writer.position(), first_run, 1, 0, SECTION1);

        self.save_string_ref(&model.name);
        self.save_string_ref(&model.path);
        let skel_pos = self.save_offset_slot();
        let vtx_pos = self.save_offset_slot();
        let shape_pos = self.save_offset_slot();
        let shape_dict_pos = self.save_offset_slot();
        let mat_pos = self.save_offset_slot();
        if version_major == 9 {
            self.writer.write_u64(0);
            self.rlt
                .save_entry(self.writer.position(), 4, 1, 0, SECTION1);
        }
        let mat_dict_pos = self.save_offset_slot();
        if version_major >= 10 {
            self.writer.write_u64(0);
            self.rlt
                .save_entry(self.writer.position(), 3, 1, 0, SECTION1);
        }
        let _user_pos = self.save_offset_slot();
        let _user_dict_pos = self.save_offset_slot();
        self.writer.write_i64(0);
        self.writer.write_u16(model.vertex_buffers.len() as u16);
        self.writer.write_u16(model.shapes.len() as u16);
        self.writer.write_u16(model.materials.len() as u16);
        if version_major >= 9 {
            self.writer.write_u16(0);
            self.writer.write_u16(model.user_data.len() as u16);
            self.writer.write_u16(0);
            self.writer.write_u32(0);
        } else {
            self.writer.write_u16(model.user_data.len() as u16);
            let total_vtx = model.vertex_buffers.iter().map(|v| v.vertex_count).sum();
            self.writer.write_u32(total_vtx);
            self.writer.write_u32(0);
        }

        self.model_header_slots.push(ModelHeaderSlots {
            skel: skel_pos,
            vtx: vtx_pos,
            shape: shape_pos,
            shape_dict: shape_dict_pos,
            mat: mat_pos,
            mat_dict: mat_dict_pos,
        });
    }

    fn write_model_subheaders(&mut self, model: &Model, version_major: u32) {
        let slots = *self.model_slots();
        let mut vtx_header_positions = Vec::new();
        if !model.vertex_buffers.is_empty() {
            self.write_offset(slots.vtx);
            for (i, vb) in model.vertex_buffers.iter().enumerate() {
                self.current_index = i;
                vtx_header_positions.push(self.writer.position());
                self.save_vertex_buffer_header(vb, version_major);
            }
        }
        if !model.materials.is_empty() {
            self.write_offset(slots.mat);
            for (i, (_, mat)) in model.materials.iter().enumerate() {
                self.current_index = i;
                self.save_material_header(mat, version_major);
            }
        }
        if !model.shapes.is_empty() {
            self.write_offset(slots.shape);
            let shape_vtx_indices: Vec<u16> = model
                .shapes
                .values()
                .map(|shape| shape.vertex_buffer_index)
                .collect();
            for (i, (_, shape)) in model.shapes.iter().enumerate() {
                self.current_index = i;
                self.save_shape_header(shape, version_major);
            }
            for (slot, vtx_index) in self
                .pending_shape_vtx_runtime_slots
                .drain(..)
                .zip(shape_vtx_indices)
            {
                let target = vtx_header_positions
                    .get(vtx_index as usize)
                    .copied()
                    .unwrap_or(0);
                if target != 0 {
                    self.writer.write_offset_to(slot, target);
                }
            }
        } else {
            self.pending_shape_vtx_runtime_slots.clear();
        }
        if !model.skeleton.bones.is_empty() {
            self.write_offset(slots.skel);
            self.save_skeleton_header(&model.skeleton, version_major);
        }
    }

    fn write_model_block(&mut self, model: &Model, version_major: u32) {
        let slots = *self.model_slots();
        if !model.skeleton.bones.is_empty() {
            self.rlt.save_entry(
                self.writer.position(),
                3,
                model.skeleton.bones.len() as u32,
                if version_major >= 10 {
                    8
                } else if version_major >= 8 {
                    9
                } else {
                    7
                },
                SECTION1,
            );
            self.write_skeleton_block(&model.skeleton, version_major);
        }
        if !model.shapes.is_empty() {
            self.write_offset(slots.shape_dict);
            let keys: Vec<String> = model.shapes.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        if !model.materials.is_empty() {
            self.write_offset(slots.mat_dict);
            let keys: Vec<String> = model.materials.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        for (_, shape) in &model.shapes {
            self.write_shapes(model, shape, version_major);
        }
        for vb in &model.vertex_buffers {
            self.write_vertex_buffer_block(vb, version_major);
        }
        for (_, mat) in &model.materials {
            self.write_material_block(mat, version_major);
        }
    }

    fn save_skeleton_header(&mut self, skel: &Skeleton, version_major: u32) {
        self.writer.write_signature("FSKL");
        if version_major >= 9 {
            self.writer.write_u32(skel.flags);
        } else {
            self.writer.save_header_block(false);
        }
        // From v9 the reserved word that follows the four pointers is relocated with them.
        self.rlt.save_entry(
            self.writer.position(),
            if version_major >= 9 { 5 } else { 4 },
            1,
            0,
            SECTION1,
        );
        let bone_dict = self.save_offset_slot();
        let bone_array = self.save_offset_slot();
        let matrix = self.save_offset_slot();
        let inverse = self.save_offset_slot();
        if version_major == 8 {
            self.writer.write_zeroes(16);
        }
        if version_major >= 9 {
            self.writer.write_u64(0);
        }
        self.rlt
            .save_entry(self.writer.position(), 1, 1, 0, SECTION1);
        let mirror = self.save_offset_slot();
        if version_major < 9 {
            self.writer.write_u32(skel.flags);
        }
        self.writer.write_u16(skel.bones.len() as u16);
        self.writer
            .write_u16(skel.inverse_model_matrices.len() as u16);
        self.writer.write_u16(skel.num_rigid_matrices);
        if version_major >= 9 {
            self.writer.write_u16(0);
        } else {
            self.writer.write_zeroes(6);
        }
        self.skeleton_header_slots.push(SkeletonHeaderSlots {
            bone_dict,
            bone_array,
            matrix,
            inverse,
            mirror,
        });
    }

    fn write_skeleton_block(&mut self, skel: &Skeleton, version_major: u32) {
        let slots = self.skeleton_header_slots.remove(0);
        self.write_offset(slots.bone_array);
        for (i, (_, bone)) in skel.bones.iter().enumerate() {
            self.current_index = i;
            self.save_bone_header(bone, version_major);
        }

        if !skel.matrix_to_bone_list.is_empty() {
            self.writer.align_bytes(8);
            self.write_offset(slots.matrix);
            for v in &skel.matrix_to_bone_list {
                self.writer.write_u16(*v);
            }
        }
        if !skel.inverse_model_matrices.is_empty() {
            self.writer.align_bytes(8);
            self.write_offset(slots.inverse);
            for m in &skel.inverse_model_matrices {
                for v in &m.values {
                    self.writer.write_f32(*v);
                }
            }
        }
        if !skel.mirrored_bone_indices.is_empty() {
            self.writer.align_bytes(8);
            self.write_offset(slots.mirror);
            for v in &skel.mirrored_bone_indices {
                self.writer.write_u16(*v);
            }
        }
        self.write_offset(slots.bone_dict);
        let keys: Vec<String> = skel.bones.keys().cloned().collect();
        save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
    }

    fn save_bone_header(&mut self, bone: &Bone, version_major: u32) {
        self.save_string_ref(&bone.name);
        let _user = self.save_offset_slot();
        let _user_dict = self.save_offset_slot();
        if version_major > 9 {
            self.writer.write_zeroes(8);
        } else if version_major == 8 || version_major == 9 {
            self.writer.write_zeroes(16);
        }
        self.writer.write_u16(self.current_index as u16);
        self.writer.write_i16(bone.parent_index);
        self.writer.write_i16(bone.smooth_matrix_index);
        self.writer.write_i16(bone.rigid_matrix_index);
        self.writer.write_i16(bone.billboard_index);
        self.writer.write_u16(bone.user_data.len() as u16);
        self.writer.write_u32(bone.flags);
        self.writer.write_f32(bone.scale.x);
        self.writer.write_f32(bone.scale.y);
        self.writer.write_f32(bone.scale.z);
        self.writer.write_f32(bone.rotation.x);
        self.writer.write_f32(bone.rotation.y);
        self.writer.write_f32(bone.rotation.z);
        self.writer.write_f32(bone.rotation.w);
        self.writer.write_f32(bone.position.x);
        self.writer.write_f32(bone.position.y);
        self.writer.write_f32(bone.position.z);
    }

    fn save_shape_header(&mut self, shape: &Shape, version_major: u32) {
        self.writer.write_signature("FSHP");
        if version_major >= 9 {
            self.writer.write_u32(shape.flags);
        } else {
            self.writer.write_zeroes(12);
        }
        // 9: the eight pointers plus the trailing reserved word.
        self.rlt
            .save_entry(self.writer.position(), 9, 1, 0, SECTION1);
        self.save_string_ref(&shape.name);
        let vtx_runtime = self.save_offset_slot();
        self.pending_shape_vtx_runtime_slots.push(vtx_runtime);
        let mesh_slot = self.save_offset_slot();
        let skin_slot = self.save_offset_slot();
        let _keys = self.save_offset_slot();
        let _key_dict = self.save_offset_slot();
        let bounds_slot = self.save_offset_slot();
        let radius_slot = self.save_offset_slot();
        self.shape_mesh_slots.push(mesh_slot);
        self.pending_skin_slots.push(skin_slot);
        self.pending_bounds_slots.push(bounds_slot);
        self.pending_radius_slots.push(radius_slot);
        self.writer.write_i64(0);
        if version_major < 9 {
            self.writer.write_u32(shape.flags);
        }
        self.writer.write_u16(self.current_index as u16);
        self.writer.write_u16(shape.material_index);
        self.writer.write_u16(shape.bone_index);
        self.writer.write_u16(shape.vertex_buffer_index);
        self.writer.write_u16(shape.skin_bone_indices.len() as u16);
        self.writer.write_u8(shape.vertex_skin_count);
        self.writer.write_u8(shape.meshes.len() as u8);
        self.writer.write_u8(shape.key_shapes.len() as u8);
        self.writer.write_u8(shape.target_attrib_count);
        if version_major >= 9 {
            self.writer.write_u16(0);
        } else {
            self.writer.write_zeroes(6);
        }
    }

    fn write_shapes(&mut self, model: &Model, shape: &Shape, version_major: u32) {
        let mesh_header_slot = self.shape_mesh_slots.remove(0);
        let mut first_mesh_pos = 0usize;
        for (i, mesh) in shape.meshes.iter().enumerate() {
            if i == 0 {
                first_mesh_pos = self.writer.position();
            }
            self.save_mesh_header(mesh);
        }
        if !shape.meshes.is_empty() {
            self.writer
                .write_offset_to(mesh_header_slot, first_mesh_pos);
        }

        if !shape.skin_bone_indices.is_empty() {
            let skin_slot = self.pending_skin_slots.remove(0);
            let skin_start = self.writer.position();
            for v in &shape.skin_bone_indices {
                self.writer.write_u16(*v);
            }
            self.writer.write_offset_to(skin_slot, skin_start);
        } else if !self.pending_skin_slots.is_empty() {
            self.pending_skin_slots.remove(0);
        }
        if !shape.sub_mesh_boundings.is_empty() {
            let bounds_slot = self.pending_bounds_slots.remove(0);
            self.writer.align_bytes(8);
            let bounds_start = self.writer.position();
            for b in &shape.sub_mesh_boundings {
                self.writer.write_f32(b.center.x);
                self.writer.write_f32(b.center.y);
                self.writer.write_f32(b.center.z);
                self.writer.write_f32(b.extent.x);
                self.writer.write_f32(b.extent.y);
                self.writer.write_f32(b.extent.z);
            }
            self.writer.write_offset_to(bounds_slot, bounds_start);
        } else if !self.pending_bounds_slots.is_empty() {
            self.pending_bounds_slots.remove(0);
        }
        if !shape.key_shapes.is_empty() {
            // key shape blocks not needed for ef_mario primitives
        }
        if !shape.radius_array.is_empty() {
            let radius_slot = self.pending_radius_slots.remove(0);
            let radius_start = self.writer.position();
            if version_major >= 10 {
                for v in &shape.bounding_radius_list {
                    self.writer.write_f32(v.x);
                    self.writer.write_f32(v.y);
                    self.writer.write_f32(v.z);
                    self.writer.write_f32(v.w);
                }
            } else {
                for v in &shape.radius_array {
                    self.writer.write_f32(*v);
                }
            }
            self.writer.write_offset_to(radius_slot, radius_start);
        } else if !self.pending_radius_slots.is_empty() {
            self.pending_radius_slots.remove(0);
        }

        for mesh in &shape.meshes {
            self.writer.align_bytes(8);
            if !mesh.sub_meshes.is_empty() {
                let sub_slot = self.mesh_submesh_slots.remove(0);
                let sub_start = self.writer.position();
                for sm in &mesh.sub_meshes {
                    self.writer.write_u32(sm.offset);
                    self.writer.write_u32(sm.count);
                }
                self.writer.write_offset_to(sub_slot, sub_start);
            } else if !self.mesh_submesh_slots.is_empty() {
                self.mesh_submesh_slots.remove(0);
            }
            // Only emit the block when the SOURCE had one; otherwise leave the slot null, as
            // game containers do. See `Mesh::buffer_unk_data`.
            let unk_slot = self.mesh_unk_slots.remove(0);
            if !mesh.buffer_unk_data.is_empty() {
                let unk_start = self.writer.position();
                self.writer.write_bytes(&mesh.buffer_unk_data);
                self.writer.write_offset_to(unk_slot, unk_start);
            }
            let size_slot = self.mesh_buffer_size_slots.remove(0);
            let size_start = self.writer.position();
            self.writer.write_u32(mesh.index_data.len() as u32);
            self.writer.write_u32(mesh.index_flag);
            self.writer.write_zeroes(8);
            self.writer.write_offset_to(size_slot, size_start);
            let _ = model;
        }
    }

    fn save_mesh_header(&mut self, mesh: &Mesh) {
        self.rlt
            .save_entry(self.writer.position(), 1, 1, 0, SECTION1);
        let sub_slot = self.save_offset_slot();
        self.mesh_submesh_slots.push(sub_slot);
        self.rlt
            .save_entry(self.writer.position(), 1, 1, 0, SECTION4);
        self.saved_memory_pool_pointers.push(self.writer.position());
        self.writer.write_u64(0);
        self.rlt
            .save_entry(self.writer.position(), 2, 1, 0, SECTION1);
        let unk_slot = self.save_offset_slot();
        self.mesh_unk_slots.push(unk_slot);
        let size_slot = self.save_offset_slot();
        self.mesh_buffer_size_slots.push(size_slot);
        self.writer.write_u32(mesh.face_buffer_offset);
        if mesh.compact_enums {
            self.writer.write_u16(mesh.primitive_type as u16);
            self.writer.write_u16(mesh.index_format as u16);
        } else {
            self.writer.write_u32(mesh.primitive_type);
            self.writer.write_u32(mesh.index_format);
        }
        self.writer.write_u32(mesh.index_count);
        self.writer.write_u32(mesh.first_vertex);
        self.writer.write_u16(mesh.sub_meshes.len() as u16);
        self.writer.write_u16(0);
    }

    fn save_vertex_buffer_header(&mut self, vb: &VertexBuffer, version_major: u32) {
        self.writer.write_signature("FVTX");
        if version_major >= 9 {
            self.writer.write_u32(vb.flags);
        } else {
            self.writer.write_zeroes(12);
        }
        self.rlt
            .save_entry(self.writer.position(), 2, 1, 0, SECTION1);
        let attr = self.save_offset_slot();
        let attr_dict = self.save_offset_slot();
        self.rlt
            .save_entry(self.writer.position(), 1, 1, 0, SECTION4);
        self.saved_memory_pool_pointers.push(self.writer.position());
        self.writer.write_u64(0);
        // Five, not four: the trailing reserved word is relocated too, as it is in the file header.
        self.rlt
            .save_entry(self.writer.position(), 5, 1, 0, SECTION1);
        let unk = self.save_offset_slot();
        let unk2 = self.save_offset_slot();
        let sizes = self.save_offset_slot();
        let strides = self.save_offset_slot();
        self.writer.write_i64(0);
        self.writer.write_u32(vb.buffer_offset);
        self.writer.write_u8(vb.attributes.len() as u8);
        self.writer.write_u8(vb.buffers.len() as u8);
        self.writer.write_u16(self.current_index as u16);
        self.writer.write_u32(vb.vertex_count);
        self.writer.write_u16(vb.vertex_skin_count);
        if version_major >= 10 {
            self.writer.write_u16(vb.gpu_buffer_alignment);
        } else {
            self.writer.write_u16(0);
        }
        self.vertex_buffer_header_slots
            .push(VertexBufferHeaderSlots {
                attr,
                attr_dict,
                unk,
                unk2,
                sizes,
                strides,
            });
    }

    fn write_vertex_buffer_block(&mut self, vb: &VertexBuffer, version_major: u32) {
        let slots = self.vertex_buffer_header_slots.remove(0);
        if !vb.attributes.is_empty() {
            self.rlt.save_entry(
                self.writer.position(),
                1,
                vb.attributes.len() as u32,
                1,
                SECTION1,
            );
            let attr_start = self.writer.position();
            self.writer.write_offset_to(slots.attr, attr_start);
            for (_, attr) in &vb.attributes {
                self.save_vertex_attrib(attr);
            }
        }
        if !vb.buffers.is_empty() {
            // A block of its own, never the mesh's. Sharing was tried on the theory that one
            // block served both; the game's own files disprove it — `ef_return.eff` carries two
            // distinct 72-byte blocks at 0x4a0 and 0x538 and points the mesh at one, the vertex
            // buffer at the other. Sharing shifted every following offset back by 72 per model.
            let unk_start = self.writer.position();
            self.writer.write_offset_to(slots.unk, unk_start);
            if vb.buffer_unk_data.len() == vb.buffers.len() * 72 {
                self.writer.write_bytes(&vb.buffer_unk_data);
            } else {
                self.writer.write_zeroes(vb.buffers.len() * 72);
            }
            let sizes_start = self.writer.position();
            self.writer.write_offset_to(slots.sizes, sizes_start);
            for (i, _) in vb.buffers.iter().enumerate() {
                self.writer
                    .write_u32(vb.buffer_sizes.get(i).copied().unwrap_or(0));
                self.writer
                    .write_u32(vb.buffer_gpu_flags.get(i).copied().unwrap_or(0));
                self.writer.write_zeroes(8);
            }
            let strides_start = self.writer.position();
            self.writer.write_offset_to(slots.strides, strides_start);
            for stride in &vb.buffer_strides {
                self.writer.write_u32(*stride);
                self.writer.write_zeroes(12);
            }
            let unk2_start = self.writer.position();
            self.writer.write_offset_to(slots.unk2, unk2_start);
            for _ in &vb.buffers {
                self.writer.write_u64(0);
            }
        }
        if !vb.attributes.is_empty() {
            let dict_start = self.writer.position();
            self.writer.write_offset_to(slots.attr_dict, dict_start);
            let keys: Vec<String> = vb.attributes.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        let _ = version_major;
    }

    fn save_vertex_attrib(&mut self, attr: &VertexAttrib) {
        self.save_string_ref(&attr.name);
        self.writer.write_u16(attr.format);
        self.writer.write_u16(0);
        self.writer.write_u16(attr.offset);
        self.writer.write_u16(attr.buffer_index as u16);
    }

    fn save_material_header(&mut self, mat: &Material, version_major: u32) {
        self.writer.write_signature("FMAT");
        if version_major >= 9 {
            self.writer.write_u32(mat.flags);
        } else {
            self.writer.write_zeroes(12);
        }
        if version_major >= 10 {
            return;
        }
        // 16: the fifteen pointers plus the trailing reserved word.
        self.rlt
            .save_entry(self.writer.position(), 16, 1, 0, SECTION1);
        self.save_string_ref(&mat.name);
        let render = self.save_offset_slot();
        let render_dict = self.save_offset_slot();
        let shader_assign = self.save_offset_slot();
        let tex_unk1 = self.save_offset_slot();
        let tex_refs = self.save_offset_slot();
        let tex_unk2 = self.save_offset_slot();
        let samplers = self.save_offset_slot();
        let sampler_dict = self.save_offset_slot();
        let shader_params = self.save_offset_slot();
        let shader_param_dict = self.save_offset_slot();
        let shader_data = self.save_offset_slot();
        let _user = self.save_offset_slot();
        let _user_dict = self.save_offset_slot();
        let volatile = self.save_offset_slot();
        self.writer.write_i64(0);
        self.rlt
            .save_entry(self.writer.position(), 2, 1, 0, SECTION1);
        let sampler_slot = self.save_offset_slot();
        let texture_slot = self.save_offset_slot();
        if version_major != 9 {
            self.writer.write_u32(mat.flags);
        }
        self.writer.write_u16(self.current_index as u16);
        self.writer.write_u16(mat.render_infos.len() as u16);
        // C# Material.Save writes Samplers then TextureRefs (opposite of Load read order).
        self.writer.write_u8(mat.samplers.len() as u8);
        self.writer.write_u8(mat.texture_refs.len() as u8);
        self.writer.write_u16(mat.shader_params.len() as u16);
        self.writer.write_u16(0);
        self.writer.write_u16(mat.shader_param_data.len() as u16);
        self.writer.write_u16(0);
        self.writer.write_u16(mat.user_data.len() as u16);
        if version_major != 9 {
            self.writer.write_u32(0);
        }
        self.material_header_slots.push(MaterialHeaderSlots {
            render,
            render_dict,
            shader_assign,
            tex_unk1,
            tex_refs,
            tex_unk2,
            samplers,
            sampler_dict,
            shader_params,
            shader_param_dict,
            shader_data,
            volatile,
            sampler_slot,
            texture_slot,
        });
    }

    fn write_material_block(&mut self, mat: &Material, version_major: u32) {
        let slots = self.material_header_slots.remove(0);
        if !mat.render_infos.is_empty() {
            self.rlt.save_entry(
                self.writer.position(),
                2,
                mat.render_infos.len() as u32,
                1,
                SECTION1,
            );
            self.write_offset(slots.render);
            for (_, ri) in &mat.render_infos {
                self.save_render_info_header(ri);
            }
            self.write_render_info_data(mat);
        }
        if !mat.texture_refs.is_empty() {
            self.writer.align_bytes(8);
            self.write_offset(slots.tex_unk1);
            if mat.tex_unk1_data.len() == mat.texture_refs.len() * 8 {
                self.writer.write_bytes(&mat.tex_unk1_data);
            } else {
                for _ in &mat.texture_refs {
                    self.writer.write_u64(0);
                }
            }
        }
        if !mat.samplers.is_empty() {
            self.writer.align_bytes(8);
            self.write_offset(slots.samplers);
            for (_, smp) in &mat.samplers {
                self.save_sampler(smp);
            }
            self.write_offset(slots.tex_unk2);
            if mat.tex_unk2_data.len() == mat.texture_refs.len() * 120 {
                self.writer.write_bytes(&mat.tex_unk2_data);
            } else {
                self.writer.write_zeroes(mat.texture_refs.len() * 120);
            }
        }
        if !mat.shader_params.is_empty() {
            self.rlt.save_entry(
                self.writer.position() + 8,
                1,
                mat.shader_params.len() as u32,
                3,
                SECTION1,
            );
            self.write_offset(slots.shader_params);
            for (_, sp) in &mat.shader_params {
                self.save_shader_param(sp);
            }
            self.write_offset(slots.shader_data);
            self.writer.write_bytes(&mat.shader_param_data);
        }
        if !mat.volatile_flags.is_empty() {
            self.writer.align_bytes(8);
            self.write_offset(slots.volatile);
            self.writer.write_bytes(&mat.volatile_flags);
            self.writer.align_bytes(8);
        }
        if !mat.sampler_slot_array.is_empty() {
            self.write_offset(slots.sampler_slot);
            for v in &mat.sampler_slot_array {
                self.writer.write_i64(*v);
            }
        }
        if !mat.texture_slot_array.is_empty() {
            self.write_offset(slots.texture_slot);
            for v in &mat.texture_slot_array {
                self.writer.write_i64(*v);
            }
        }
        if !mat.texture_refs.is_empty() {
            self.rlt.save_entry(
                self.writer.position(),
                mat.texture_refs.len() as u32,
                1,
                0,
                SECTION1,
            );
            self.write_offset(slots.tex_refs);
            for tex in &mat.texture_refs {
                self.save_string_ref(tex);
            }
        }
        self.write_shader_assign_block(&mat.shader_assign, slots.shader_assign);
        if !mat.render_infos.is_empty() {
            self.write_offset(slots.render_dict);
            let keys: Vec<String> = mat.render_infos.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        if !mat.samplers.is_empty() {
            self.write_offset(slots.sampler_dict);
            let keys: Vec<String> = mat.samplers.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        if !mat.shader_params.is_empty() {
            self.write_offset(slots.shader_param_dict);
            let keys: Vec<String> = mat.shader_params.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        let _ = version_major;
    }

    fn save_render_info_header(&mut self, ri: &RenderInfo) {
        self.save_string_ref(&ri.name);
        let data_pos = self.save_offset_slot();
        self.render_info_data_slots.push(data_pos);
        let count = match &ri.value {
            Some(RenderInfoValue::Int32(v)) => v.len(),
            Some(RenderInfoValue::Single(v)) => v.len(),
            Some(RenderInfoValue::String(v)) => v.len(),
            None => 0,
        };
        self.writer.write_u16(count as u16);
        self.writer.write_u8(ri.info_type as u8);
        self.writer.write_zeroes(5);
    }

    fn write_render_info_data(&mut self, mat: &Material) {
        let slots: Vec<usize> = std::mem::take(&mut self.render_info_data_slots);
        let infos: Vec<&RenderInfo> = mat.render_infos.values().collect();
        let string_count: u32 = infos
            .iter()
            .filter_map(|ri| {
                if let Some(RenderInfoValue::String(v)) = &ri.value {
                    Some(v.len() as u32)
                } else {
                    None
                }
            })
            .sum();
        if string_count > 0 {
            self.rlt
                .save_entry(self.writer.position(), string_count, 1, 0, SECTION1);
        }
        for (ri, slot) in infos.iter().zip(&slots) {
            if let Some(RenderInfoValue::String(v)) = &ri.value {
                if !v.is_empty() {
                    self.write_offset(*slot);
                }
                for s in v {
                    self.save_string_ref(s);
                }
            }
        }
        for (ri, slot) in infos.iter().zip(&slots) {
            if let Some(RenderInfoValue::Int32(v)) = &ri.value {
                if !v.is_empty() {
                    self.write_offset(*slot);
                }
                for i in v {
                    self.writer.write_i32(*i);
                }
            }
        }
        for (ri, slot) in infos.iter().zip(&slots) {
            if let Some(RenderInfoValue::Single(v)) = &ri.value {
                if !v.is_empty() {
                    self.write_offset(*slot);
                }
                for f in v {
                    self.writer.write_f32(*f);
                }
            }
        }
    }

    fn save_sampler(&mut self, smp: &Sampler) {
        self.writer.write_u8(smp.wrap_u);
        self.writer.write_u8(smp.wrap_v);
        self.writer.write_u8(smp.wrap_w);
        self.writer.write_u8(smp.compare_func);
        self.writer.write_u8(smp.border_color_type);
        self.writer.write_u8(smp.anisotropic);
        self.writer.write_u16(smp.filter_flags);
        self.writer.write_f32(smp.min_lod);
        self.writer.write_f32(smp.max_lod);
        self.writer.write_f32(smp.lod_bias);
        self.writer.write_zeroes(12);
    }

    fn shader_param_data_size(param_type: u16) -> u8 {
        if param_type <= 0x0F {
            return (((param_type & 0x03) + 1) * 4) as u8;
        }
        if param_type <= 0x1B {
            let cols = (param_type & 0x03) + 1;
            let rows = ((param_type - 0x10) >> 2) + 2;
            return (cols * rows * 4) as u8;
        }
        match param_type {
            0x1C => 20, // Srt2D
            0x1D => 36, // Srt3D
            0x1E => 24, // TexSrt
            0x1F => 28, // TexSrtEx
            _ => 4,
        }
    }

    fn save_shader_param(&mut self, sp: &ShaderParam) {
        if sp.header_raw.iter().any(|&b| b != 0) && self.preserve_shader_param_headers {
            self.writer.write_bytes(&sp.header_raw);
            return;
        }

        self.writer.write_i64(0);
        self.save_string_ref(&sp.name);
        self.writer.write_u8(sp.param_type as u8);
        let data_size = Self::shader_param_data_size(sp.param_type);
        self.writer.write_u8(data_size);
        self.writer.write_u16(sp.data_offset);
        self.writer.write_i32(-1);
        self.writer.write_u16(sp.depended_index);
        self.writer.write_u16(sp.depend_index);
        self.writer.write_u32(0);
    }

    fn write_shader_assign_block(&mut self, sa: &ShaderAssign, header_slot: usize) {
        // Leave the header slot null when the source had none, rather than inventing a block.
        if !sa.present {
            return;
        }
        self.rlt
            .save_entry(self.writer.position(), 8, 1, 0, SECTION1);
        self.write_offset(header_slot);
        self.save_string_ref(&sa.shader_archive_name);
        self.save_string_ref(&sa.shading_model_name);
        let attrib = self.save_offset_slot();
        let attrib_dict = self.save_offset_slot();
        let sampler = self.save_offset_slot();
        let sampler_dict = self.save_offset_slot();
        let options = self.save_offset_slot();
        let options_dict = self.save_offset_slot();
        self.writer.write_u32(sa.revision);
        self.writer.write_u8(sa.attrib_assigns.len() as u8);
        self.writer.write_u8(sa.sampler_assigns.len() as u8);
        self.writer.write_u16(sa.shader_options.len() as u16);

        let data_count =
            (sa.attrib_assigns.len() + sa.sampler_assigns.len() + sa.shader_options.len()) as u32;
        self.rlt
            .save_entry(self.writer.position(), data_count, 1, 0, SECTION1);

        if !sa.attrib_assigns.is_empty() {
            self.write_offset(attrib);
            for (_, v) in &sa.attrib_assigns {
                self.save_string_ref(&v.value);
            }
        }
        if !sa.sampler_assigns.is_empty() {
            self.write_offset(sampler);
            for (_, v) in &sa.sampler_assigns {
                self.save_string_ref(&v.value);
            }
        }
        if !sa.shader_options.is_empty() {
            self.write_offset(options);
            for (_, v) in &sa.shader_options {
                self.save_string_ref(&v.value);
            }
        }
        if !sa.attrib_assigns.is_empty() {
            self.write_offset(attrib_dict);
            let keys: Vec<String> = sa.attrib_assigns.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        if !sa.sampler_assigns.is_empty() {
            self.write_offset(sampler_dict);
            let keys: Vec<String> = sa.sampler_assigns.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
        if !sa.shader_options.is_empty() {
            self.write_offset(options_dict);
            let keys: Vec<String> = sa.shader_options.keys().cloned().collect();
            save_dict(&mut self.writer, &mut self.strings, &mut self.rlt, &keys);
        }
    }

    fn write_strings(&mut self) {
        self.writer.align_bytes(4);
        let string_block_start = self.writer.position();
        self.writer.write_signature("_STR");
        self.writer.save_header_block(false);
        let keys = self.strings.pool_keys();
        // The field holds one LESS than the number of strings — `load_string_table` reads
        // `0..=count`, and the game's own tables confirm it (`ef_return.eff` stores 10 for the
        // same 11 strings we were labelling 11).
        self.writer.write_u32(keys.len().saturating_sub(1) as u32);
        self.strings.pool_start = self.writer.position();
        let mut estimated_len = 0usize;
        for key in &keys {
            estimated_len += 2 + key.len() + 1;
            if !estimated_len.is_multiple_of(2) {
                estimated_len += 1;
            }
        }
        self.strings.pool_len = estimated_len;
        self.writer.write_zeroes(estimated_len);
        self.strings
            .write_in_pool(&mut self.writer, self.file_name_header_pos);
        // Restore the header's own name-offset field if the source had one. `write_in_pool`
        // patches in a pointer into our pool, but game containers carry 0 here — inventing a
        // value is a gratuitous divergence from the only output known to work on hardware.
        if self.preserve_file_name_offset {
            self.writer
                .patch_u32(self.file_name_header_pos, self.source_file_name_offset);
        }
        self.writer.align_bytes(2);
        self.end_of_string_table = self.writer.position();
        let string_pool_size = self.end_of_string_table - string_block_start;
        self.writer
            .patch_u64(self.string_pool_pos, self.strings.pool_start as u64);
        self.writer
            .patch_u32(self.string_pool_pos + 8, string_pool_size as u32);
    }

    fn write_index_buffer(&mut self, file: &ResFileData) {
        let align = file.data_alignment();
        self.writer.align_bytes(align);
        self.buffer_info_offset = self.writer.position();
        self.buffer_data_start = self.buffer_info_offset;

        for (_, model) in &file.models {
            for (_, shape) in &model.shapes {
                for mesh in &shape.meshes {
                    if !self.writer.position().is_multiple_of(8) {
                        self.writer.align_bytes(8);
                    }
                    self.writer.write_bytes(&mesh.index_data);
                }
            }
        }
    }

    fn write_vertex_buffer(&mut self, file: &ResFileData) {
        self.vertex_buffer_offset_pos = self.writer.position();
        for (_, model) in &file.models {
            for vb in &model.vertex_buffers {
                let align = if vb.gpu_buffer_alignment != 0 {
                    vb.gpu_buffer_alignment as usize
                } else {
                    8
                };
                for buf in &vb.buffers {
                    if !self.writer.position().is_multiple_of(align) {
                        self.writer.align_bytes(align);
                    }
                    self.writer.write_bytes(buf);
                }
            }
        }
        self.vertex_data_end = self.writer.position();
        let _ = file;
    }

    fn finalize_buffer_section(&mut self) {
        if self.buffer_offset_field_pos != 0 {
            self.writer
                .patch_i64(self.buffer_offset_field_pos, self.buffer_data_start as i64);
        }
        if self.buffer_info_header_pos != 0 && self.inline_buffer_info_pos != 0 {
            self.writer.patch_u64(
                self.buffer_info_header_pos,
                self.inline_buffer_info_pos as u64,
            );
        }
    }

    fn compute_buffer_total_size(&mut self, file: &ResFileData) -> usize {
        let mut size = 0usize;
        for (_, model) in &file.models {
            for (_, shape) in &model.shapes {
                let align = shape_gpu_alignment(model, shape);
                for mesh in &shape.meshes {
                    size += mesh.index_data.len();
                    if !size.is_multiple_of(align) {
                        size += align - (size % align);
                    }
                }
            }
            for vb in &model.vertex_buffers {
                let align = if vb.gpu_buffer_alignment != 0 {
                    vb.gpu_buffer_alignment as usize
                } else {
                    8
                };
                for buf in &vb.buffers {
                    size += buf.len();
                    if !size.is_multiple_of(align) {
                        size += align - (size % align);
                    }
                }
            }
        }
        let align = file.data_alignment();
        if !size.is_multiple_of(align) {
            size += align - (size % align);
        }
        size
    }

    fn write_memory_pool(&mut self, file: &ResFileData) {
        let align = file.data_alignment();
        self.writer.align_bytes(align);
        let buffer_end = self.writer.position();
        let section2_size = buffer_end - self.buffer_info_offset;
        self.writer
            .patch_u32(self.total_buffer_size_pos + 4, section2_size as u32);
        self.memory_pool_offset = self.writer.position();
        self.writer.write_zeroes(288);
        for ptr in &self.saved_memory_pool_pointers {
            self.writer.patch_u64(*ptr, self.memory_pool_offset as u64);
        }
    }

    fn computed_index_buffer_size(file: &ResFileData) -> u32 {
        let mut size = 0u32;
        for (_, model) in &file.models {
            for (_, shape) in &model.shapes {
                let align = shape_gpu_alignment(model, shape) as u32;
                for mesh in &shape.meshes {
                    size += mesh.index_data.len() as u32;
                    size = align_up(size, align);
                }
            }
        }
        size
    }

    fn computed_vertex_buffer_size(file: &ResFileData) -> u32 {
        let mut size = 0u32;
        for (_, model) in &file.models {
            for vb in &model.vertex_buffers {
                let align = if vb.gpu_buffer_alignment != 0 {
                    vb.gpu_buffer_alignment as u32
                } else {
                    8
                };
                for buf in &vb.buffers {
                    size += buf.len() as u32;
                    size = align_up(size, align);
                }
            }
        }
        size
    }

    fn setup_relocation_table(&mut self, file: &ResFileData) {
        let section1_size = self.end_of_string_table;
        self.rlt.set_section(SECTION1, 0, section1_size as u32);

        let index_size = Self::computed_index_buffer_size(file);
        self.rlt
            .set_section(SECTION2, self.buffer_info_offset as u32, index_size);

        let vertex_size = Self::computed_vertex_buffer_size(file);
        self.rlt
            .set_section(SECTION3, self.vertex_buffer_offset_pos as u32, vertex_size);
        self.rlt
            .set_section(SECTION4, self.memory_pool_offset as u32, 288);
        let sec5_pos = if self.memory_pool_offset != 0 {
            self.memory_pool_offset as u32
        } else {
            section1_size as u32
        };
        self.rlt.set_section(SECTION5, sec5_pos, 0);
    }

    fn finalize_header(&mut self, _file: &ResFileData) {
        if self.rlt.relocation_table_offset_pos != 0 && self.rlt.written_offset != 0 {
            self.writer.patch_u32(
                self.rlt.relocation_table_offset_pos,
                self.rlt.written_offset as u32,
            );
        }
        self.writer
            .patch_u32(self.file_size_pos, self.writer.position() as u32);
    }
}

fn align_up(value: u32, align: u32) -> u32 {
    if align == 0 {
        return value;
    }
    if !value.is_multiple_of(align) {
        value + (align - (value % align))
    } else {
        value
    }
}

fn shape_gpu_alignment(model: &Model, shape: &Shape) -> usize {
    model
        .vertex_buffers
        .get(shape.vertex_buffer_index as usize)
        .map(|vb| {
            if vb.gpu_buffer_alignment != 0 {
                vb.gpu_buffer_alignment as usize
            } else {
                8
            }
        })
        .unwrap_or(8)
}

/// Alignment the GPU region requires for one vertex buffer's data.
///
/// `ResVertex + 0x56`, "alignment of vertex buffer data in GPU region". 8 is the fallback for
/// containers that leave it unset.
pub(crate) fn vertex_gpu_alignment(vb: &crate::bfres::types::VertexBuffer) -> u32 {
    if vb.gpu_buffer_alignment != 0 {
        vb.gpu_buffer_alignment as u32
    } else {
        8
    }
}

/// Record where each buffer will land in the GPU region.
///
/// These offsets are what the GPU reads through — `ResMesh + 0x20` and `ResVertex + 0x48`, both
/// relative to the region start — so they MUST agree with where [`ResFileSaver::write_index_buffer`]
/// and [`ResFileSaver::write_vertex_buffer`] actually put the bytes.
///
/// They did not. Those writers align each vertex chunk to the buffer's own
/// `gpu_buffer_alignment`, while this function recorded offsets using a hardcoded 8. For any
/// model with a larger alignment every recorded offset pointed somewhere the data was not, and
/// the mesh drew nothing.
///
/// It survived every round-trip test because the LOADER re-walks the region applying the same
/// real alignment, so it lands on the right bytes whatever the offsets say and the data looks
/// intact. Only the GPU, which trusts the offsets, sees the difference.
///
/// The region base is aligned to `data_alignment()` and every GPU alignment divides it, so
/// advancing this relative cursor is equivalent to the writers aligning their absolute
/// position.
fn assign_buffer_offsets(file: &mut ResFileData) {
    // Index data first, matching C# Mesh.SetFaceBufferOffset.
    let mut index_cursor = 0u32;
    for (_, model) in &mut file.models {
        for (_, shape) in &mut model.shapes {
            for mesh in &mut shape.meshes {
                mesh.face_buffer_offset = index_cursor;
                index_cursor += mesh.index_data.len() as u32;
                index_cursor = align_up(index_cursor, 8);
            }
        }
    }

    // Then vertex data, each chunk on its own buffer's GPU alignment.
    let mut vertex_cursor = index_cursor;
    for (_, model) in &mut file.models {
        for vb in &mut model.vertex_buffers {
            let align = vertex_gpu_alignment(vb);
            vertex_cursor = align_up(vertex_cursor, align);
            vb.buffer_offset = vertex_cursor;
            for buf in &vb.buffers {
                vertex_cursor = align_up(vertex_cursor, align);
                vertex_cursor += buf.len() as u32;
            }
        }
    }
}

pub fn save_to_bytes(file: &ResFileData) -> Vec<u8> {
    let mut file = file.clone();
    assign_buffer_offsets(&mut file);
    if file.version_major >= 9 && !file.models.is_empty() {
        // Keep the 0x1000 GPU-region alignment, but do NOT fabricate `reserve10`: it is a
        // header value belonging to the source file, and the writer now round-trips it.
        file.data_alignment_override = 0x1000;
    }
    let mut ctx = SaveCtx::new(&file);
    ctx.save_res_file(&file);
    ctx.writer.into_bytes()
}
