use std::{
    cell::{Ref, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Instant,
};

use cgmath::{ElementWise, Vector2, Vector3, Vector4, VectorSpace};

use crate::{
    accessory::Accessory,
    accessory_program_bundle::AccessoryProgramBundle,
    audio_player::{AudioPlayer, ClockAudioPlayer},
    background_video_renderer::BackgroundVideoRenderer,
    camera::{Camera, PerspectiveCamera},
    clear_pass::ClearPass,
    debug_capture::DebugCapture,
    drawable::{DrawContext, DrawType, Drawable},
    effect::{self, common::RenderPassScope, global_uniform::GlobalUniform, Effect, ScriptOrder},
    error::Error,
    event_publisher::EventPublisher,
    file_manager::FileManager,
    file_utils,
    forward::QuadVertexUnit,
    grid::Grid,
    image_loader::ImageLoader,
    image_view::ImageView,
    injector::Injector,
    internal::{BlitPass, DebugDrawer},
    light::{DirectionalLight, Light},
    model::{BindPose, Bone, Model, VisualizationClause},
    model_program_bundle::ModelProgramBundle,
    motion::Motion,
    physics_engine::{PhysicsEngine, RigidBodyFollowBone, SimulationMode, SimulationTiming},
    pixel_format::PixelFormat,
    primitive_2d::Primitive2d,
    progress::CancelPublisher,
    shadow_camera::ShadowCamera,
    time_line_segment::TimeLineSegment,
    track::Track,
    translator::{LanguageType, Translator},
    undo::UndoStack,
    uri::Uri,
    utils::lerp_f32,
};

pub trait Confirmor {
    fn seek(&mut self, frame_index: u32, project: &Project);
    fn play(&mut self, project: &Project);
    fn resume(&mut self, project: &Project);
}

pub trait RendererCapability {
    fn suggested_sample_level(&self) -> u32;
    fn supports_sample_level(&self, value: u32) -> bool;
}

pub trait SharedCancelPublisherFactory {
    fn cancel_publisher(&self) -> Rc<RefCell<dyn CancelPublisher>>;
}

pub trait SharedDebugCaptureFactory {
    fn debug_factory(&self) -> Rc<RefCell<dyn DebugCapture>>;
}

pub trait SharedResourceFactory {
    fn accessory_program_bundle(&self) -> &AccessoryProgramBundle;
    fn model_program_bundle(&self) -> &ModelProgramBundle;
    fn effect_global_uniform(&self) -> &GlobalUniform;
    fn toon_image(&self, value: i32) -> &dyn ImageView;
    fn toon_color(&self, value: i32) -> Vector4<f32>;
}

#[derive(Debug, Clone, Copy)]
struct SaveState {
    active_model: Option<ModelHandle>,
    camera_angle: Vector3<f32>,
    camera_look_at: Vector3<f32>,
    camera_distance: f32,
    camera_fov: i32,
    light_color: Vector3<f32>,
    light_direction: Vector3<f32>,
    physics_simulation_mode: SimulationMode,
    local_frame_index: u32,
    state_flags: ProjectStates,
    visible_grid: bool,
}

impl SaveState {
    pub fn new(project: &Project) -> Self {
        Self {
            active_model: project.active_model_pair.0,
            camera_angle: project.camera.angle(),
            camera_look_at: project.camera.look_at(project.active_model()),
            camera_distance: project.camera.distance(),
            camera_fov: project.camera.fov(),
            light_color: project.light.color(),
            light_direction: project.light.direction(),
            physics_simulation_mode: project.physics_engine.simulation_mode,
            local_frame_index: project.local_frame_index.0,
            state_flags: project.state_flags,
            visible_grid: project.grid.visible(),
        }
    }
}

pub struct DrawQueue {}

pub struct BatchDrawQueue {}

pub struct SerialDrawQueue {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditingMode {
    None,
    Select,
    Move,
    Rotate,
}

pub enum FilePathMode {}

pub enum CursorType {}

#[derive(Debug, Clone, Copy, Default)]
struct HandleAllocator(u32);

impl HandleAllocator {
    pub fn new() -> Self {
        Self(0)
    }

    pub fn next(&mut self) -> u32 {
        self.0 += 1;
        self.0
    }

    pub fn clear(&mut self) {
        self.0 = 0u32;
    }
}

pub struct RenderPassBundle {}

pub struct SharedRenderTargetImageContainer {}

struct Pass {
    name: String,
    color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    color_texture_format: wgpu::TextureFormat,
    depth_texture_format: wgpu::TextureFormat,
    sampler: wgpu::Sampler,
}

impl Pass {
    pub fn new(
        name: &str,
        size: Vector2<u32>,
        color_texture_format: wgpu::TextureFormat,
        sample_count: u32,
        device: &wgpu::Device,
    ) -> Self {
        let depth_texture_format = wgpu::TextureFormat::Depth24PlusStencil8;
        let (color_texture, color_view, depth_texture, depth_view, sampler) = Self::_update(
            name,
            size,
            color_texture_format,
            depth_texture_format,
            sample_count,
            device,
        );
        Self {
            name: name.to_owned(),
            color_texture,
            color_view,
            depth_texture,
            depth_view,
            color_texture_format,
            depth_texture_format,
            sampler,
        }
    }

    pub fn update(
        &mut self,
        size: Vector2<u32>,
        color_texture_format: wgpu::TextureFormat,
        sample_count: u32,
        device: &wgpu::Device,
    ) {
        let (color_texture, color_view, depth_texture, depth_view, sampler) = Self::_update(
            self.name.as_str(),
            size,
            color_texture_format,
            self.depth_texture_format,
            sample_count,
            device,
        );
        self.color_texture = color_texture;
        self.color_view = color_view;
        self.color_texture_format = color_texture_format;
        self.depth_texture = depth_texture;
        self.depth_view = depth_view;
        self.sampler = sampler;
    }

    fn _update(
        name: &str,
        size: Vector2<u32>,
        color_texture_format: wgpu::TextureFormat,
        depth_texture_format: wgpu::TextureFormat,
        sample_count: u32,
        device: &wgpu::Device,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Sampler,
    ) {
        // TODO: Feature Query For msaa?
        let color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(format!("{}/ColorTexture", name).as_str()),
            size: wgpu::Extent3d {
                width: size.x as u32,
                height: size.y as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: color_texture_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(format!("{}/DepthTexture", name).as_str()),
            size: wgpu::Extent3d {
                width: size.x as u32,
                height: size.y as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24PlusStencil8,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let common_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(format!("{}/Sampler", name).as_str()),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::default(),
            ..Default::default()
        });
        (
            color_texture,
            color_view,
            depth_texture,
            depth_view,
            common_sampler,
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct FpsUnit {
    value: u32,
    scale_factor: f32,
    inverted_value: f32,
    inverted_scale_factor: f32,
}

impl FpsUnit {
    pub const HALF_BASE_FPS: u32 = 30u32;
    pub const HALF_BASE_FPS_F32: f32 = Self::HALF_BASE_FPS as f32;

    pub fn new(value: u32) -> Self {
        Self {
            value: value.max(Self::HALF_BASE_FPS),
            scale_factor: (value as f32) / Self::HALF_BASE_FPS_F32,
            inverted_value: 1f32 / (value as f32),
            inverted_scale_factor: Self::HALF_BASE_FPS_F32 / (value as f32),
        }
    }

    pub fn value(&self) -> u32 {
        self.value
    }
}

struct OffscreenRenderTargetCondition {}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectStates {
    pub disable_hidden_bone_bounds_rigid_body: bool,
    pub display_user_interface: bool,
    pub display_transform_handle: bool,
    pub enable_loop: bool,
    pub enable_shared_camera: bool,
    pub enable_ground_shadow: bool,
    pub enable_multiple_bone_selection: bool,
    pub enable_bezier_curve_adjustment: bool,
    pub enable_motion_merge: bool,
    pub enable_effect_plugin: bool,
    pub enable_viewport_locked: bool,
    pub disable_display_sync: bool,
    pub primary_cursor_type_left: bool,
    pub loading_redo_file: bool,
    pub enable_playing_audio_part: bool,
    pub enable_viewport_with_transparent: bool,
    pub enable_compiled_effect_cache: bool,
    pub reset_all_passes: bool,
    pub cancel_requested: bool,
    pub enable_uniformed_viewport_image_size: bool,
    pub viewport_hovered: bool,
    pub enable_fps_counter: bool,
    pub enable_performance_monitor: bool,
    pub input_text_focus: bool,
    pub viewport_image_size_changed: bool,
    pub enable_physics_simulation_for_bone_keyframe: bool,
    pub enable_image_anisotropy: bool,
    pub enable_image_mipmap: bool,
    pub enable_power_saving: bool,
    pub enable_model_editing: bool,
    pub viewport_window_detached: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConfirmSeekFlags {
    pub bone: bool,
    pub camera: bool,
    pub light: bool,
    pub model: bool,
    pub morph: bool,
    pub self_shadow: bool,
    pub accessory: bool,
    pub all: bool,
}

pub type ModelHandle = u32;

pub struct Project {
    // background_video_renderer: Box<dyn BackgroundVideoRenderer<Error>>,
    // confirmor: Box<dyn Confirmor>,
    // file_manager: Box<dyn FileManager>,
    // event_publisher: Box<dyn EventPublisher>,
    // primitive_2d: Box<dyn Primitive2d>,
    // renderer_capability: Box<dyn RendererCapability>,
    // shared_cancel_publisher_factory: Box<dyn SharedCancelPublisherFactory>,
    // shared_resource_factory: Box<dyn SharedResourceFactory>,
    // translator: Box<dyn Translator>,
    // shared_image_loader: Option<Rc<ImageLoader>>,
    transform_model_order_list: Vec<ModelHandle>,
    active_model_pair: (Option<ModelHandle>, Option<ModelHandle>),
    // active_accessory: Option<Rc<RefCell<Accessory>>>,
    audio_player: Box<dyn AudioPlayer + Send>,
    physics_engine: Box<PhysicsEngine>,
    camera: PerspectiveCamera,
    light: DirectionalLight,
    shadow_camera: ShadowCamera,
    grid: Box<Grid>,
    camera_motion: Motion,
    light_motion: Motion,
    self_shadow_motion: Motion,
    // undo_stack: Rc<RefCell<UndoStack>>,
    // all_models: Vec<Rc<RefCell<Model>>>,
    // all_accessories: Vec<Rc<RefCell<Accessory>>>,
    // all_motions: Vec<Rc<RefCell<Motion>>>,
    // drawable_to_motion_ptrs: HashMap<Rc<RefCell<dyn Drawable>>, Rc<RefCell<Motion>>>,
    model_to_motion: HashMap<ModelHandle, Motion>,
    // all_traces: Vec<Rc<RefCell<dyn Track>>>,
    // selected_track: Option<Rc<RefCell<dyn Track>>>,
    last_save_state: Option<SaveState>,
    model_program_bundle: Box<ModelProgramBundle>,
    // draw_queue: Box<DrawQueue>,
    // batch_draw_queue: Box<BatchDrawQueue>,
    // serial_draw_queue: Box<SerialDrawQueue>,
    // offscreen_render_pass_scope: Box<RenderPassScope>,
    // viewport_pass_blitter: Box<BlitPass>,
    // render_pass_blitter: Box<BlitPass>,
    // shared_image_blitter: Box<BlitPass>,
    // render_pass_clear: Box<ClearPass>,
    clear_pass: Box<ClearPass>,
    // shared_debug_drawer: Rc<RefCell<DebugDrawer>>,
    viewport_texture_format: (wgpu::TextureFormat, wgpu::TextureFormat),
    // last_bind_pose: BindPose,
    // rigid_body_visualization_clause: VisualizationClause,
    draw_type: DrawType,
    // file_uri: (Uri, file_utils::TransientPath),
    // redo_file_uri: Uri,
    // drawables_to_attach_offscreen_render_target_effect: (String, HashSet<Rc<RefCell<dyn Drawable>>>),
    // current_render_pass: Option<Rc<RenderPass<'a>>>,
    // last_drawn_render_pass: Option<Rc<RenderPass<'a>>>,
    // current_offscreen_render_pass: Option<Rc<RenderPass<'a>>>,
    // origin_offscreen_render_pass: Option<Rc<RenderPass<'a>>>,
    // script_external_render_pass: Option<Rc<RenderPass<'a>>>,
    // shared_render_target_image_containers: HashMap<String, SharedRenderTargetImageContainer>,
    editing_mode: EditingMode,
    // file_path_mode: FilePathMode,
    playing_segment: TimeLineSegment,
    selection_segment: TimeLineSegment,
    base_duration: u32,
    language: LanguageType,
    viewport_size: (Vector2<u32>, Vector2<u32>),
    // background_video_rect: Vector4<i32>,
    // bone_selection_rect: Vector4<i32>,
    // logical_scale_cursor_positions: HashMap<CursorType, Vector4<i32>>,
    // logical_scale_moving_cursor_position: Vector2<i32>,
    // scroll_delta: Vector2<i32>,
    viewport_background_color: Vector4<f32>,
    // all_offscreen_render_targets: HashMap<Rc<RefCell<Effect>>, HashMap<String, Vec<OffscreenRenderTargetCondition>>>,
    fallback_texture: wgpu::TextureView,
    shared_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    shadow_bind_group_layout: wgpu::BindGroupLayout,
    texture_fallback_bind: wgpu::BindGroup,
    shadow_fallback_bind: wgpu::BindGroup,
    // // TODO: bx::HandleAlloc *m_objectHandleAllocator;
    object_handler_allocator: HandleAllocator,
    // accessory_handle_map: HashMap<u16, Rc<RefCell<Accessory>>>,
    model_handle_map: HashMap<ModelHandle, Model>,
    // motion_handle_map: HashMap<u16, Rc<RefCell<Motion>>>,
    // render_pass_bundle_map: HashMap<u32, RenderPassBundle>,
    // hashed_render_pass_bundle_map: HashMap<u32, Rc<RefCell<RenderPassBundle>>>,
    // redo_object_handles: HashMap<u16, u32>,
    // render_pass_string_map: HashMap<u32, String>,
    // render_pipeline_string_map: HashMap<u32, String>,
    viewport_primary_pass: Pass,
    viewport_secondary_pass: Pass,
    // context_2d_pass: Pass,
    // background_image: (Texture, Vector2<u32>),
    preferred_motion_fps: FpsUnit,
    // editing_fps: u32,
    // bone_interpolation_type: i32,
    // camera_interpolation_type: i32,
    // model_clipboard: Vec<u8>,
    // motion_clipboard: Vec<u8>,
    // effect_order_set: HashMap<effect::ScriptOrderType, HashSet<Rc<RefCell<dyn Drawable>>>>,
    // effect_references: HashMap<String, (Rc<RefCell<Effect>>, i32)>,
    // loaded_effect_set: HashSet<Rc<RefCell<Effect>>>,
    depends_on_script_external: Vec<Box<dyn Drawable + Send>>,
    transform_performed_at: (u32, i32),
    // indices_of_material_to_attach_effect: (u16, HashSet<usize>),
    // uptime: (f64, f64),
    local_frame_index: (u32, u32),
    time_step_factor: f32,
    // background_video_scale_factor: f32,
    // circle_radius: f32,
    sample_level: (u32, u32),
    state_flags: ProjectStates,
    confirm_seek_flags: ConfirmSeekFlags,
    // last_physics_debug_flags: u32,
    // coordination_system: u32,
    // cursor_modifiers: u32,
    // actual_fps: u32,
    // actual_sequence: u32,
    // active: bool,
    tmp_texture_map: HashMap<String, wgpu::Texture>,
}

impl Project {
    pub const MINIMUM_BASE_DURATION: u32 = 300;
    pub const MAXIMUM_BASE_DURATION: u32 = i32::MAX as u32;
    pub const DEFAULT_CIRCLE_RADIUS_SIZE: f32 = 7.5f32;

    pub const DEFAULT_VIEWPORT_IMAGE_SIZE: [u32; 2] = [640, 360];
    pub const TIME_BASED_AUDIO_SOURCE_DEFAULT_SAMPLE_RATE: u32 = 1440;

    pub const REDO_LOG_FILE_EXTENSION: &'static str = "redo";
    pub const ARCHIVED_NATIVE_FORMAT_FILE_EXTENSION: &'static str = "nma";
    pub const FILE_SYSTEM_BASED_NATIVE_FORMAT_FILE_EXTENSION: &'static str = "nmm";
    pub const POLYGON_MOVIE_MAKER_FILE_EXTENSION: &'static str = "pmm";
    pub const VIEWPORT_PRIMARY_NAME: &'static str = "@mdanceio/Viewport/Primary";
    pub const VIEWPORT_SECONDARY_NAME: &'static str = "@mdanceio/Viewport/Secondary";

    pub fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        injector: Injector,
    ) -> Self {
        log::trace!("Start Creating new Project");
        let viewport_size = Vector2::new(injector.viewport_size[0], injector.viewport_size[1]);

        let viewport_primary_pass = Pass::new(
            Self::VIEWPORT_PRIMARY_NAME,
            viewport_size,
            injector.texture_format(),
            1,
            &device,
        );

        let viewport_secondary_pass = Pass::new(
            Self::VIEWPORT_SECONDARY_NAME,
            viewport_size,
            injector.texture_format(),
            1,
            &device,
        );
        log::trace!("Finish Primary and Secondary Pass");

        let fallback_texture = Self::create_white_fallback_image(&device, &queue)
            .create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BindGroupLayout/Texture"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let texture_fallback_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BindGroup/TextureFallback"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&fallback_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&fallback_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&fallback_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ShadowBindGroupLayout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                        count: None,
                    },
                ],
            });
        let shadow_fallback_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BindGroup/ShadowFallbackBindGroup"),
            layout: &shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&fallback_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

        let mut camera = PerspectiveCamera::new();

        camera.update(viewport_size, camera.look_at(None));
        camera.set_dirty(false);
        let mut shadow_camera =
            ShadowCamera::new(&shadow_bind_group_layout, &shadow_sampler, &device);
        if adapter.get_info().backend == wgpu::Backend::Gl {
            // TODO: disable shadow map when gles3
            shadow_camera.set_enabled(false);
        }
        shadow_camera.set_dirty(false);
        let directional_light = DirectionalLight::new();

        let mut physics_engine = Box::new(PhysicsEngine::new());
        physics_engine.simulation_mode = SimulationMode::EnableTracing;

        let mut object_handler_allocator = HandleAllocator::new();

        let mut camera_motion = Motion::empty();
        camera_motion.initialize_camera_frame_0(&camera, None);
        let mut light_motion = Motion::empty();
        light_motion.initialize_light_frame_0(&directional_light);
        let mut self_shadow_motion = Motion::empty();
        self_shadow_motion.initialize_self_shadow_frame_0(&shadow_camera);

        // TODO: build tracks

        Self {
            audio_player: Box::new(ClockAudioPlayer::default()),
            editing_mode: EditingMode::None,
            playing_segment: TimeLineSegment::default(),
            selection_segment: TimeLineSegment::default(),
            language: LanguageType::English,
            base_duration: Self::MINIMUM_BASE_DURATION,
            preferred_motion_fps: FpsUnit::new(60u32),
            time_step_factor: 1f32,
            viewport_size: (viewport_size, viewport_size),
            active_model_pair: (None, None),
            grid: Box::new(Grid::new(device)),
            camera_motion,
            light_motion,
            self_shadow_motion,
            model_to_motion: HashMap::new(),
            model_program_bundle: Box::new(ModelProgramBundle::new(device)),
            draw_type: DrawType::Color,
            depends_on_script_external: vec![],
            clear_pass: Box::new(ClearPass::new(device)),
            viewport_texture_format: (injector.texture_format(), injector.texture_format()),
            viewport_background_color: Vector4::new(0f32, 0f32, 0f32, 1f32),
            local_frame_index: (0, 0),
            transform_performed_at: (Motion::MAX_KEYFRAME_INDEX, 0),
            sample_level: (0u32, 0u32),
            camera,
            shadow_camera,
            light: directional_light,
            fallback_texture,
            shared_sampler: sampler,
            shadow_sampler,
            texture_bind_group_layout,
            shadow_bind_group_layout,
            texture_fallback_bind,
            shadow_fallback_bind,
            object_handler_allocator,
            model_handle_map: HashMap::new(),
            transform_model_order_list: vec![],
            viewport_primary_pass,
            viewport_secondary_pass,
            physics_engine,
            tmp_texture_map: HashMap::new(),
            state_flags: ProjectStates {
                display_transform_handle: true,
                display_user_interface: true,
                enable_motion_merge: true,
                enable_uniformed_viewport_image_size: true,
                enable_fps_counter: true,
                enable_performance_monitor: true,
                enable_physics_simulation_for_bone_keyframe: true,
                enable_image_anisotropy: true,
                enable_ground_shadow: true,
                reset_all_passes: adapter.get_info().backend != wgpu::Backend::Gl,
                ..Default::default()
            },
            confirm_seek_flags: ConfirmSeekFlags::default(),
            last_save_state: None,
        }
        // TODO: may need to publish set fps event
    }

    pub fn parse_language(&self) -> nanoem::common::LanguageType {
        match self.language {
            LanguageType::Japanese
            | LanguageType::ChineseSimplified
            | LanguageType::ChineseTraditional
            | LanguageType::Korean => nanoem::common::LanguageType::Japanese,
            LanguageType::English => nanoem::common::LanguageType::English,
        }
    }

    pub fn sample_count(&self) -> u32 {
        1 << self.sample_level.0
    }

    pub fn sample_level(&self) -> u32 {
        self.sample_level.0
    }

    pub fn current_frame_index(&self) -> u32 {
        self.local_frame_index.0
    }

    pub fn global_camera(&self) -> &PerspectiveCamera {
        &self.camera
    }

    pub fn global_camera_mut(&mut self) -> &mut PerspectiveCamera {
        &mut self.camera
    }

    pub fn active_camera(&self) -> &dyn Camera {
        // TODO: may use model camera
        &self.camera
    }

    pub fn shadow_camera(&self) -> &ShadowCamera {
        &self.shadow_camera
    }

    pub fn global_light(&self) -> &dyn Light {
        &self.light
    }

    pub fn global_light_mut(&mut self) -> &mut DirectionalLight {
        &mut self.light
    }

    pub fn shared_fallback_image(&self) -> &wgpu::TextureView {
        &self.fallback_texture
    }

    pub fn viewport_texture_format(&self) -> wgpu::TextureFormat {
        self.viewport_texture_format.0
    }

    pub fn viewport_primary_depth_view(&self) -> &wgpu::TextureView {
        &self.viewport_primary_pass.depth_view
    }

    pub fn is_render_pass_viewport(&self) -> bool {
        // TODO
        true
    }

    pub fn current_color_attachment_texture(&self) -> Option<&wgpu::TextureView> {
        None
    }

    pub fn set_transform_performed_at(&mut self, value: (u32, i32)) {
        self.transform_performed_at = value
    }

    pub fn reset_transform_performed_at(&mut self) {
        self.set_transform_performed_at((Motion::MAX_KEYFRAME_INDEX, 0))
    }

    pub fn duration(&self, base_duration: u32) -> u32 {
        let mut duration =
            base_duration.clamp(Self::MINIMUM_BASE_DURATION, Self::MAXIMUM_BASE_DURATION);
        duration = duration.max(self.camera_motion.duration());
        duration = duration.max(self.light_motion.duration());
        for motion in self.model_to_motion.values() {
            duration = duration.max(motion.duration());
        }
        duration
    }

    pub fn project_duration(&self) -> u32 {
        self.duration(self.base_duration)
    }

    pub fn base_duration(&self) -> u32 {
        self.base_duration
    }

    pub fn set_base_duration(&mut self, value: u32) {
        self.base_duration = value
            .clamp(Self::MINIMUM_BASE_DURATION, Self::MAXIMUM_BASE_DURATION)
            .max(self.base_duration);
        let new_duration = self.duration(value);
        self.playing_segment.to = self.playing_segment.to.max(new_duration);
        self.selection_segment.to = self.selection_segment.to.max(new_duration);
    }

    pub fn set_playing_segment(&mut self, value: &TimeLineSegment) {
        self.playing_segment = value.normalized(self.project_duration());
    }

    pub fn set_selection_segment(&mut self, value: &TimeLineSegment) {
        self.selection_segment = value.normalized(self.project_duration());
    }

    pub fn set_physics_simulation_mode(&mut self, value: SimulationMode) {
        if self.physics_engine.simulation_mode != value {
            self.physics_engine.simulation_mode = value;
            self.reset_physics_simulation();
        }
    }

    pub fn active_model(&self) -> Option<&Model> {
        self.active_model_pair
            .0
            .and_then(|idx| self.model_handle_map.get(&idx))
    }

    pub fn active_model_mut(&mut self) -> Option<&mut Model> {
        self.active_model_pair
            .0
            .and_then(|idx| self.model_handle_map.get_mut(&idx))
    }

    pub fn set_active_model(&mut self, model: Option<ModelHandle>) {
        let last_active_model = self.active_model_pair.0;
        if last_active_model != model && !self.state_flags.enable_model_editing {
            self.active_model_pair = (model, last_active_model);
            if self.editing_mode == EditingMode::None {
                self.editing_mode = EditingMode::Select;
            } else if model.is_none() {
                self.editing_mode = EditingMode::None;
            }
            // TODO: publish event
            // TODO: rebuild tracks
            self.internal_seek(self.local_frame_index.0);
        }
    }

    pub fn model_mut(&mut self, handle: ModelHandle) -> Option<&mut Model> {
        self.model_handle_map.get_mut(&handle)
    }

    pub fn find_model_by_name(&self, name: &str) -> Option<&Model> {
        for (idx, model) in &self.model_handle_map {
            if model.get_name() == name || model.get_canonical_name() == name {
                return Some(model);
            }
        }
        return None;
    }

    pub fn resolve_bone(&self, value: (&str, &str)) -> Option<&Bone> {
        self.find_model_by_name(value.0)
            .and_then(|model| model.find_bone(value.1))
    }

    pub fn resolve_model_motion(&self, model: ModelHandle) -> Option<&Motion> {
        self.model_to_motion.get(&model)
    }

    pub fn physics_simulation_time_step(&self) -> f32 {
        self.inverted_preferred_motion_fps()
            * self.preferred_motion_fps.scale_factor
            * self.time_step_factor
    }

    pub fn inverted_preferred_motion_fps(&self) -> f32 {
        self.preferred_motion_fps.inverted_value
    }

    pub fn is_physics_simulation_enabled(&self) -> bool {
        match self.physics_engine.simulation_mode {
            crate::physics_engine::SimulationMode::Disable => false,
            crate::physics_engine::SimulationMode::EnableAnytime
            | crate::physics_engine::SimulationMode::EnableTracing => true,
            crate::physics_engine::SimulationMode::EnablePlaying => self.is_playing(),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.audio_player.is_playing()
    }

    fn continue_playing(&mut self) -> bool {
        if self.audio_player.is_finished()
            || self.current_frame_index()
                >= self.playing_segment.frame_index_to(self.project_duration())
        {
            self.stop();
            if self.state_flags.enable_loop {
                self.internal_seek(0);
                self.play();
            } else {
                return false;
            }
        }
        return true;
    }

    pub fn play(&mut self) {
        if !self.state_flags.enable_model_editing {
            let duration_at = self.project_duration();
            let local_frame_index_at = self.current_frame_index();
            self.prepare_playing();
            self.synchronize_all_motions(self.playing_segment.from, 0f32, SimulationTiming::Before);
            self.reset_physics_simulation();
            self.audio_player.play();
        }
    }

    pub fn stop(&mut self) {
        let last_duration = self.project_duration();
        let last_local_frame_index = self.current_frame_index();
        self.audio_player.stop();
        self.audio_player.update();
        self.prepare_stopping(false);
        self.synchronize_all_motions(0, 0f32, SimulationTiming::Before);
        self.reset_physics_simulation();
        self.synchronize_all_motions(0, 0f32, SimulationTiming::After);
        self.mark_all_models_dirty();
        self.local_frame_index = (0, 0);
    }

    fn prepare_playing(&mut self) {
        self.state_flags.input_text_focus = false;
        self.last_save_state = Some(SaveState::new(self));
        self.set_active_model(None);
    }

    fn prepare_stopping(&mut self, force_seek: bool) {
        if let Some(state) = self.last_save_state {
            self.restore_state(&state, force_seek);
        }
        self.last_save_state = None;
    }
}

impl Project {
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // TODO: seek if playing
        let start = Instant::now();
        if self.is_playing() && self.continue_playing() {
            self.audio_player.update();
            let fps = self.preferred_motion_fps.value;
            let base = FpsUnit::HALF_BASE_FPS;
            let fps_rate = fps / base;
            let frame_index =
                (self.audio_player.current_rational().subdivide() * (fps as f64)) as u32;
            let last_frame_index = self
                .audio_player
                .last_rational()
                .map(|rational| (rational.subdivide() * (fps as f64)) as u32);
            let inverted_fpx_rate = 1f32 / fps_rate as f32;
            let amount = (frame_index % fps_rate) as f32 * inverted_fpx_rate;
            let delta = last_frame_index.map_or(0f32, |last_frame_index| {
                if frame_index > last_frame_index {
                    (frame_index - last_frame_index).min(0xffff) as f32
                        * inverted_fpx_rate
                        * self.physics_simulation_time_step()
                } else {
                    0f32
                }
            });
            let seek_start = Instant::now();
            self.internal_seek_precisely(
                (frame_index as f32 * inverted_fpx_rate) as u32,
                amount,
                delta,
            );
            log::info!("Internal Seek Use {:?}", Instant::now() - seek_start);
        }
        // TODO: simulate if simulation anytime
        for (_, model) in &mut self.model_handle_map {
            model.update_staging_vertex_buffer(&self.camera, device, queue);
        }
        // TODO: mark all animated images updatable
        // TODO: render background video
        log::info!("Full Update Use {:?}", Instant::now() - start);
    }

    fn restore_state(&mut self, state: &SaveState, force_seek: bool) {
        self.set_active_model(state.active_model);
        self.camera.set_angle(state.camera_angle);
        self.camera.set_look_at(state.camera_look_at);
        self.camera.set_distance(state.camera_distance);
        self.camera.set_fov(state.camera_fov);
        let bound_look_at = self.camera.bound_look_at(self);
        self.camera.update(self.viewport_size.0, bound_look_at);
        self.light.set_color(state.light_color);
        self.light.set_direction(state.light_direction);
        if force_seek {
            self.internal_seek(state.local_frame_index);
        }
        self.state_flags = state.state_flags;
        self.set_physics_simulation_mode(state.physics_simulation_mode);
    }

    pub fn reset_all_passes(&mut self, device: &wgpu::Device) -> bool {
        if !self.state_flags.reset_all_passes
            || self.viewport_size.1.x == 0
            || self.viewport_size.1.y == 0
        {
            return false;
        }
        self.viewport_size.0 = self.viewport_size.1;
        self.sample_level.0 = self.sample_level.1;
        self.viewport_primary_pass.update(
            self.viewport_size.0,
            self.viewport_texture_format(),
            self.sample_count(),
            device,
        );
        self.viewport_secondary_pass.update(
            self.viewport_size.0,
            self.viewport_texture_format(),
            self.sample_count(),
            device,
        );
        let bound_look_at = self.camera.bound_look_at(self);
        self.camera.update(self.viewport_size.0, bound_look_at);
        self.state_flags.reset_all_passes = false;
        return true;
    }

    pub fn update_global_camera(&mut self) {
        let bound_look_at = self.global_camera().bound_look_at(self);
        let viewport_image_size = self.viewport_size.0;
        self.global_camera_mut()
            .update(viewport_image_size, bound_look_at);
    }

    pub fn seek(&mut self, frame_index: u32, force_seek: bool) {
        self.seek_precisely(frame_index, 0f32, force_seek)
    }

    pub fn seek_precisely(&mut self, frame_index: u32, amount: f32, force_seek: bool) {
        if self.can_seek() {
            // TODO: if not force, use a confirmer to seek
            let last_duration = self.project_duration();
            let seek_from = self.local_frame_index.0;
            let fps = self.preferred_motion_fps.value();
            let base = FpsUnit::HALF_BASE_FPS;
            let fps_rate = fps / base;
            let seconds = (frame_index as f64) / (base as f64);
            let delta = if frame_index > seek_from {
                ((frame_index - seek_from) * fps_rate) as f32 * self.physics_simulation_time_step()
            } else {
                0f32
            };
            self.set_base_duration(frame_index);
            // TODO: seek audio player
            self.internal_seek_precisely(frame_index, amount, delta);
            // TODO: publish event
        }
    }

    fn can_seek(&self) -> bool {
        let mut seekable = !self.state_flags.enable_model_editing;
        if let Some(model) = self.active_model() {
            seekable &= !(model.has_any_dirty_bone() && self.confirm_seek_flags.bone);
            seekable &= !(model.has_any_dirty_morph() && self.confirm_seek_flags.morph);
        } else {
            seekable &= !(self.camera.is_dirty() && self.confirm_seek_flags.camera);
            seekable &= !(self.light.is_dirty() && self.confirm_seek_flags.light);
        }
        seekable
    }

    fn internal_seek(&mut self, frame_index: u32) {
        self.internal_seek_precisely(frame_index, 0f32, 0f32);
    }

    fn internal_seek_precisely(&mut self, frame_index: u32, amount: f32, delta: f32) {
        log::info!("Before Internal seek: {:?}", self.local_frame_index);
        log::info!("Seek to {:?}", frame_index);
        if self.transform_performed_at.0 != Motion::MAX_KEYFRAME_INDEX
            && frame_index != self.transform_performed_at.0
        {
            // TODO: active model undo stack set offset
            self.reset_transform_performed_at();
        }
        if frame_index < self.local_frame_index.0 {
            self.restart(frame_index);
        }
        self.synchronize_all_motions(frame_index, amount, SimulationTiming::Before);
        self.internal_perform_physics_simulation(delta);
        self.synchronize_all_motions(frame_index, amount, SimulationTiming::After);
        self.mark_all_models_dirty();
        self.light.set_dirty(false);
        self.camera.set_dirty(false);
        // TODO: seek background
        // FIXME?: there's nothing to ensure local_frame <= frame_index
        self.local_frame_index.1 = frame_index - self.local_frame_index.0;
        self.local_frame_index.0 = frame_index;
    }

    pub fn reset_physics_simulation(&mut self) {
        self.physics_engine.reset();
        for (handle, model) in &mut self.model_handle_map {
            model.initialize_all_rigid_bodies_transform_feedback(&mut self.physics_engine);
            model.synchronize_all_rigid_bodies_transform_feedback_from_simulation(
                RigidBodyFollowBone::Perform,
                &mut self.physics_engine,
            );
            model.mark_staging_vertex_buffer_dirty();
        }
        self.physics_engine.step(0f32);
        self.restart(self.current_frame_index());
    }

    pub fn reset_all_model_edges(
        &mut self,
        outside_parent_bone_map: &HashMap<(String, String), Bone>,
    ) {
        let physics_simulation_time_step = self.physics_simulation_time_step();
        for (handle, model) in &mut self.model_handle_map {
            if model.edge_size_scale_factor() > 0f32 && !model.is_staging_vertex_buffer_dirty() {
                model.reset_all_morph_deform_states(
                    self.model_to_motion.get(handle).unwrap(),
                    self.local_frame_index.0,
                    &mut self.physics_engine,
                );
                model.deform_all_morphs(false);
                model.perform_all_bones_transform(
                    &mut self.physics_engine,
                    physics_simulation_time_step,
                    outside_parent_bone_map,
                );
                model.mark_staging_vertex_buffer_dirty();
            }
        }
    }

    pub fn synchronize_all_motions(
        &mut self,
        frame_index: u32,
        amount: f32,
        timing: SimulationTiming,
    ) {
        // TODO: for model motions
        for handle in &self.transform_model_order_list {
            if let Some(model) = self.model_handle_map.get_mut(handle) {
                let outside_parent_bone_map = HashMap::new();
                if let Some(motion) = self.model_to_motion.get(handle) {
                    model.synchronize_motion(
                        motion,
                        frame_index,
                        amount,
                        timing,
                        &mut self.physics_engine,
                        &outside_parent_bone_map,
                    );
                }
            }
        }
        if timing == SimulationTiming::After {
            // TODO: for accessory motions
            self.synchronize_camera(frame_index, amount);
            self.synchronize_light(frame_index, amount);
            self.synchronize_self_shadow(frame_index, amount);
        }
    }

    pub fn synchronize_camera(&mut self, frame_index: u32, amount: f32) {
        const CAMERA_DIRECTION: Vector3<f32> = Vector3::new(-1f32, 1f32, 1f32);
        let mut camera0 = self.camera.clone();
        camera0.synchronize_parameters(&self.camera_motion, frame_index, self);
        let look_at0 = camera0.look_at(self.active_model());
        if amount > 0f32
            && self
                .camera_motion
                .find_camera_keyframe(frame_index + 1)
                .is_none()
        {
            let mut camera1 = self.camera.clone();
            camera1.synchronize_parameters(&self.camera_motion, frame_index, self);
            let look_at1 = camera1.look_at(self.active_model());
            self.camera.set_angle(
                camera0
                    .angle()
                    .lerp(camera1.angle(), amount)
                    .mul_element_wise(CAMERA_DIRECTION),
            );
            self.camera
                .set_distance(lerp_f32(camera0.distance(), camera1.distance(), amount));
            self.camera.set_fov_radians(lerp_f32(
                camera0.fov_radians(),
                camera1.fov_radians(),
                amount,
            ));
            self.camera.set_look_at(look_at0.lerp(look_at1, amount));
        } else {
            self.camera
                .set_angle(camera0.angle().mul_element_wise(CAMERA_DIRECTION));
            self.camera.set_distance(camera0.distance());
            self.camera.set_fov_radians(camera0.fov_radians());
            self.camera.set_look_at(look_at0);
        }
        self.camera.set_perspective(camera0.is_perspective());
        self.camera.interpolation = camera0.interpolation;
        let bound_look_at = self.camera.bound_look_at(self);
        self.camera.update(self.viewport_size.0, bound_look_at);
        self.camera.set_dirty(false);
    }

    pub fn synchronize_light(&mut self, frame_index: u32, amount: f32) {
        let mut light0 = self.light.clone();
        light0.synchronize_parameters(&self.light_motion, frame_index);
        if amount > 0f32 {
            let mut light1 = self.light.clone();
            light1.synchronize_parameters(&self.light_motion, frame_index + 1);
            self.light
                .set_color(light0.color().lerp(light1.color(), amount));
            self.light
                .set_direction(light0.direction().lerp(light1.direction(), amount));
        } else {
            self.light.set_color(light0.color());
            self.light.set_direction(light0.direction());
        }
        self.light.set_dirty(false);
    }

    pub fn synchronize_self_shadow(&mut self, frame_index: u32, amount: f32) {
        if let Some(keyframe) = self
            .self_shadow_motion
            .find_self_shadow_keyframe(frame_index)
        {
            self.shadow_camera.set_distance(keyframe.distance);
            self.shadow_camera
                .set_coverage_mode((keyframe.mode as u32).into());
            self.shadow_camera.set_dirty(false);
        }
    }

    pub fn mark_all_models_dirty(&mut self) {
        for (_, model) in &mut self.model_handle_map {
            model.mark_staging_vertex_buffer_dirty();
        }
        // TODO: mark blitter dirty
    }
}

impl Project {
    pub fn perform_model_bones_transform(&mut self, model: Option<ModelHandle>) {
        let physics_simulation_time_step = self.physics_simulation_time_step();
        let outside_parent_bone_map = HashMap::new();
        if let Some(model) = model
            .or_else(|| self.active_model_pair.0)
            .and_then(|handle| self.model_handle_map.get_mut(&handle))
        {
            model.perform_all_bones_transform(
                &mut self.physics_engine,
                physics_simulation_time_step,
                &outside_parent_bone_map,
            );
        }
    }

    pub fn register_bone_keyframes(
        &mut self,
        model: Option<ModelHandle>,
        bones: &HashMap<String, Vec<u32>>,
    ) {
        self.reset_transform_performed_at();
        if let Some((handle, model)) =
            model
                .or_else(|| self.active_model_pair.0)
                .and_then(|handle| {
                    self.model_handle_map
                        .get_mut(&handle)
                        .map(|model| (handle, model))
                })
        {
            if let Some(motion) = self.model_to_motion.get_mut(&handle) {
                let mut updaters = motion.build_add_bone_keyframes_updaters(
                    model,
                    bones,
                    self.state_flags.enable_bezier_curve_adjustment,
                    self.state_flags.enable_physics_simulation_for_bone_keyframe,
                );
                motion.apply_add_bone_keyframes_updaters(model, &mut updaters);
            }
        }
    }
}

impl Project {
    pub fn load_model(&mut self, model_data: &[u8], device: &wgpu::Device, queue: &wgpu::Queue) {
        if let Ok(model) = Model::new_from_bytes(
            model_data,
            self.parse_language(),
            &mut self.physics_engine,
            &self.camera,
            &self.fallback_texture,
            &self.shared_sampler,
            &self.texture_bind_group_layout,
            device,
            queue,
        ) {
            let handle = self.add_model(model);
            self.set_active_model(Some(handle));
        }
    }

    pub fn add_model(&mut self, mut model: Model) -> ModelHandle {
        model.clear_all_bone_bounds_rigid_bodies();
        if !self.state_flags.disable_hidden_bone_bounds_rigid_body {
            model.create_all_bone_bounds_rigid_bodies();
        }
        let model_handle = self.object_handler_allocator.next();
        self.model_handle_map.insert(model_handle, model);
        self.transform_model_order_list.push(model_handle);
        // TODO: add effect to kScriptOrderTypeStandard
        // TODO: publish event
        let motion = Motion::empty();
        // TODO: clear model undo stack
        self.add_model_motion(motion, model_handle);
        // TODO: applyAllOffscreenRenderTargetEffectsToDrawable
        model_handle
    }

    pub fn load_model_motion(&mut self, motion_data: &[u8]) -> Result<(), Error> {
        if self.active_model().is_some() {
            Motion::new_from_bytes(motion_data, self.local_frame_index.0).and_then(|motion| {
                if motion.opaque.target_model_name == Motion::CAMERA_AND_LIGHT_TARGET_MODEL_NAME {
                    return Err(Error::new(
                        "読み込まれたモーションはモデル用ではありません",
                        "",
                        crate::error::DomainType::DomainTypeApplication,
                    ));
                }
                // TODO: record history in motion redo
                let (missing_bones, missing_morphs) =
                    motion.test_all_missing_model_objects(self.active_model().unwrap());
                if missing_bones.len() > 0 && missing_morphs.len() > 0 {
                    // TODO: Dialog hint motion missing
                }
                // TODO: add all to motion selection
                let _ = self.add_model_motion(motion, self.active_model_pair.0.unwrap());
                self.restart_from_current();
                Ok(())
            })
        } else {
            Err(Error::new(
                "モデルモーションを読み込むためのモデルが選択されていません",
                "モデルを選択してください",
                crate::error::DomainType::DomainTypeApplication,
            ))
        }
    }

    pub fn load_camera_motion(&mut self, motion_data: &[u8]) -> Result<(), Error> {
        Motion::new_from_bytes(motion_data, self.local_frame_index.0).and_then(|motion| {
            if motion.opaque.target_model_name != Motion::CAMERA_AND_LIGHT_TARGET_MODEL_NAME {
                return Err(Error::new(
                    "読み込まれたモーションはカメラ及び照明用ではありません",
                    "",
                    crate::error::DomainType::DomainTypeApplication,
                ));
            }
            // TODO: record history in motion redo
            let _ = self.set_camera_motion(motion);
            Ok(())
        })
    }

    pub fn load_light_motion(&mut self, motion_data: &[u8]) -> Result<(), Error> {
        Motion::new_from_bytes(motion_data, self.local_frame_index.0).and_then(|motion| {
            if motion.opaque.target_model_name != Motion::CAMERA_AND_LIGHT_TARGET_MODEL_NAME {
                return Err(Error::new(
                    "読み込まれたモーションはカメラ及び照明用ではありません",
                    "",
                    crate::error::DomainType::DomainTypeApplication,
                ));
            }
            // TODO: record history in motion redo
            let _ = self.set_light_motion(motion);
            Ok(())
        })
    }

    pub fn add_model_motion(&mut self, mut motion: Motion, model: ModelHandle) -> Option<Motion> {
        let last_model_motion = self
            .model_to_motion
            .get(&model)
            .map(|motion| motion.clone());
        if let Some(last_model_motion) = last_model_motion.as_ref() {
            if self.state_flags.enable_motion_merge {
                motion.merge_all_keyframes(last_model_motion);
            }
        }
        if let Some(model_object) = self.model_handle_map.get(&model) {
            motion.initialize_model_frame_0(model_object);
            // TODO: clear model undo stack
            self.model_to_motion.insert(model, motion);
            self.set_base_duration(self.project_duration());
            // TODO: publish add motion event
            return last_model_motion;
        }
        return None;
    }

    pub fn set_camera_motion(&mut self, mut motion: Motion) -> Motion {
        let last_motion = self.camera_motion.clone();
        self.camera_motion = motion;
        self.set_base_duration(self.project_duration());
        let active_model = self
            .active_model_pair
            .0
            .and_then(|idx| self.model_handle_map.get(&idx));
        self.camera_motion
            .initialize_camera_frame_0(&self.camera, active_model);
        self.synchronize_camera(self.local_frame_index.0, 0f32);
        // TODO: publish motion event
        last_motion
    }

    pub fn set_light_motion(&mut self, mut motion: Motion) -> Motion {
        let last_motion = self.light_motion.clone();
        self.light_motion = motion;
        self.set_base_duration(self.project_duration());
        self.light_motion.initialize_light_frame_0(&self.light);
        self.synchronize_light(self.local_frame_index.0, 0f32);
        // TODO: publish motion event
        last_motion
    }

    pub fn restart_from_current(&mut self) {
        self.restart(self.local_frame_index.0);
    }

    fn restart(&mut self, frame_index: u32) {
        self.synchronize_all_motions(frame_index, 0f32, SimulationTiming::Before);
        for (handle, model) in &mut self.model_handle_map {
            model.initialize_all_rigid_bodies_transform_feedback(&mut self.physics_engine);
            // TODO: soft_bodies
        }
        self.internal_perform_physics_simulation(self.physics_simulation_time_step());
        self.synchronize_all_motions(frame_index, 0f32, SimulationTiming::After);
        self.mark_all_models_dirty();
    }

    fn internal_perform_physics_simulation(&mut self, delta: f32) {
        if self.is_physics_simulation_enabled() {
            self.physics_engine.step(delta);
            for (_, model) in &mut self.model_handle_map {
                if model.is_physics_simulation_enabled() {
                    model.synchronize_all_rigid_bodies_transform_feedback_from_simulation(
                        RigidBodyFollowBone::Perform,
                        &mut self.physics_engine,
                    );
                }
            }
        }
    }

    pub fn load_texture(
        &mut self,
        key: &str,
        data: &[u8],
        dimensions: (u32, u32),
        update_bind: bool,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let texture_size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };
        let texture = self
            .tmp_texture_map
            .entry(key.to_owned())
            .or_insert_with(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(format!("Texture/{}", key).as_str()),
                    size: texture_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                })
            });
        // TODO: may have different size when different image with same name
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(4 * dimensions.0),
                rows_per_image: std::num::NonZeroU32::new(dimensions.1),
            },
            wgpu::Extent3d {
                width: dimensions.0,
                height: dimensions.1,
                depth_or_array_layers: 1,
            },
        );
        if update_bind {
            for (handle, model) in &mut self.model_handle_map {
                model.update_image(
                    key,
                    texture,
                    &self.shared_sampler,
                    &self.texture_bind_group_layout,
                    &self.fallback_texture,
                    device,
                )
            }
        }
    }

    pub fn update_bind_texture(&mut self, device: &wgpu::Device) {
        for (handle, model) in &mut self.model_handle_map {
            model.create_all_images(
                &self.tmp_texture_map,
                &self.shared_sampler,
                &self.texture_bind_group_layout,
                &self.fallback_texture,
                device,
            );
        }
    }

    fn create_white_fallback_image(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        Self::create_fallback_image([0xffu8, 0xffu8, 0xffu8, 0xffu8], device, queue)
    }

    fn create_black_fallback_image(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        Self::create_fallback_image([0x00u8, 0x00u8, 0x00u8, 0xffu8], device, queue)
    }

    fn create_fallback_image(
        data: [u8; 4],
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> wgpu::Texture {
        let texture_size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("@mdanceio/FallbackImage"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &fallback_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(4),
                rows_per_image: std::num::NonZeroU32::new(1),
            },
            texture_size,
        );
        fallback_texture
    }

    fn has_any_depends_on_script_external_effect(&self) -> bool {
        self.depends_on_script_external
            .iter()
            .any(|drawable| drawable.is_visible())
    }

    fn draw_all_effects_depends_on_script_external(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        // TODO: now use device from arg
        if self.has_any_depends_on_script_external_effect() {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            encoder.push_debug_group(
                format!(
                    "Project::drawDependsOnScriptExternal(size={})",
                    self.depends_on_script_external.len()
                )
                .as_str(),
            );
            for drawable in &self.depends_on_script_external {
                drawable.draw(
                    view,
                    Some(&self.viewport_primary_pass.depth_view),
                    DrawType::ScriptExternalColor,
                    DrawContext {
                        camera: &self.camera,
                        shadow: &self.shadow_camera,
                        light: &self.light,
                        shared_fallback_texture: &self.fallback_texture,
                        viewport_texture_format: self.viewport_texture_format(),
                        is_render_pass_viewport: self.is_render_pass_viewport(),
                        all_models: &self.model_handle_map.values(),
                        effect: &mut self.model_program_bundle,
                        texture_bind_layout: &self.texture_bind_group_layout,
                        shadow_bind_layout: &self.shadow_bind_group_layout,
                        texture_fallback_bind: &self.texture_fallback_bind,
                        shadow_fallback_bind: &self.shadow_fallback_bind,
                    },
                    device,
                    queue,
                );
            }
            encoder.pop_debug_group();
        }
    }

    fn clear_view_port_primary_pass(
        &self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.push_debug_group("Project::clearViewportPass");
        let depth_stencil_attachment_view = &self.viewport_primary_pass.depth_view;
        {
            let pipeline = self.clear_pass.get_pipeline(
                &[self.viewport_primary_pass.color_texture_format],
                self.viewport_primary_pass.depth_texture_format,
                device,
            );
            let mut _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("@mdanceio/ClearRenderPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.viewport_background_color[0] as f64,
                            g: self.viewport_background_color[1] as f64,
                            b: self.viewport_background_color[2] as f64,
                            a: self.viewport_background_color[3] as f64,
                        }),
                        store: true,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_stencil_attachment_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: true,
                    }),
                    // stencil_ops: Some(wgpu::Operations {
                    //     load: wgpu::LoadOp::Clear(1),
                    //     store: false,
                    // }),
                    stencil_ops: None,
                }),
            });
            _render_pass.set_vertex_buffer(0, self.clear_pass.vertex_buffer.slice(..));
            _render_pass.set_pipeline(&pipeline);
            _render_pass.draw(0..4, 0..1)
        }
        encoder.pop_debug_group();
        queue.submit(Some(encoder.finish()));
    }

    pub fn draw_grid(&self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.grid.draw(
            view,
            Some(&self.viewport_primary_depth_view()),
            self,
            device,
            queue,
        );
    }

    pub fn draw_shadow_map(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.shadow_camera.is_enabled() {
            let (light_view, light_projection) = self
                .shadow_camera
                .get_view_projection(&self.camera, &self.light);
            self.shadow_camera.clear(&self.clear_pass, device, queue);
            // if self.editing_mode != EditingMode::Select {
            // scope(m_currentOffscreenRenderPass, pass), scope2(m_originOffscreenRenderPass, pass)
            for (handle, drawable) in &self.model_handle_map {
                // TODO: judge effect script class
                let color_view = self.shadow_camera.color_image();
                let depth_view = self.shadow_camera.depth_image();
                drawable.draw(
                    color_view,
                    Some(depth_view),
                    DrawType::ShadowMap,
                    DrawContext {
                        camera: &self.camera,
                        shadow: &self.shadow_camera,
                        light: &self.light,
                        shared_fallback_texture: &self.fallback_texture,
                        viewport_texture_format: self.viewport_texture_format(),
                        is_render_pass_viewport: self.is_render_pass_viewport(),
                        all_models: &self.model_handle_map.values(),
                        effect: &mut self.model_program_bundle,
                        texture_bind_layout: &self.texture_bind_group_layout,
                        shadow_bind_layout: &self.shadow_bind_group_layout,
                        texture_fallback_bind: &self.texture_fallback_bind,
                        shadow_fallback_bind: &self.shadow_fallback_bind,
                    },
                    device,
                    queue,
                )
            }
            // }
        }
    }

    pub fn draw_viewport(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        log::info!("Start drawing viewport");
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.push_debug_group("Project::draw_viewport");
        let is_drawing_color_type = self.draw_type == DrawType::Color;
        let (view_matrix, projection_matrix) = self.active_camera().get_view_transform();
        self.draw_all_effects_depends_on_script_external(view, device, queue);
        self.clear_view_port_primary_pass(view, device, queue);
        if is_drawing_color_type {
            self.draw_grid(view, device, queue);
        }
        self._draw_viewport(ScriptOrder::Standard, self.draw_type, view, device, queue);
        if is_drawing_color_type {
            // TODO: 渲染前边部分
        }
        self.local_frame_index.1 = 0;
        encoder.pop_debug_group();
        queue.submit(Some(encoder.finish()));
        log::info!("Submit new viewport task");
        queue.on_submitted_work_done(|| {
            log::info!("Submission finished");
        })
    }

    pub fn draw_viewport_with_depth(
        &mut self,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        log::info!("Start drawing viewport");
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.push_debug_group("Project::draw_viewport");
        let is_drawing_color_type = self.draw_type == DrawType::Color;
        let (view_matrix, projection_matrix) = self.active_camera().get_view_transform();
        self.draw_all_effects_depends_on_script_external(view, device, queue);
        self.clear_view_port_primary_pass(view, device, queue);
        if is_drawing_color_type {
            self.draw_grid(view, device, queue);
        }
        self._draw_viewport_with_depth(
            ScriptOrder::Standard,
            self.draw_type,
            view,
            depth_view,
            device,
            queue,
        );
        if is_drawing_color_type {
            // TODO: 渲染前边部分
        }
        self.local_frame_index.1 = 0;
        encoder.pop_debug_group();
        queue.submit(Some(encoder.finish()));
        log::info!("Submit new viewport task");
        queue.on_submitted_work_done(|| {
            log::info!("Submission finished");
        })
    }

    fn _draw_viewport(
        &mut self,
        order: ScriptOrder,
        typ: DrawType,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        log::info!("Start internal drawing viewport");
        for (handle, drawable) in &self.model_handle_map {
            drawable.draw(
                view,
                Some(&self.viewport_primary_pass.depth_view),
                typ,
                DrawContext {
                    camera: &self.camera,
                    shadow: &self.shadow_camera,
                    light: &self.light,
                    shared_fallback_texture: &self.fallback_texture,
                    viewport_texture_format: self.viewport_texture_format(),
                    is_render_pass_viewport: self.is_render_pass_viewport(),
                    all_models: &self.model_handle_map.values(),
                    effect: &mut self.model_program_bundle,
                    texture_bind_layout: &self.texture_bind_group_layout,
                    shadow_bind_layout: &self.shadow_bind_group_layout,
                    texture_fallback_bind: &self.texture_fallback_bind,
                    shadow_fallback_bind: &self.shadow_fallback_bind,
                },
                device,
                queue,
            );
        }
    }

    fn _draw_viewport_with_depth(
        &mut self,
        order: ScriptOrder,
        typ: DrawType,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        log::info!("Start internal drawing viewport");
        for (handle, drawable) in &self.model_handle_map {
            drawable.draw(
                view,
                Some(depth_view),
                typ,
                DrawContext {
                    camera: &self.camera,
                    shadow: &self.shadow_camera,
                    light: &self.light,
                    shared_fallback_texture: &self.fallback_texture,
                    viewport_texture_format: self.viewport_texture_format(),
                    is_render_pass_viewport: self.is_render_pass_viewport(),
                    all_models: &self.model_handle_map.values(),
                    effect: &mut self.model_program_bundle,
                    texture_bind_layout: &self.texture_bind_group_layout,
                    shadow_bind_layout: &self.shadow_bind_group_layout,
                    texture_fallback_bind: &self.texture_fallback_bind,
                    shadow_fallback_bind: &self.shadow_fallback_bind,
                },
                device,
                queue,
            );
        }
    }
}
