use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use bytemuck::{Pod, Zeroable};
use cgmath::{ElementWise, Matrix4, Quaternion, Vector3, Vector4};
use nanoem::model::ModelFormatType;
use par::shape::ShapesMesh;
use wgpu::{AddressMode, Buffer};

use crate::{
    bounding_box::BoundingBox,
    camera::Camera,
    drawable::{DrawType, Drawable},
    effect::IEffect,
    forward::LineVertexUnit,
    image_loader::Image,
    image_view::ImageView,
    internal::LinearDrawer,
    model_object_selection::ModelObjectSelection,
    model_program_bundle::{ModelProgramBundle, ObjectTechnique},
    pass,
    project::Project,
    undo::UndoStack,
    uri::Uri,
};

pub type NanoemModel = nanoem::model::Model<(), Material, (), (), (), (), (), (), ()>;
pub type NanoemBone = nanoem::model::ModelBone<(), ()>;
pub type NanoemMaterial = nanoem::model::ModelMaterial<Material>;
pub type NanoemMorph = nanoem::model::ModelMorph<()>;
pub type NanoemConstraint = nanoem::model::ModelConstraint<()>;
pub type NanoemRigidBody = nanoem::model::ModelRigidBody<()>;

pub trait SkinDeformer {
    // TODO
}

pub struct BindPose {
    // TODO
}

pub trait Gizmo {
    // TODO
}

pub trait VertexWeightPainter {
    // TODO
}

pub enum AxisType {
    None,
    Center,
    X,
    Y,
    Z,
}

pub enum EditActionType {
    None,
    SelectModelObject,
    PaintVertexWeight,
    CreateTriangleVertices,
    CreateParentBone,
    CreateTargetBone,
}

pub enum TransformCoordinateType {
    Global,
    Local,
}

pub enum ResetType {
    TranslationAxisX,
    TranslationAxisY,
    TranslationAxisZ,
    Orientation,
    OrientationAngleX,
    OrientationAngleY,
    OrientationAngleZ,
}

