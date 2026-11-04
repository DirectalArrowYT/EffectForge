use indexmap::IndexMap;

use super::common::{BfresError, BfresResult, BinReader};
use super::dict::load_dict_keys;
use super::types::*;

pub struct LoadCtx<'a> {
    pub reader: BinReader<'a>,
    pub buffer_offset: i64,
    pub version_major: u32,
    /// First word of the inline buffer-info block, captured so the writer can echo it back.
    pub buffer_info_unk: Option<u32>,
}

impl<'a> LoadCtx<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: BinReader::new(data),
            buffer_offset: 0,
            version_major: 0,
            buffer_info_unk: None,
        }
    }

    fn file_data_offset(&self, relative: u32) -> usize {
        // BfresLibrary / Syroot Mesh use `(uint)BufferInfo.BufferOffset + relative` for pool data.
        self.buffer_offset as u32 as usize + relative as usize
    }

    fn read_list<T, F>(&mut self, count: usize, offset: u64, mut load: F) -> BfresResult<Vec<T>>
    where
        F: FnMut(&mut Self) -> BfresResult<T>,
    {
        if count == 0 || offset == 0 {
            return Ok(Vec::new());
        }
        if offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(load(self)?);
        }
        self.reader.seek(resume)?;
        Ok(items)
    }

    fn load_dict_values<T, F>(
        &mut self,
        dict_offset: u64,
        values_offset: u64,
        mut load: F,
    ) -> BfresResult<IndexMap<String, T>>
    where
        F: FnMut(&mut Self) -> BfresResult<T>,
    {
        if dict_offset == 0 {
            return Ok(IndexMap::new());
        }
        if dict_offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: dict_offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(dict_offset as usize)?;
        let keys = load_dict_keys(&mut self.reader)?;
        let values = self.read_list(keys.len(), values_offset, &mut load)?;
        self.reader.seek(resume)?;
        Ok(Self::zip_keys_values(keys, values))
    }

    fn load_dict_values_inline<T, F>(&mut self, load: F) -> BfresResult<IndexMap<String, T>>
    where
        F: FnMut(&mut Self) -> BfresResult<T>,
    {
        let values_offset = self.reader.read_switch_offset()?;
        let dict_offset = self.reader.read_switch_offset()?;
        self.load_dict_values(dict_offset, values_offset, load)
    }

    fn load_dict_keys_after_offset(&mut self) -> BfresResult<Vec<String>> {
        let dict_offset = self.reader.read_switch_offset()?;
        if dict_offset == 0 {
            return Ok(Vec::new());
        }
        if dict_offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: dict_offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(dict_offset as usize)?;
        let keys = load_dict_keys(&mut self.reader)?;
        self.reader.seek(resume)?;
        Ok(keys)
    }

    fn zip_keys_values<T>(keys: Vec<String>, values: Vec<T>) -> IndexMap<String, T> {
        keys.into_iter().zip(values).collect()
    }

    fn load_strings(&mut self, count: usize, offset: u64) -> BfresResult<Vec<String>> {
        if count == 0 || offset == 0 {
            return Ok(Vec::new());
        }
        if offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let strings = (0..count)
            .map(|_| self.reader.read_string_ref())
            .collect::<BfresResult<Vec<_>>>()?;
        self.reader.seek(resume)?;
        Ok(strings)
    }

    fn load_i64s(&mut self, count: usize, offset: u64) -> BfresResult<Vec<i64>> {
        if count == 0 || offset == 0 {
            return Ok(Vec::new());
        }
        if offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let values = (0..count)
            .map(|_| self.reader.read_i64())
            .collect::<BfresResult<Vec<_>>>()?;
        self.reader.seek(resume)?;
        Ok(values)
    }

    fn read_boundings(&mut self, count: usize) -> BfresResult<Vec<Bounding>> {
        (0..count)
            .map(|_| {
                Ok(Bounding {
                    center: Vec3 {
                        x: self.reader.read_f32()?,
                        y: self.reader.read_f32()?,
                        z: self.reader.read_f32()?,
                    },
                    extent: Vec3 {
                        x: self.reader.read_f32()?,
                        y: self.reader.read_f32()?,
                        z: self.reader.read_f32()?,
                    },
                })
            })
            .collect()
    }

    fn read_matrix3x4(&mut self) -> BfresResult<Matrix3x4> {
        let mut values = [0f32; 12];
        for v in &mut values {
            *v = self.reader.read_f32()?;
        }
        Ok(Matrix3x4 { values })
    }

    pub fn load_res_file(&mut self) -> BfresResult<ResFileData> {
        let magic = self.reader.read_bytes(4)?;
        if magic != b"FRES" {
            return Err(BfresError::InvalidMagic);
        }
        let _padding = self.reader.read_u32()?;
        let version = self.reader.read_u32()?;
        let byte_order = self.reader.read_u16()?;
        let alignment = self.reader.read_u8()?;
        let target_address_size = self.reader.read_u8()?;
        let offset_to_file_name = self.reader.read_u32()?;
        let flag = self.reader.read_u16()?;
        let block_offset = self.reader.read_u16()?;
        let _relocation_table_offset = self.reader.read_u32()?;
        let _siz_file = self.reader.read_u32()?;

        let mut file = ResFileData {
            alignment,
            flag,
            block_offset,
            target_address_size,
            file_name_offset_field: offset_to_file_name,
            has_file_name_offset_field: true,
            ..Default::default()
        };
        let _ = byte_order;
        file.set_version_from_u32(version);
        self.version_major = file.version_major;

        file.name = self.reader.read_string_ref()?;
        let model_offset = self.reader.read_switch_offset()?;
        let model_dict_offset = self.reader.read_switch_offset()?;

        if file.version_major >= 9 {
            self.reader.read_bytes(32)?;
        }

        for _ in 0..5 {
            self.reader.read_switch_offset()?;
            self.reader.read_switch_offset()?;
        }

        self.reader.read_switch_offset()?; // memory pool offset
        let buffer_info_offset = self.reader.read_switch_offset()?;
        self.load_buffer_info_at(buffer_info_offset)?;
        if let Some(unk) = self.buffer_info_unk {
            file.buffer_info_unk = unk;
            file.has_buffer_info_unk = true;
        }

        let _external_values = self.reader.read_switch_offset()?;
        let _external_dict = self.reader.read_switch_offset()?;
        self.reader.read_i64()?;
        let string_table_offset = self.reader.read_switch_offset()?;
        let resume = self.reader.pos;
        if string_table_offset != 0 {
            self.reader.seek(string_table_offset as usize)?;
            self.load_string_table(&mut file)?;
        }
        self.reader.seek(resume)?;
        let _string_pool_size = self.reader.read_u32()?;
        let _num_model = self.reader.read_u16()?;

        if file.version_major >= 9 {
            let unk1 = self.reader.read_u16()?;
            let unk2 = self.reader.read_u16()?;
            if unk1 != 0 || unk2 != 0 {
                return Err(BfresError::InvalidData("unexpected unk sections".into()));
            }
        }

        let _num_skeletal = self.reader.read_u16()?;
        let _num_material_anim = self.reader.read_u16()?;
        let _num_bone_vis = self.reader.read_u16()?;
        let _num_shape_anim = self.reader.read_u16()?;
        let _num_scene_anim = self.reader.read_u16()?;
        let _num_external = self.reader.read_u16()?;
        file.external_flag = self.reader.read_u8()?;
        file.reserve10 = self.reader.read_u8()?;
        self.reader.read_u32()?;

        if file.reserve10 == 1 || file.external_flag != 0 {
            file.data_alignment_override = 0x1000;
        }

        let model_keys = self.load_dict_keys_at(model_dict_offset)?;
        for model_index in 0..model_keys.len() {
            if let Ok((name, model)) =
                self.load_model_by_index(model_dict_offset, model_offset, model_index)
            {
                file.models.insert(name, model);
            }
        }

        Ok(file)
    }

    fn load_dict_keys_at(&mut self, dict_offset: u64) -> BfresResult<Vec<String>> {
        if dict_offset == 0 {
            return Ok(Vec::new());
        }
        if dict_offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: dict_offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(dict_offset as usize)?;
        let keys = load_dict_keys(&mut self.reader)?;
        self.reader.seek(resume)?;
        Ok(keys)
    }

    pub fn load_model_by_index(
        &mut self,
        model_dict_offset: u64,
        model_array_offset: u64,
        model_index: usize,
    ) -> BfresResult<(String, Model)> {
        let keys = self.load_dict_keys_at(model_dict_offset)?;
        if model_index >= keys.len() {
            return Err(BfresError::InvalidData(format!(
                "model index {model_index} out of range (count {})",
                keys.len()
            )));
        }
        if model_array_offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: model_array_offset as usize,
            });
        }
        self.reader.seek(model_array_offset as usize)?;
        for _ in 0..model_index {
            self.load_model_inner()?;
        }
        let model = self.load_model_inner()?;
        Ok((keys[model_index].clone(), model))
    }

    fn prepare_for_export(&mut self) -> BfresResult<(ResFileData, u64, u64, usize)> {
        let magic = self.reader.read_bytes(4)?;
        if magic != b"FRES" {
            return Err(BfresError::InvalidMagic);
        }
        let _padding = self.reader.read_u32()?;
        let version = self.reader.read_u32()?;
        let byte_order = self.reader.read_u16()?;
        let alignment = self.reader.read_u8()?;
        let target_address_size = self.reader.read_u8()?;
        let offset_to_file_name = self.reader.read_u32()?;
        let flag = self.reader.read_u16()?;
        let block_offset = self.reader.read_u16()?;
        let _relocation_table_offset = self.reader.read_u32()?;
        let _siz_file = self.reader.read_u32()?;

        let mut file = ResFileData {
            alignment,
            flag,
            block_offset,
            target_address_size,
            file_name_offset_field: offset_to_file_name,
            has_file_name_offset_field: true,
            ..Default::default()
        };
        let _ = byte_order;
        file.set_version_from_u32(version);
        self.version_major = file.version_major;

        file.name = self.reader.read_string_ref()?;
        let model_offset = self.reader.read_switch_offset()?;
        let model_dict_offset = self.reader.read_switch_offset()?;

        if file.version_major >= 9 {
            self.reader.read_bytes(32)?;
        }

        for _ in 0..5 {
            self.reader.read_switch_offset()?;
            self.reader.read_switch_offset()?;
        }

        self.reader.read_switch_offset()?;
        let buffer_info_offset = self.reader.read_switch_offset()?;
        self.load_buffer_info_at(buffer_info_offset)?;
        if let Some(unk) = self.buffer_info_unk {
            file.buffer_info_unk = unk;
            file.has_buffer_info_unk = true;
        }

        let _external_values = self.reader.read_switch_offset()?;
        let _external_dict = self.reader.read_switch_offset()?;
        self.reader.read_i64()?;
        let string_table_offset = self.reader.read_switch_offset()?;
        let resume = self.reader.pos;
        if string_table_offset != 0 {
            self.reader.seek(string_table_offset as usize)?;
            self.load_string_table(&mut file)?;
        }
        self.reader.seek(resume)?;
        let _string_pool_size = self.reader.read_u32()?;
        let _num_model = self.reader.read_u16()?;
        let export_resume = self.reader.pos;

        Ok((file, model_dict_offset, model_offset, export_resume))
    }

    fn finish_export_file(
        &mut self,
        file: &mut ResFileData,
        export_resume: usize,
    ) -> BfresResult<()> {
        self.reader.seek(export_resume)?;

        if file.version_major >= 9 {
            let unk1 = self.reader.read_u16()?;
            let unk2 = self.reader.read_u16()?;
            if unk1 != 0 || unk2 != 0 {
                return Err(BfresError::InvalidData("unexpected unk sections".into()));
            }
        }

        let _num_skeletal = self.reader.read_u16()?;
        let _num_material_anim = self.reader.read_u16()?;
        let _num_bone_vis = self.reader.read_u16()?;
        let _num_shape_anim = self.reader.read_u16()?;
        let _num_scene_anim = self.reader.read_u16()?;
        let _num_external = self.reader.read_u16()?;
        file.external_flag = self.reader.read_u8()?;
        file.reserve10 = self.reader.read_u8()?;
        self.reader.read_u32()?;

        if file.reserve10 == 1 || file.external_flag != 0 {
            file.data_alignment_override = 0x1000;
        }

        Ok(())
    }

    pub fn load_res_file_for_export(
        &mut self,
        model_index: usize,
    ) -> BfresResult<(ResFileData, String, Model)> {
        let (mut file, model_dict_offset, model_offset, export_resume) =
            self.prepare_for_export()?;

        let (model_name, model) =
            self.load_model_by_index(model_dict_offset, model_offset, model_index)?;

        self.reader.seek(export_resume)?;
        self.finish_export_file(&mut file, export_resume)?;

        Ok((file, model_name, model))
    }

    fn load_buffer_info_at(&mut self, offset: u64) -> BfresResult<()> {
        if offset == 0 {
            return Ok(());
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        self.buffer_info_unk = Some(self.reader.read_u32()?);
        let _size = self.reader.read_u32()?;
        self.buffer_offset = self.reader.read_i64()?;
        self.reader.read_bytes(16)?;
        self.reader.seek(resume)?;
        Ok(())
    }

    fn load_string_table(&mut self, file: &mut ResFileData) -> BfresResult<()> {
        if self.reader.pos < 0x14 {
            return Err(BfresError::InvalidData("string table underflow".into()));
        }
        self.reader.seek(self.reader.pos - 0x14)?;
        let _sig = self.reader.read_u32()?;
        let _block_offset = self.reader.read_u32()?;
        let _block_size = self.reader.read_i64()?;
        let string_count = self.reader.read_u32()?;
        for _ in 0..=string_count {
            let _size = self.reader.read_u16()?;
            let start = self.reader.pos;
            while self.reader.pos < self.reader.data.len() && self.reader.data[self.reader.pos] != 0
            {
                self.reader.pos += 1;
            }
            let s = String::from_utf8_lossy(&self.reader.data[start..self.reader.pos]).into_owned();
            file.string_table_strings.push(s);
            self.reader.pos += 1;
            self.reader.align(2)?;
        }
        Ok(())
    }

    fn load_model_inner(&mut self) -> BfresResult<Model> {
        self.reader.read_bytes(4)?; // FMDL
        let mut model = Model::default();
        if self.version_major >= 9 {
            model.flags = self.reader.read_u32()?;
        } else {
            self.reader.read_u32()?;
            self.reader.read_i64()?;
        }

        model.name = self.reader.read_string_ref()?;
        model.path = self.reader.read_string_ref()?;
        model.skeleton = self
            .load_skeleton_from_offset()
            .map_err(|e| BfresError::InvalidData(format!("skeleton: {e}")))?;
        let vertex_array_offset = self.reader.read_switch_offset()?;
        model.shapes = self
            .load_dict_values_inline(|ctx| ctx.load_shape())
            .map_err(|e| BfresError::InvalidData(format!("shapes: {e}")))?;

        let material_values_offset = self.reader.read_switch_offset()?;
        let material_dict_offset = if self.version_major == 9 {
            let off = self.reader.read_switch_offset()?;
            let dict = self.reader.read_switch_offset()?;
            if dict == 0 {
                off
            } else {
                dict
            }
        } else {
            self.reader.read_switch_offset()?
        };
        if self.version_major >= 10 {
            self.reader.read_switch_offset()?;
        }
        model.materials = self
            .load_dict_values(material_dict_offset, material_values_offset, |ctx| {
                ctx.load_material()
            })
            .map_err(|e| BfresError::InvalidData(format!("materials: {e}")))?;
        model.user_data = self
            .load_dict_values_inline(|ctx| ctx.load_user_data())
            .map_err(|e| BfresError::InvalidData(format!("user_data: {e}")))?;
        let _user_pointer = self.reader.read_switch_offset()?;

        let num_vertex_buffer = self.reader.read_u16()?;
        let _num_shape = self.reader.read_u16()?;
        let _num_material = self.reader.read_u16()?;
        if self.version_major >= 9 {
            self.reader.read_u16()?;
            self.reader.read_u16()?;
            self.reader.read_u16()?;
            self.reader.read_u32()?;
        } else {
            self.reader.read_u16()?;
            self.reader.read_u32()?;
            self.reader.read_u32()?;
        }

        model.vertex_buffers = self
            .read_list(num_vertex_buffer as usize, vertex_array_offset, |ctx| {
                ctx.load_vertex_buffer()
            })
            .map_err(|e| {
                BfresError::InvalidData(format!(
                    "vertex_buffers (count={num_vertex_buffer}, offset=0x{vertex_array_offset:x}): {e}"
                ))
            })?;
        Ok(model)
    }

    fn load_skeleton_from_offset(&mut self) -> BfresResult<Skeleton> {
        let offset = self.reader.read_switch_offset()?;
        if offset == 0 {
            return Ok(Skeleton::default());
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let skel = self.load_skeleton()?;
        self.reader.seek(resume)?;
        Ok(skel)
    }

    fn load_skeleton(&mut self) -> BfresResult<Skeleton> {
        self.reader.read_bytes(4)?;
        let mut skl = Skeleton::default();
        if self.version_major >= 9 {
            skl.flags = self.reader.read_u32()?;
        } else {
            self.reader.read_u32()?;
            self.reader.read_switch_offset()?;
        }
        let bone_dict_offset = self.reader.read_switch_offset()?;
        let bone_array_offset = self.reader.read_switch_offset()?;
        skl.bones =
            self.load_dict_values(bone_dict_offset, bone_array_offset, |ctx| ctx.load_bone())?;
        let matrix_offset = self.reader.read_switch_offset()?;
        let inverse_offset = self.reader.read_switch_offset()?;
        self.reader.read_i64()?;
        let mirror_offset = if self.version_major >= 9 {
            match self.reader.read_i64()? as u32 {
                0 | u32::MAX => 0,
                raw => u64::from(raw),
            }
        } else {
            0
        };
        if self.version_major == 8 {
            self.reader.read_bytes(16)?;
        }
        if self.version_major < 9 {
            skl.flags = self.reader.read_u32()?;
        }
        let num_bone = self.reader.read_u16()?;
        skl.num_smooth_matrices = self.reader.read_u16()?;
        skl.num_rigid_matrices = self.reader.read_u16()?;
        self.reader.read_bytes(6)?;

        if mirror_offset != 0 {
            self.reader.seek(mirror_offset as usize)?;
            skl.mirrored_bone_indices = (0..num_bone as usize)
                .map(|_| self.reader.read_u16())
                .collect::<BfresResult<Vec<_>>>()?;
        }
        if matrix_offset != 0 {
            self.reader.seek(matrix_offset as usize)?;
            let count = skl.num_smooth_matrices as usize + skl.num_rigid_matrices as usize;
            skl.matrix_to_bone_list = (0..count)
                .map(|_| self.reader.read_u16())
                .collect::<BfresResult<Vec<_>>>()?;
        }
        if inverse_offset != 0 {
            self.reader.seek(inverse_offset as usize)?;
            skl.inverse_model_matrices = (0..skl.num_smooth_matrices as usize)
                .map(|_| self.read_matrix3x4())
                .collect::<BfresResult<Vec<_>>>()?;
        }
        Ok(skl)
    }

    fn load_bone(&mut self) -> BfresResult<Bone> {
        let mut bone = Bone {
            name: self.reader.read_string_ref()?,
            ..Default::default()
        };
        let _user_data_offset = self.reader.read_switch_offset()?;
        let _user_data_dict = self.reader.read_switch_offset()?;
        if self.version_major > 9 {
            self.reader.read_bytes(8)?;
        } else if self.version_major == 8 || self.version_major == 9 {
            self.reader.read_bytes(16)?;
        }
        let _idx = self.reader.read_u16()?;
        bone.parent_index = self.reader.read_i16()?;
        bone.smooth_matrix_index = self.reader.read_i16()?;
        bone.rigid_matrix_index = self.reader.read_i16()?;
        bone.billboard_index = self.reader.read_i16()?;
        let _num_user = self.reader.read_u16()?;
        bone.flags = self.reader.read_u32()?;
        bone.scale = Vec3 {
            x: self.reader.read_f32()?,
            y: self.reader.read_f32()?,
            z: self.reader.read_f32()?,
        };
        bone.rotation = Vec4 {
            x: self.reader.read_f32()?,
            y: self.reader.read_f32()?,
            z: self.reader.read_f32()?,
            w: self.reader.read_f32()?,
        };
        bone.position = Vec3 {
            x: self.reader.read_f32()?,
            y: self.reader.read_f32()?,
            z: self.reader.read_f32()?,
        };
        Ok(bone)
    }

    fn load_shape(&mut self) -> BfresResult<Shape> {
        self.reader.read_bytes(4)?; // FSHP
        let mut shape = Shape::default();
        if self.version_major >= 9 {
            shape.flags = self.reader.read_u32()?;
        } else {
            self.reader.read_u32()?;
            self.reader.read_switch_offset()?;
        }
        shape.name = self.reader.read_string_ref()?;
        let vertex_buffer_offset = self.reader.read_switch_offset()?;
        if vertex_buffer_offset != 0 {
            let resume = self.reader.pos;
            self.reader.seek(vertex_buffer_offset as usize)?;
            let _ = self.load_vertex_buffer()?;
            self.reader.seek(resume)?;
        }
        let mesh_offset = self.reader.read_switch_offset()?;
        let skin_offset = self.reader.read_switch_offset()?;
        let _key_shapes = self.load_dict_values_inline(|_| Ok(()))?;
        let bounding_offset = self.reader.read_switch_offset()?;
        let radius_offset = if self.version_major > 2 {
            self.reader.read_switch_offset()?
        } else {
            0
        };
        if self.version_major > 2 {
            self.reader.read_i64()?;
        } else {
            shape.radius_array.push(self.reader.read_f32()?);
        }
        if self.version_major < 9 {
            shape.flags = self.reader.read_u32()?;
        }
        let _idx = self.reader.read_u16()?;
        shape.material_index = self.reader.read_u16()?;
        shape.bone_index = self.reader.read_u16()?;
        shape.vertex_buffer_index = self.reader.read_u16()?;
        let num_skin = self.reader.read_u16()?;
        shape.vertex_skin_count = self.reader.read_u8()?;
        let num_mesh = self.reader.read_u8()?;
        let num_keys = self.reader.read_u8()?;
        shape.target_attrib_count = self.reader.read_u8()?;
        if self.version_major <= 2 || self.version_major >= 9 {
            self.reader.read_u16()?;
        } else {
            self.reader.read_bytes(6)?;
        }

        let header_end = self.reader.pos;

        if radius_offset != 0 && num_mesh > 0 {
            self.reader.seek(radius_offset as usize)?;
            if self.version_major >= 10 {
                let num_boundings = if num_skin == 0 {
                    num_mesh as usize
                } else {
                    num_skin as usize
                };
                for _ in 0..num_boundings {
                    shape.bounding_radius_list.push(Vec4 {
                        x: self.reader.read_f32()?,
                        y: self.reader.read_f32()?,
                        z: self.reader.read_f32()?,
                        w: self.reader.read_f32()?,
                    });
                }
                let max = shape
                    .bounding_radius_list
                    .iter()
                    .map(|v| v.w)
                    .fold(0f32, f32::max);
                shape.radius_array.push(max);
            } else {
                for _ in 0..num_mesh {
                    shape.radius_array.push(self.reader.read_f32()?);
                }
            }
        }

        shape.meshes = self.read_list(num_mesh as usize, mesh_offset, |ctx| ctx.load_mesh())?;
        if skin_offset != 0 && num_skin > 0 {
            self.reader.seek(skin_offset as usize)?;
            shape.skin_bone_indices = (0..num_skin as usize)
                .map(|_| self.reader.read_u16())
                .collect::<BfresResult<Vec<_>>>()?;
        }
        if bounding_offset != 0 {
            let bounding_count = shape.meshes.iter().map(|m| m.sub_meshes.len() + 1).sum();
            self.reader.seek(bounding_offset as usize)?;
            shape.sub_mesh_boundings = self.read_boundings(bounding_count)?;
        }
        self.reader.seek(header_end)?;
        let _ = num_keys;
        Ok(shape)
    }

    fn load_mesh(&mut self) -> BfresResult<Mesh> {
        let sub_mesh_offset = self.reader.read_switch_offset()?;
        self.reader.read_switch_offset()?; // memory pool
        let buffer_unk_offset = self.reader.read_switch_offset()?;
        let buffer_size_offset = self.reader.read_switch_offset()?;
        let buffer_size = self.load_buffer_size_at(buffer_size_offset)?;
        let face_buffer_offset = self.reader.read_u32()?;
        let (primitive_type, index_format, index_count, first_vertex, num_sub_mesh, compact_enums) =
            self.read_mesh_header_fields(buffer_size.size)?;
        self.reader.read_u16()?; // padding

        let sub_meshes = self.read_list(num_sub_mesh as usize, sub_mesh_offset, |ctx| {
            Ok(SubMesh {
                offset: ctx.reader.read_u32()?,
                count: ctx.reader.read_u32()?,
            })
        })?;

        let data_offset = self.file_data_offset(face_buffer_offset);
        if data_offset > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: data_offset,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(data_offset)?;
        let index_data = self.reader.read_bytes(buffer_size.size as usize)?;
        self.reader.seek(resume)?;

        let buffer_unk_data = if buffer_unk_offset != 0 {
            self.load_bytes_at(buffer_unk_offset, 72)?
        } else {
            Vec::new()
        };

        Ok(Mesh {
            buffer_unk_data,
            primitive_type,
            index_format,
            index_count,
            first_vertex,
            sub_meshes,
            index_data,
            index_flag: buffer_size.flag,
            face_buffer_offset,
            compact_enums,
        })
    }

    fn read_mesh_header_fields(
        &mut self,
        index_buffer_size: u32,
    ) -> BfresResult<(u32, u32, u32, u32, u16, bool)> {
        let start = self.reader.pos;
        if let Some(fields) = self.try_read_mesh_header_fields(start, true, index_buffer_size) {
            return Ok(fields);
        }
        if let Some(fields) = self.try_read_mesh_header_fields(start, false, index_buffer_size) {
            return Ok(fields);
        }
        Err(BfresError::InvalidData(
            "mesh header fields could not be parsed".into(),
        ))
    }

    fn try_read_mesh_header_fields(
        &mut self,
        start: usize,
        compact_enums: bool,
        index_buffer_size: u32,
    ) -> Option<(u32, u32, u32, u32, u16, bool)> {
        self.reader.seek(start).ok()?;
        let (primitive_type, index_format) = if compact_enums {
            (
                self.reader.read_u16().ok()? as u32,
                self.reader.read_u16().ok()? as u32,
            )
        } else {
            (self.reader.read_u32().ok()?, self.reader.read_u32().ok()?)
        };
        let index_count = self.reader.read_u32().ok()?;
        let first_vertex = self.reader.read_u32().ok()?;
        let num_sub_mesh = self.reader.read_u16().ok()?;

        if primitive_type > 0x10 {
            return None;
        }
        let format_size = match index_format {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => return None,
        };
        if index_count == 0 || index_count.saturating_mul(format_size) != index_buffer_size {
            return None;
        }
        if num_sub_mesh == 0 {
            return None;
        }

        Some((
            primitive_type,
            index_format,
            index_count,
            first_vertex,
            num_sub_mesh,
            compact_enums,
        ))
    }

    fn load_bytes_at(&mut self, offset: u64, len: usize) -> BfresResult<Vec<u8>> {
        if offset == 0 || len == 0 {
            return Ok(Vec::new());
        }
        if offset as usize > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: offset as usize,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let data = self.reader.read_bytes(len)?;
        self.reader.seek(resume)?;
        Ok(data)
    }

    fn load_buffer_size_at(&mut self, offset: u64) -> BfresResult<BufferSize> {
        if offset == 0 {
            return Ok(BufferSize::default());
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let size = self.reader.read_u32()?;
        let flag = self.reader.read_u32()?;
        self.reader.read_bytes(40)?;
        self.reader.seek(resume)?;
        Ok(BufferSize { size, flag })
    }

    fn load_vertex_buffer(&mut self) -> BfresResult<VertexBuffer> {
        self.reader.read_bytes(4)?; // FVTX
        let mut vb = VertexBuffer::default();
        if self.version_major >= 9 {
            vb.flags = self.reader.read_u32()?;
        } else {
            self.reader.read_u32()?;
            self.reader.read_i64()?;
        }
        vb.attributes = self.load_dict_values_inline(|ctx| ctx.load_vertex_attrib())?;
        let _memory_pool = self.reader.read_switch_offset()?;
        let unk_offset = self.reader.read_switch_offset()?;
        if self.version_major > 2 {
            self.reader.read_switch_offset()?;
        }
        let buffer_size_offset = self.reader.read_switch_offset()?;
        let stride_offset = self.reader.read_switch_offset()?;
        self.reader.read_i64()?;
        vb.buffer_offset = self.reader.read_u32()?;
        let _num_attrib = self.reader.read_u8()?;
        let num_buffer = self.reader.read_u8()?;
        self.reader.read_u16()?;
        vb.vertex_count = self.reader.read_u32()?;
        vb.vertex_skin_count = self.reader.read_u16()?;
        if self.version_major >= 10 {
            vb.gpu_buffer_alignment = self.reader.read_u16()?;
        } else {
            self.reader.read_u16()?;
        }

        let strides = self.read_list(num_buffer as usize, stride_offset, |ctx| {
            let stride = ctx.reader.read_u32()?;
            ctx.reader.read_bytes(12)?;
            Ok(stride)
        })?;
        let sizes = self.read_list(num_buffer as usize, buffer_size_offset, |ctx| {
            let size = ctx.reader.read_u32()?;
            let flags = ctx.reader.read_u32()?;
            ctx.reader.read_bytes(8)?;
            Ok((size, flags))
        })?;

        for (stride, (size, flags)) in strides.into_iter().zip(sizes) {
            vb.buffer_strides.push(stride);
            vb.buffer_sizes.push(size);
            vb.buffer_gpu_flags.push(flags);
        }

        vb.buffer_unk_data = self.load_bytes_at(unk_offset, num_buffer as usize * 72)?;

        let vb_data_offset = self.file_data_offset(vb.buffer_offset);
        if vb_data_offset > self.reader.data.len() {
            return Err(BfresError::InvalidOffset {
                offset: vb_data_offset,
            });
        }
        let resume = self.reader.pos;
        self.reader.seek(vb_data_offset)?;
        for i in 0..num_buffer as usize {
            let align = if vb.gpu_buffer_alignment != 0 {
                vb.gpu_buffer_alignment as usize
            } else {
                8
            };
            self.reader.align(align)?;
            let size = vb.buffer_sizes.get(i).copied().unwrap_or(0);
            vb.buffers.push(self.reader.read_bytes(size as usize)?);
        }
        self.reader.seek(resume)?;
        Ok(vb)
    }

    fn load_vertex_attrib(&mut self) -> BfresResult<VertexAttrib> {
        let name = self.reader.read_string_ref()?;
        let format = self.reader.read_u16()?;
        self.reader.read_u16()?;
        let offset = self.reader.read_u16()?;
        let buffer_index = self.reader.read_u16()? as u8;
        Ok(VertexAttrib {
            name,
            buffer_index,
            offset,
            format,
        })
    }

    fn file_offset_from_i64(value: i64) -> u64 {
        match value as u32 {
            0 | u32::MAX => 0,
            raw => u64::from(raw),
        }
    }

    fn load_material(&mut self) -> BfresResult<Material> {
        self.load_material_inner()
            .map_err(|e| BfresError::InvalidData(format!("load_material: {e}")))
    }

    fn load_material_inner(&mut self) -> BfresResult<Material> {
        self.reader.read_bytes(4)?; // FMAT
        let mut mat = Material::default();
        if self.version_major >= 9 {
            mat.flags = self.reader.read_u32()?;
        } else {
            self.reader.read_u32()?;
            self.reader.read_i64()?;
        }
        mat.name = self.reader.read_string_ref()?;

        if self.version_major >= 10 {
            return self.load_material_v10(mat);
        }

        // Switch MaterialParser field order (BfresLibrary/Switch/Model/MaterialParser.cs).
        mat.render_infos = self
            .load_dict_values_inline(|ctx| ctx.load_render_info())
            .map_err(|e| BfresError::InvalidData(format!("render_infos: {e}")))?;
        mat.shader_assign = self
            .load_shader_assign()
            .map_err(|e| BfresError::InvalidData(format!("shader_assign: {e}")))?;
        let tex_unk1_offset = Self::file_offset_from_i64(self.reader.read_i64()?);
        let texture_name_array = Self::file_offset_from_i64(self.reader.read_i64()?);
        let tex_unk2_offset = Self::file_offset_from_i64(self.reader.read_i64()?);
        mat.samplers = self
            .load_dict_values_inline(|ctx| ctx.load_sampler())
            .map_err(|e| BfresError::InvalidData(format!("samplers: {e}")))?;
        mat.shader_params = self
            .load_dict_values_inline(|ctx| ctx.load_shader_param())
            .map_err(|e| BfresError::InvalidData(format!("shader_params: {e}")))?;
        let source_param_offset = Self::file_offset_from_i64(self.reader.read_i64()?);
        mat.user_data = self.load_dict_values_inline(|ctx| ctx.load_user_data())?;
        let volatile_flags_offset = Self::file_offset_from_i64(self.reader.read_i64()?);
        let _user_pointer = self.reader.read_i64()?;
        let sampler_slot_array_offset = Self::file_offset_from_i64(self.reader.read_i64()?);
        let texture_slot_array_offset = Self::file_offset_from_i64(self.reader.read_i64()?);

        if self.version_major < 9 {
            mat.flags = self.reader.read_u32()?;
        }
        let _idx = self.reader.read_u16()?;
        let _num_render_info = self.reader.read_u16()?;
        let num_texture_ref = self.reader.read_u8()?;
        let num_sampler = self.reader.read_u8()?;
        let num_shader_param = self.reader.read_u16()?;
        let _num_shader_param_volatile = self.reader.read_u16()?;
        let shader_param_size = self.reader.read_u16()?;
        let _siz_param_raw = self.reader.read_u16()?;
        let _num_user_data = self.reader.read_u16()?;
        if self.version_major < 9 {
            self.reader.read_u32()?;
        }

        mat.texture_refs = self.load_strings(num_texture_ref as usize, texture_name_array)?;
        if source_param_offset != 0
            && shader_param_size != 0
            && (source_param_offset as usize) <= self.reader.data.len()
        {
            self.reader.seek(source_param_offset as usize)?;
            mat.shader_param_data = self.reader.read_bytes(shader_param_size as usize)?;
        }
        if volatile_flags_offset != 0 && num_shader_param > 0 {
            let volatile_len = (num_shader_param as usize).div_ceil(8);
            if (volatile_flags_offset as usize) <= self.reader.data.len() {
                self.reader.seek(volatile_flags_offset as usize)?;
                mat.volatile_flags = self.reader.read_bytes(volatile_len)?;
            }
        }
        mat.texture_slot_array =
            self.load_i64s(num_texture_ref as usize, sampler_slot_array_offset)?;
        mat.sampler_slot_array = self.load_i64s(num_sampler as usize, texture_slot_array_offset)?;
        if num_texture_ref > 0 {
            mat.tex_unk1_data =
                self.load_bytes_at(tex_unk1_offset, num_texture_ref as usize * 8)?;
            mat.tex_unk2_data =
                self.load_bytes_at(tex_unk2_offset, num_texture_ref as usize * 120)?;
        }
        Ok(mat)
    }

    fn load_material_v10(&mut self, mut mat: Material) -> BfresResult<Material> {
        let _shader_info_offset = self.reader.read_switch_offset()?;
        let _texture_array_offset = self.reader.read_switch_offset()?;
        let texture_names_offset = self.reader.read_switch_offset()?;
        let _sampler_array_offset = self.reader.read_switch_offset()?;
        let sampler_info_array = self.reader.read_switch_offset()?;
        let sampler_keys = self.load_dict_keys_after_offset()?;
        let _render_info_data = self.reader.read_switch_offset()?;
        let _render_info_counter = self.reader.read_switch_offset()?;
        let _render_info_offsets = self.reader.read_switch_offset()?;
        let shader_param_data_offset = self.reader.read_switch_offset()?;
        let _param_indices = self.reader.read_switch_offset()?;
        self.reader.read_switch_offset()?;
        let _user_data_offset = self.reader.read_switch_offset()?;
        let _user_data_keys = self.load_dict_keys_after_offset()?;
        let volatile_offset = self.reader.read_switch_offset()?;
        let _user_pointer = self.reader.read_i64()?;
        let sampler_slot_offset = self.reader.read_switch_offset()?;
        let texture_slot_offset = self.reader.read_switch_offset()?;
        let _idx = self.reader.read_u16()?;
        let num_sampler = self.reader.read_u8()?;
        let num_texture_ref = self.reader.read_u8()?;
        self.reader.read_u16()?;
        let _num_user_data = self.reader.read_u16()?;
        let shader_param_size = self.reader.read_u16()?;
        self.reader.read_u16()?;
        self.reader.read_u32()?;

        mat.texture_refs = self.load_strings(num_texture_ref as usize, texture_names_offset)?;
        let samplers = self.read_list(num_sampler as usize, sampler_info_array, |ctx| {
            ctx.load_sampler()
        })?;
        mat.samplers = Self::zip_keys_values(sampler_keys, samplers);
        if shader_param_data_offset != 0 {
            self.reader.seek(shader_param_data_offset as usize)?;
            mat.shader_param_data = self.reader.read_bytes(shader_param_size as usize)?;
        }
        if volatile_offset != 0 {
            self.reader.seek(volatile_offset as usize)?;
            mat.volatile_flags = self.reader.read_bytes(32)?;
        }
        mat.texture_slot_array = self.load_i64s(num_texture_ref as usize, sampler_slot_offset)?;
        mat.sampler_slot_array = self.load_i64s(num_sampler as usize, texture_slot_offset)?;
        Ok(mat)
    }

    fn load_shader_assign(&mut self) -> BfresResult<ShaderAssign> {
        let offset = self.reader.read_switch_offset()?;
        if offset == 0 {
            return Ok(ShaderAssign::default());
        }
        let resume = self.reader.pos;
        self.reader.seek(offset as usize)?;
        let mut sa = self.load_shader_assign_data()?;
        sa.present = true;
        self.reader.seek(resume)?;
        Ok(sa)
    }

    fn load_shader_assign_data(&mut self) -> BfresResult<ShaderAssign> {
        let shader_archive_name = self.reader.read_string_ref()?;
        let shading_model_name = self.reader.read_string_ref()?;
        let attrib_assigns = self.load_dict_values_inline(|ctx| {
            let value = ctx.reader.read_string_ref()?;
            Ok(ResString { value })
        })?;
        let sampler_assigns = self.load_dict_values_inline(|ctx| {
            let value = ctx.reader.read_string_ref()?;
            Ok(ResString { value })
        })?;
        let shader_options = self.load_dict_values_inline(|ctx| {
            let value = ctx.reader.read_string_ref()?;
            Ok(ResString { value })
        })?;
        let revision = self.reader.read_u32()?;
        let _num_attrib = self.reader.read_u8()?;
        let _num_sampler = self.reader.read_u8()?;
        let _num_option = self.reader.read_u16()?;
        Ok(ShaderAssign {
            shader_archive_name,
            shading_model_name,
            revision,
            attrib_assigns,
            sampler_assigns,
            shader_options,
            ..Default::default()
        })
    }

    fn load_render_info(&mut self) -> BfresResult<RenderInfo> {
        let name = self.reader.read_string_ref()?;
        let data_offset = self.reader.read_switch_offset()?;
        let count = self.reader.read_u16()?;
        let info_type = self.reader.read_u8()? as u16;
        self.reader.read_bytes(5)?;
        let value = if data_offset != 0 {
            let resume = self.reader.pos;
            self.reader.seek(data_offset as usize)?;
            let parsed = match info_type {
                0 => Some(RenderInfoValue::Int32(
                    (0..count as usize)
                        .map(|_| self.reader.read_i32())
                        .collect::<BfresResult<Vec<_>>>()?,
                )),
                1 => Some(RenderInfoValue::Single(
                    (0..count as usize)
                        .map(|_| self.reader.read_f32())
                        .collect::<BfresResult<Vec<_>>>()?,
                )),
                2 => Some(RenderInfoValue::String(
                    (0..count as usize)
                        .map(|_| self.reader.read_string_ref())
                        .collect::<BfresResult<Vec<_>>>()?,
                )),
                _ => None,
            };
            self.reader.seek(resume)?;
            parsed
        } else {
            None
        };
        Ok(RenderInfo {
            name,
            info_type,
            value,
        })
    }

    fn load_sampler(&mut self) -> BfresResult<Sampler> {
        let sampler = Sampler {
            wrap_u: self.reader.read_u8()?,
            wrap_v: self.reader.read_u8()?,
            wrap_w: self.reader.read_u8()?,
            compare_func: self.reader.read_u8()?,
            border_color_type: self.reader.read_u8()?,
            anisotropic: self.reader.read_u8()?,
            filter_flags: self.reader.read_u16()?,
            min_lod: self.reader.read_f32()?,
            max_lod: self.reader.read_f32()?,
            lod_bias: self.reader.read_f32()?,
            name: String::new(),
        };
        self.reader.read_bytes(12)?;
        Ok(sampler)
    }

    fn load_user_data(&mut self) -> BfresResult<UserData> {
        let name = self.reader.read_string_ref()?;
        let data_type = self.reader.read_u8()?;
        let count = self.reader.read_u16()?;
        self.reader.read_u16()?;
        let value = match data_type {
            0 => Some(UserDataValue::Int32(
                (0..count as usize)
                    .map(|_| self.reader.read_i32())
                    .collect::<BfresResult<Vec<_>>>()?,
            )),
            1 => Some(UserDataValue::Single(
                (0..count as usize)
                    .map(|_| self.reader.read_f32())
                    .collect::<BfresResult<Vec<_>>>()?,
            )),
            2 => Some(UserDataValue::String(
                (0..count as usize)
                    .map(|_| self.reader.read_string_ref())
                    .collect::<BfresResult<Vec<_>>>()?,
            )),
            3 => Some(UserDataValue::WString(
                (0..count as usize)
                    .map(|_| self.reader.read_string_ref())
                    .collect::<BfresResult<Vec<_>>>()?,
            )),
            4 => Some(UserDataValue::Byte(self.reader.read_bytes(count as usize)?)),
            _ => None,
        };
        Ok(UserData {
            name,
            data_type,
            value,
        })
    }

    fn load_shader_param(&mut self) -> BfresResult<ShaderParam> {
        let start = self.reader.pos;
        let header_raw = self
            .reader
            .read_bytes(32)?
            .try_into()
            .map_err(|_| BfresError::InvalidData("shader param header".into()))?;
        self.reader.seek(start)?;
        let callback_pointer = self.reader.read_i64()?;
        let name = self.reader.read_string_ref()?;
        let param_type = self.reader.read_u8()? as u16;
        let _siz = self.reader.read_u8()?;
        let data_offset = self.reader.read_u16()?;
        let _offset = self.reader.read_i32()?;
        let depended_index = self.reader.read_u16()?;
        let depend_index = self.reader.read_u16()?;
        self.reader.read_u32()?;
        let _ = callback_pointer;
        Ok(ShaderParam {
            name,
            param_type,
            data_offset,
            depended_index,
            depend_index,
            header_raw,
        })
    }
}

pub fn load_from_bytes(data: &[u8]) -> BfresResult<ResFileData> {
    let mut ctx = LoadCtx::new(data);
    ctx.load_res_file()
}

pub fn load_for_export(
    data: &[u8],
    model_index: usize,
) -> BfresResult<(ResFileData, String, Model)> {
    let mut ctx = LoadCtx::new(data);
    ctx.load_res_file_for_export(model_index)
}

/// Reusable BFRES export context that parses file headers once per source blob.
pub struct ResExportSession<'a> {
    pub ctx: LoadCtx<'a>,
    file: ResFileData,
    model_dict_offset: u64,
    model_offset: u64,
    export_resume: usize,
    footer_loaded: bool,
}

impl<'a> ResExportSession<'a> {
    pub fn open(data: &'a [u8]) -> BfresResult<Self> {
        let mut ctx = LoadCtx::new(data);
        let (file, model_dict_offset, model_offset, export_resume) = ctx.prepare_for_export()?;
        Ok(Self {
            ctx,
            file,
            model_dict_offset,
            model_offset,
            export_resume,
            footer_loaded: false,
        })
    }

    pub fn export_model(
        &mut self,
        model_index: usize,
    ) -> BfresResult<(ResFileData, String, Model)> {
        let (model_name, model) =
            self.ctx
                .load_model_by_index(self.model_dict_offset, self.model_offset, model_index)?;

        if !self.footer_loaded {
            self.ctx.reader.seek(self.export_resume)?;
            self.ctx
                .finish_export_file(&mut self.file, self.export_resume)?;
            self.footer_loaded = true;
        }

        Ok((self.file.clone(), model_name, model))
    }
}
