use indexmap::IndexMap;

#[derive(Debug, Clone, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Debug, Clone)]
pub struct Matrix3x4 {
    pub values: [f32; 12],
}

#[derive(Debug, Clone, Default)]
pub struct Bounding {
    pub center: Vec3,
    pub extent: Vec3,
}

#[derive(Debug, Clone, Default)]
pub struct SubMesh {
    pub offset: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct KeyShape {
    pub target_attrib_indices: [u8; 20],
    pub target_attrib_index_offsets: [u8; 4],
}

#[derive(Debug, Clone, Default)]
pub struct VertexAttrib {
    pub name: String,
    pub buffer_index: u8,
    pub offset: u16,
    pub format: u16,
}

#[derive(Debug, Clone, Default)]
pub struct BufferSize {
    pub size: u32,
    pub flag: u32,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct VertexBufferSize {
    pub size: u32,
    pub gpu_access_flags: u32,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct VertexBufferStride {
    pub stride: u32,
}

#[derive(Debug, Clone, Default)]
pub struct VertexBuffer {
    pub flags: u32,
    pub attributes: IndexMap<String, VertexAttrib>,
    pub buffers: Vec<Vec<u8>>,
    pub buffer_strides: Vec<u32>,
    pub buffer_sizes: Vec<u32>,
    pub buffer_gpu_flags: Vec<u32>,
    pub buffer_offset: u32,
    pub vertex_count: u32,
    pub vertex_skin_count: u16,
    pub gpu_buffer_alignment: u16,
    /// Preserved FVTX unk block (`Buffers.len() * 72` bytes).
    pub buffer_unk_data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub primitive_type: u32,
    pub index_format: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub sub_meshes: Vec<SubMesh>,
    pub index_data: Vec<u8>,
    pub index_flag: u32,
    pub face_buffer_offset: u32,
    /// When true, PrimitiveType/IndexFormat are serialized as u16 (C# Write(..., true)).
    pub compact_enums: bool,
    /// The mesh's "buffer unk" block, captured verbatim, or empty when the source had none.
    ///
    /// The writer used to fabricate a fresh 72-byte zero block for EVERY mesh and point the
    /// header slot at it. Game containers do not, which is where our rebuilds gained exactly
    /// 72 bytes per mesh (Kirby +2808 over 39, Bomberman +432 over 6) and why every downstream
    /// offset shifted.
    pub buffer_unk_data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub flags: u32,
    pub name: String,
    pub vertex_buffer_index: u16,
    pub material_index: u16,
    pub bone_index: u16,
    pub vertex_skin_count: u8,
    pub target_attrib_count: u8,
    pub meshes: Vec<Mesh>,
    pub skin_bone_indices: Vec<u16>,
    pub key_shapes: IndexMap<String, KeyShape>,
    pub sub_mesh_boundings: Vec<Bounding>,
    pub radius_array: Vec<f32>,
    pub bounding_radius_list: Vec<Vec4>,
}

#[derive(Debug, Clone, Default)]
pub struct ResString {
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct ShaderAssign {
    /// False when the material's `FMAT+0x20` pointer was null in the source.
    ///
    /// A null pointer and an all-default block are different on disk but both used to load as
    /// `ShaderAssign::default()`, so the writer synthesised a 72-byte block for materials that
    /// never had one — shifting the string pool and everything after it.
    pub present: bool,
    pub shader_archive_name: String,
    pub shading_model_name: String,
    pub revision: u32,
    pub attrib_assigns: IndexMap<String, ResString>,
    pub sampler_assigns: IndexMap<String, ResString>,
    pub shader_options: IndexMap<String, ResString>,
}

#[derive(Debug, Clone)]
pub enum RenderInfoValue {
    Int32(Vec<i32>),
    Single(Vec<f32>),
    String(Vec<String>),
}

#[derive(Debug, Clone, Default)]
pub struct RenderInfo {
    pub name: String,
    pub info_type: u16,
    pub value: Option<RenderInfoValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Sampler {
    pub name: String,
    pub wrap_u: u8,
    pub wrap_v: u8,
    pub wrap_w: u8,
    pub compare_func: u8,
    pub border_color_type: u8,
    pub anisotropic: u8,
    pub filter_flags: u16,
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ShaderParam {
    pub name: String,
    pub param_type: u16,
    pub data_offset: u16,
    pub depended_index: u16,
    pub depend_index: u16,
    /// Exact on-disk header bytes for round-trip fidelity.
    pub header_raw: [u8; 32],
}

#[derive(Debug, Clone)]
pub enum UserDataValue {
    Int32(Vec<i32>),
    Single(Vec<f32>),
    String(Vec<String>),
    WString(Vec<String>),
    Byte(Vec<u8>),
}

#[derive(Debug, Clone, Default)]
pub struct UserData {
    pub name: String,
    pub data_type: u8,
    pub value: Option<UserDataValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Material {
    pub flags: u32,
    pub name: String,
    pub render_infos: IndexMap<String, RenderInfo>,
    pub shader_assign: ShaderAssign,
    pub texture_refs: Vec<String>,
    pub samplers: IndexMap<String, Sampler>,
    pub shader_params: IndexMap<String, ShaderParam>,
    pub shader_param_data: Vec<u8>,
    pub volatile_flags: Vec<u8>,
    pub user_data: IndexMap<String, UserData>,
    pub sampler_slot_array: Vec<i64>,
    pub texture_slot_array: Vec<i64>,
    /// Preserved texture unk1 blob (`texture_refs.len() * 8` bytes).
    pub tex_unk1_data: Vec<u8>,
    /// Preserved texture unk2 blob (`texture_refs.len() * 120` bytes).
    pub tex_unk2_data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Bone {
    pub name: String,
    pub parent_index: i16,
    pub smooth_matrix_index: i16,
    pub rigid_matrix_index: i16,
    pub billboard_index: i16,
    pub flags: u32,
    pub scale: Vec3,
    pub rotation: Vec4,
    pub position: Vec3,
    pub user_data: IndexMap<String, UserData>,
}

#[derive(Debug, Clone, Default)]
pub struct Skeleton {
    pub flags: u32,
    pub bones: IndexMap<String, Bone>,
    pub matrix_to_bone_list: Vec<u16>,
    pub inverse_model_matrices: Vec<Matrix3x4>,
    pub mirrored_bone_indices: Vec<u16>,
    pub num_smooth_matrices: u16,
    pub num_rigid_matrices: u16,
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub flags: u32,
    pub name: String,
    pub path: String,
    pub skeleton: Skeleton,
    pub vertex_buffers: Vec<VertexBuffer>,
    pub shapes: IndexMap<String, Shape>,
    pub materials: IndexMap<String, Material>,
    pub user_data: IndexMap<String, UserData>,
}

#[derive(Debug, Clone, Default)]
pub struct ExternalFile {
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct ResFileData {
    pub name: String,
    pub version_major: u32,
    pub version_minor: u32,
    pub version_minor2: u32,
    pub alignment: u8,
    pub flag: u16,
    pub block_offset: u16,
    pub target_address_size: u8,
    pub external_flag: u8,
    pub reserve10: u8,
    /// The header's own "offset to file name" field (0x10), preserved verbatim.
    ///
    /// Game-authored effect containers leave this ZERO; the writer used to patch in a pointer
    /// into its string pool, which is a value the source never had. Round-tripping it is part
    /// of getting `canonicalize` to reproduce the input byte for byte, which is the only local
    /// proof of a correct writer that does not depend on trusting another implementation.
    pub file_name_offset_field: u32,
    /// True when [`Self::file_name_offset_field`] came from a real file rather than a default.
    pub has_file_name_offset_field: bool,
    /// First word of the inline buffer-info block, preserved from the source.
    ///
    /// The writer hardcoded 34; game containers vary (`ef_return.eff` carries 36). It sits
    /// immediately before the GPU region's declared size, so a wrong value here was the first
    /// byte to diverge in an otherwise faithful round-trip.
    pub buffer_info_unk: u32,
    /// True when [`Self::buffer_info_unk`] came from a real file rather than a default.
    pub has_buffer_info_unk: bool,
    pub data_alignment_override: u32,
    pub models: IndexMap<String, Model>,
    pub external_files: IndexMap<String, ExternalFile>,
    pub string_table_strings: Vec<String>,
    /// When true, write shader param headers verbatim for round-trip fidelity.
    pub preserve_shader_param_headers: bool,
}

impl ResFileData {
    pub fn encode_version(&self) -> u32 {
        let major2 = if self.version_major == 0 { 5 } else { 0 };
        super::common::encode_version(
            self.version_major,
            major2,
            self.version_minor,
            self.version_minor2,
        )
    }

    pub fn set_version_from_u32(&mut self, version: u32) {
        let (major, minor, minor2) = super::common::decode_version(version);
        self.version_major = major;
        self.version_minor = minor;
        self.version_minor2 = minor2;
    }

    pub fn data_alignment(&self) -> usize {
        if self.data_alignment_override != 0 {
            return self.data_alignment_override as usize;
        }
        super::common::data_alignment(self.alignment)
    }

    pub fn collect_string_keys(&self) -> Vec<String> {
        self.string_table_strings.clone()
    }
}