struct LoadingImageItem {
    file_uri: Uri,
    filename: String,
    wrap: AddressMode,
    flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VertexUnit {
    position: [f32; 4],
    normal: [f32; 4],
    texcoord: [f32; 4],
    edge: [f32; 4],
    uva: [[f32; 4]; 4],
    weights: [f32; 4],
    indices: [f32; 4],
    info: [f32; 4], /* type,vertexIndex,edgeSize,padding */
}

pub struct NewModelDescription {
    name: HashMap<nanoem::common::LanguageType, String>,
    comment: HashMap<nanoem::common::LanguageType, String>,
}

pub enum ImportFileType {
    None,
    WaveFrontObj,
    DirectX,
    Metasequoia,
}

pub struct ImportDescription {
    file_uri: Uri,
    name: HashMap<nanoem::common::LanguageType, String>,
    comment: HashMap<nanoem::common::LanguageType, String>,
    transform: Matrix4<f32>,
    file_type: ImportFileType,
}

pub struct ExportDescription {
    transform: Matrix4<f32>,
}

struct ParallelSkinningTaskData {
    draw_type: DrawType,
    edge_size_scale_factor: f32,
    bone_indices: HashMap<Rc<RefCell<NanoemMaterial>>, HashMap<i32, i32>>,
    output: u8,
    materials: Rc<RefCell<[NanoemMaterial]>>,
    vertices: Rc<RefCell<[NanoemMaterial]>>,
    num_vertices: usize,
}

struct DrawArrayBuffer {
    vertices: Vec<LineVertexUnit>,
    buffer: Buffer,
}

struct DrawIndexedBuffer {
    vertices: Vec<LineVertexUnit>,
    active_indices: Vec<u32>,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    active_index_buffer: Buffer,
    color: Vector4<f32>,
}

struct OffscreenPassiveRenderTargetEffect {
    passive_effect: Rc<RefCell<dyn IEffect>>,
    enabled: bool,
}

pub struct Model {
    handle: u16,
    camera: Rc<RefCell<dyn Camera>>,
    selection: Rc<RefCell<dyn ModelObjectSelection>>,
    drawer: Box<LinearDrawer>,
    skin_deformer: Rc<RefCell<dyn SkinDeformer>>,
    gizmo: Rc<RefCell<dyn Gizmo>>,
    vertex_weight_painter: Rc<RefCell<dyn VertexWeightPainter>>,
    offscreen_passive_render_target_effects: HashMap<String, OffscreenPassiveRenderTargetEffect>,
    draw_all_vertex_normals: DrawArrayBuffer,
    draw_all_vertex_points: DrawArrayBuffer,
    draw_all_vertex_faces: DrawIndexedBuffer,
    draw_all_vertex_weights: DrawIndexedBuffer,
    draw_rigid_body: HashMap<Rc<RefCell<ShapesMesh>>, DrawIndexedBuffer>,
    draw_joint: HashMap<Rc<RefCell<ShapesMesh>>, DrawIndexedBuffer>,
    opaque: Rc<RefCell<NanoemModel>>,
    undo_stack: Box<UndoStack>,
    editing_undo_stack: Box<UndoStack>,
    active_morph_ptr: HashMap<nanoem::model::ModelMorphCategory, Rc<RefCell<NanoemMorph>>>,
    active_constraint_ptr: Rc<RefCell<NanoemConstraint>>,
    active_material_ptr: Rc<RefCell<NanoemMaterial>>,
    hovered_bone_ptr: Rc<RefCell<NanoemBone>>,
    vertex_buffer_data: Vec<u8>,
    face_states: Vec<u32>,
    active_bone_pair_ptr: (Rc<RefCell<NanoemBone>>, Rc<RefCell<NanoemBone>>),
    active_effect_pair_ptr: (Rc<RefCell<dyn IEffect>>, Rc<RefCell<dyn IEffect>>),
    screen_image: Image,
    loading_image_items: Vec<LoadingImageItem>,
    image_map: HashMap<String, Image>,
    bone_index_hash_map: HashMap<Rc<RefCell<NanoemMaterial>>, HashMap<i32, i32>>,
    bones: HashMap<String, Rc<RefCell<NanoemBone>>>,
    morphs: HashMap<String, Rc<RefCell<NanoemMorph>>>,
    constraints: HashMap<Rc<RefCell<NanoemBone>>, Rc<RefCell<NanoemConstraint>>>,
    redo_bone_names: Vec<String>,
    redo_morph_names: Vec<String>,
    outside_parents: HashMap<Rc<RefCell<NanoemBone>>, (String, String)>,
    image_uris: HashMap<String, Uri>,
    attachment_uris: HashMap<String, Uri>,
    bone_bound_rigid_bodies: HashMap<Rc<RefCell<NanoemBone>>, Rc<RefCell<NanoemRigidBody>>>,
    constraint_joint_bones: HashMap<Rc<RefCell<NanoemBone>>, Rc<RefCell<NanoemConstraint>>>,
    inherent_bones: HashMap<Rc<RefCell<NanoemBone>>, HashSet<NanoemBone>>,
    constraint_effect_bones: HashSet<Rc<RefCell<NanoemBone>>>,
    parent_bone_tree: HashMap<Rc<RefCell<NanoemBone>>, Vec<Rc<RefCell<NanoemBone>>>>,
    shared_fallback_bone: Rc<RefCell<Bone>>,
    bounding_box: BoundingBox,
    // UserData m_userData;
    annotations: HashMap<String, String>,
    vertex_buffers: [wgpu::Buffer; 2],
    index_buffer: wgpu::Buffer,
    edge_color: Vector4<f32>,
    transform_axis_type: AxisType,
    edit_action_type: EditActionType,
    transform_coordinate_type: TransformCoordinateType,
    file_uri: Uri,
    name: String,
    comment: String,
    canonical_name: String,
    states: u32,
    edge_size_scale_factor: f32,
    opacity: f32,
    // void *m_dispatchParallelTaskQueue
    count_vertex_skinning_needed: i32,
    stage_vertex_buffer_index: i32,
}

impl Model {
    pub const INITIAL_WORLD_MATRIX: Matrix4<f32> = Matrix4::new(
        1f32, 0f32, 0f32, 0f32, 0f32, 1f32, 0f32, 0f32, 0f32, 0f32, 1f32, 0f32, 0f32, 0f32, 0f32,
        1f32,
    );
    pub const DEFAULT_CM_SCALE_FACTOR: f32 = 0.1259496f32;
    pub const DEFAULT_MODEL_CORRECTION_HEIGHT: f32 = -2f32;
    pub const PMX_FORMAT_EXTENSION: &'static str = "pmx";
    pub const PMD_FORMAT_EXTENSION: &'static str = "pmd";

    pub fn new(project: &Project, handle: u16) -> Self {
        Self {
            handle,
            camera: todo!(),
            selection: todo!(),
            drawer: todo!(),
            skin_deformer: todo!(),
            gizmo: todo!(),
            vertex_weight_painter: todo!(),
            offscreen_passive_render_target_effects: todo!(),
            draw_all_vertex_normals: todo!(),
            draw_all_vertex_points: todo!(),
            draw_all_vertex_faces: todo!(),
            draw_all_vertex_weights: todo!(),
            draw_rigid_body: todo!(),
            draw_joint: todo!(),
            opaque: todo!(),
            undo_stack: todo!(),
            editing_undo_stack: todo!(),
            active_morph_ptr: todo!(),
            active_constraint_ptr: todo!(),
            active_material_ptr: todo!(),
            hovered_bone_ptr: todo!(),
            vertex_buffer_data: todo!(),
            face_states: todo!(),
            active_bone_pair_ptr: todo!(),
            active_effect_pair_ptr: todo!(),
            screen_image: todo!(),
            loading_image_items: todo!(),
            image_map: todo!(),
            bone_index_hash_map: todo!(),
            bones: todo!(),
            morphs: todo!(),
            constraints: todo!(),
            redo_bone_names: todo!(),
            redo_morph_names: todo!(),
            outside_parents: todo!(),
            image_uris: todo!(),
            attachment_uris: todo!(),
            bone_bound_rigid_bodies: todo!(),
            constraint_joint_bones: todo!(),
            inherent_bones: todo!(),
            constraint_effect_bones: todo!(),
            parent_bone_tree: todo!(),
            shared_fallback_bone: todo!(),
            bounding_box: todo!(),
            annotations: todo!(),
            vertex_buffers: todo!(),
            index_buffer: todo!(),
            edge_color: todo!(),
            transform_axis_type: todo!(),
            edit_action_type: todo!(),
            transform_coordinate_type: todo!(),
            file_uri: todo!(),
            name: todo!(),
            comment: todo!(),
            canonical_name: todo!(),
            states: todo!(),
            edge_size_scale_factor: todo!(),
            opacity: todo!(),
            count_vertex_skinning_needed: todo!(),
            stage_vertex_buffer_index: todo!(),
        }
    }

    pub fn loadable_extensions() -> Vec<&'static str> {
        vec![Self::PMD_FORMAT_EXTENSION, Self::PMX_FORMAT_EXTENSION]
    }

    pub fn is_loadable_extension(extension: &str) -> bool {
        Self::loadable_extensions()
            .iter()
            .any(|ext| ext.to_lowercase().eq(extension))
    }

    pub fn uri_has_loadable_extension(uri: &Uri) -> bool {
        if let Some(ext) = uri.absolute_path_extension() {
            Self::is_loadable_extension(ext)
        } else {
            false
        }
    }

    pub fn generate_new_model_data(
        desc: &NewModelDescription,
    ) -> Result<Vec<u8>, nanoem::common::Status> {
        let mut buffer = nanoem::common::MutableBuffer::create()?;
        let mut model = NanoemModel::default();
        {
            model.set_additional_uv_size(0);
            model.set_codec_type(nanoem::common::CodecType::Utf16);
            model.set_format_type(ModelFormatType::Pmx2_0);
            for language in nanoem::common::LanguageType::all() {
                model.set_name(
                    desc.name.get(language).unwrap_or(&"".to_string()),
                    language.clone(),
                );
                model.set_comment(
                    desc.comment.get(language).unwrap_or(&"".to_string()),
                    language.clone(),
                );
            }
        }
        let mut center_bone = NanoemBone::default();
        center_bone.set_name(
            &Bone::NAME_CENTER_IN_JAPANESE.to_string(),
            nanoem::common::LanguageType::Japanese,
        );
        center_bone.set_name(&"Center".to_string(), nanoem::common::LanguageType::English);
        center_bone.set_visible(true);
        center_bone.set_movable(true);
        center_bone.set_rotatable(true);
        center_bone.set_user_handleable(true);
        let center_bone_rc = Rc::from(RefCell::from(center_bone));
        model.insert_bone(&center_bone_rc, -1)?;
        {
            let mut root_label = nanoem::model::ModelLabel::default();
            root_label.set_name(&"Root".to_string(), nanoem::common::LanguageType::Japanese);
            root_label.set_name(&"Root".to_string(), nanoem::common::LanguageType::English);
            root_label.insert_item_object(
                &nanoem::model::ModelLabelItem::create_from_bone_object(center_bone_rc),
                -1,
            );
            root_label.set_special(true);
            model.insert_label(&root_label, -1);
        }
        {
            let mut expression_label = nanoem::model::ModelLabel::default();
            expression_label.set_name(
                &Label::NAME_EXPRESSION_IN_JAPANESE.to_string(),
                nanoem::common::LanguageType::Japanese,
            );
            expression_label.set_name(
                &"Expression".to_string(),
                nanoem::common::LanguageType::English,
            );
            expression_label.set_special(true);
            model.insert_label(&expression_label, -1);
        }
        model.save_to_buffer(&mut buffer)?;
        Ok(buffer.get_data())
    }

    pub fn find_bone(&self, name: &String) -> Option<Rc<RefCell<NanoemBone>>> {
        self.bones.get(name).map(|rc| rc.clone())
    }

    pub fn get_name(&self) -> &String {
        &self.name
    }

    pub fn get_canonical_name(&self) -> &String {
        &self.canonical_name
    }

    pub fn is_add_blend_enabled(&self) -> bool {
        // TODO: isAddBlendEnabled
        true
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn world_transform(&self, initial: &Matrix4<f32>) -> Matrix4<f32> {
        initial.clone()
    }
}

impl Drawable for Model {
    fn draw(&self, typ: DrawType, project: &Project, device: &wgpu::Device, adapter_info: wgpu::AdapterInfo) {
        if self.is_visible() {
            match typ {
                DrawType::Color => self.draw_color(project, device, adapter_info),
                DrawType::Edge => todo!(),
                DrawType::GroundShadow => todo!(),
                DrawType::ShadowMap => todo!(),
                DrawType::ScriptExternalColor => todo!(),
            }
        }
    }

    fn is_visible(&self) -> bool {
        // TODO: isVisible
        true
    }
}

impl Model {
    fn draw_color(&self, project: &Project, device: &wgpu::Device, adapter_info: wgpu::AdapterInfo) {
        let viewport_primary_texture_view = project.viewport_primary_texture_view();
        let mut index_offset = 0usize;
        let model_ref = self.opaque.borrow();
        let materials = model_ref.get_all_material_objects();
        for nanoem_material in materials {
            let num_indices = nanoem_material.get_num_vertex_indices();
            let buffer = pass::Buffer::new(
                num_indices,
                index_offset,
                &self.vertex_buffers[1 - self.stage_vertex_buffer_index as usize],
                &self.index_buffer,
                true,
            );
            if let Some(material) = nanoem_material.get_user_data() {
                if material.borrow().is_visible() {
                    // TODO: get technique by discovery
                    let mut technique =
                        ObjectTechnique::new(nanoem_material.is_point_draw_enabled());
                    let technique_type = technique.technique_type();
                    while let Some((pass, shader)) = technique.execute(device) {
                        pass.set_global_parameters(self, project);
                        pass.set_camera_parameters(
                            project.active_camera(),
                            &Self::INITIAL_WORLD_MATRIX,
                            self,
                        );
                        pass.set_light_parameters(project.global_light(), false);
                        pass.set_all_model_parameters(self, project);
                        pass.set_material_parameters(
                            nanoem_material,
                            technique_type,
                            project.shared_fallback_image(),
                        );
                        pass.set_shadow_map_parameters(
                            project.shadow_camera(),
                            &Self::INITIAL_WORLD_MATRIX,
                            project,
                            adapter_info.backend,
                            technique_type,
                            project.shared_fallback_image(),
                        );
                        pass.execute(
                            &buffer,
                            &viewport_primary_texture_view,
                            None,
                            shader,
                            technique_type,
                            device,
                            self,
                            project,
                        );
                    }
                    // if (!technique->hasNextScriptCommand() && !scriptExternalColor) {
                    // technique->resetScriptCommandState();
                    // technique->resetScriptExternalColor();
                    // }
                }
            }
            index_offset += num_indices;
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Matrices {
    world_transform: Matrix4<f32>,
    local_transform: Matrix4<f32>,
    normal_transform: Matrix4<f32>,
    skinning_transform: Matrix4<f32>,
}

#[derive(Debug, Clone, Copy)]
struct BezierControlPoints {
    translation_x: Vector4<u8>,
    translation_y: Vector4<u8>,
    translation_z: Vector4<u8>,
    orientation: Vector4<u8>,
}

#[derive(Debug, Clone, Copy)]
struct LinearInterpolationEnable {
    translation_x: bool,
    translation_y: bool,
    translation_z: bool,
    orientation: bool,
}

struct FrameTransform {
    translation: Vector3<f32>,
    orientation: Quaternion<f32>,
    bezier_control_points: BezierControlPoints,
    enable_linear_interpolation: LinearInterpolationEnable,
}

#[derive(Debug, Clone)]
struct Bone {
    name: String,
    canonical_name: String,
    matrices: Matrices,
    local_orientation: Quaternion<f32>,
    local_inherent_orientation: Quaternion<f32>,
    local_morph_orientation: Quaternion<f32>,
    local_user_orientation: Quaternion<f32>,
    constraint_joint_orientation: Quaternion<f32>,
    local_translation: Vector3<f32>,
    local_inherent_translation: Vector3<f32>,
    local_morph_translation: Vector3<f32>,
    local_user_translation: Vector3<f32>,
    bezier_control_points: BezierControlPoints,
    states: u32,
}

impl Bone {
    const DEFAULT_BAZIER_CONTROL_POINT: [u8; 4] = [20, 20, 107, 107];
    const DEFAULT_AUTOMATIC_BAZIER_CONTROL_POINT: [u8; 4] = [64, 0, 64, 127];
    const NAME_ROOT_PARENT_IN_JAPANESE: &'static [u8] = &[
        0xe5, 0x85, 0xa8, 0xe3, 0x81, 0xa6, 0xe3, 0x81, 0xae, 0xe8, 0xa6, 0xaa, 0x0,
    ];
    const NAME_CENTER_IN_JAPANESE_UTF8: &'static [u8] = &[
        0xe3, 0x82, 0xbb, 0xe3, 0x83, 0xb3, 0xe3, 0x82, 0xbf, 0xe3, 0x83, 0xbc, 0,
    ];
    const NAME_CENTER_IN_JAPANESE: &'static str = "センター";
    const NAME_CENTER_OF_VIEWPOINT_IN_JAPANESE: &'static [u8] = &[
        0xe6, 0x93, 0x8d, 0xe4, 0xbd, 0x9c, 0xe4, 0xb8, 0xad, 0xe5, 0xbf, 0x83, 0,
    ];
    const NAME_CENTER_OFFSET_IN_JAPANESE: &'static [u8] = &[
        0xe3, 0x82, 0xbb, 0xe3, 0x83, 0xb3, 0xe3, 0x82, 0xbf, 0xe3, 0x83, 0xbc, 0xe5, 0x85, 0x88, 0,
    ];
    const NAME_LEFT_IN_JAPANESE: &'static [u8] = &[0xe5, 0xb7, 0xa6, 0x0];
    const NAME_RIGHT_IN_JAPANESE: &'static [u8] = &[0xe5, 0x8f, 0xb3, 0x0];
    const NAME_DESTINATION_IN_JAPANESE: &'static [u8] = &[0xe5, 0x85, 0x88, 0x0];
    const LEFT_KNEE_IN_JAPANESE: &'static [u8] =
        &[0xe5, 0xb7, 0xa6, 0xe3, 0x81, 0xb2, 0xe3, 0x81, 0x96, 0x0];
    const RIGHT_KNEE_IN_JAPANESE: &'static [u8] =
        &[0xe5, 0x8f, 0xb3, 0xe3, 0x81, 0xb2, 0xe3, 0x81, 0x96, 0x0];

    // fn synchronize_transform(motion: &mut Motion, model_bone: &ModelBone, model_rigid_body: &ModelRigidBody, frame_index: u32, transform: &FrameTransform) {
    //     let name = model_bone.get_name(LanguageType::Japanese).unwrap();
    //     if let Some(Keyframe) = motion.find_bone_keyframe_object(name, index)
    // }
}

struct Label {
    // TODO
}

impl Label {
    const NAME_EXPRESSION_IN_JAPANESE_UTF8: &'static [u8] =
        &[0xe8, 0xa1, 0xa8, 0xe6, 0x83, 0x85, 0x0];
    const NAME_EXPRESSION_IN_JAPANESE: &'static str = "表情";
}

pub struct MaterialColor {
    pub ambient: Vector3<f32>,
    pub diffuse: Vector3<f32>,
    pub specular: Vector3<f32>,
    pub diffuse_opacity: f32,
    pub specular_power: f32,
    pub diffuse_texture_blend_factor: Vector4<f32>,
    pub sphere_texture_blend_factor: Vector4<f32>,
    pub toon_texture_blend_factor: Vector4<f32>,
}

struct MaterialBlendColor {
    base: MaterialColor,
    add: MaterialColor,
    mul: MaterialColor,
}

pub struct MaterialEdge {
    pub color: Vector3<f32>,
    pub opacity: f32,
    pub size: f32,
}

struct MaterialBlendEdge {
    base: MaterialEdge,
    add: MaterialEdge,
    mul: MaterialEdge,
}

pub struct Material {
    // TODO
    color: MaterialBlendColor,
    edge: MaterialBlendEdge,
    diffuse_image: Option<Rc<RefCell<dyn ImageView>>>,
    sphere_map_image: Option<Rc<RefCell<dyn ImageView>>>,
    toon_image: Option<Rc<RefCell<dyn ImageView>>>,
}

impl Material {
    pub const MINIUM_SPECULAR_POWER: f32 = 0.1f32;

    pub fn is_visible(&self) -> bool {
        // TODO: isVisible
        true
    }

    pub fn color(&self) -> MaterialColor {
        MaterialColor {
            ambient: self
                .color
                .base
                .ambient
                .mul_element_wise(self.color.mul.ambient)
                + self.color.add.ambient,
            diffuse: self
                .color
                .base
                .diffuse
                .mul_element_wise(self.color.mul.diffuse)
                + self.color.add.diffuse,
            specular: self
                .color
                .base
                .specular
                .mul_element_wise(self.color.mul.specular)
                + self.color.add.specular,
            diffuse_opacity: self.color.base.diffuse_opacity * self.color.mul.diffuse_opacity
                + self.color.add.diffuse_opacity,
            specular_power: (self.color.base.specular_power * self.color.mul.specular_power
                + self.color.add.specular_power)
                .min(Self::MINIUM_SPECULAR_POWER),
            diffuse_texture_blend_factor: self
                .color
                .base
                .diffuse_texture_blend_factor
                .mul_element_wise(self.color.mul.diffuse_texture_blend_factor)
                + self.color.add.diffuse_texture_blend_factor,
            sphere_texture_blend_factor: self
                .color
                .base
                .sphere_texture_blend_factor
                .mul_element_wise(self.color.mul.sphere_texture_blend_factor)
                + self.color.add.sphere_texture_blend_factor,
            toon_texture_blend_factor: self
                .color
                .base
                .toon_texture_blend_factor
                .mul_element_wise(self.color.mul.toon_texture_blend_factor)
                + self.color.add.toon_texture_blend_factor,
        }
    }

    pub fn edge(&self) -> MaterialEdge {
        MaterialEdge {
            color: self.edge.base.color.mul_element_wise(self.edge.mul.color) + self.edge.add.color,
            opacity: self.edge.base.opacity * self.edge.mul.opacity + self.edge.add.opacity,
            size: self.edge.base.size * self.edge.mul.size + self.edge.add.size,
        }
    }

    pub fn diffuse_image(&self) -> Option<Rc<RefCell<dyn ImageView>>> {
        self.diffuse_image.clone()
    }

    pub fn sphere_map_image(&self) -> Option<Rc<RefCell<dyn ImageView>>> {
        self.sphere_map_image.clone()
    }

    pub fn toon_image(&self) -> Option<Rc<RefCell<dyn ImageView>>> {
        self.toon_image.clone()
    }
}

pub struct VisualizationClause {
    // TODO
}
