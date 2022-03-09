use std::{cell::RefCell, collections::HashMap, mem, ops::Deref, rc::Rc};

use bytemuck::Zeroable;
use cgmath::{Matrix4, Vector4};
use nanoem::model::ModelMaterialSphereMapTextureType;
use wgpu::util::DeviceExt;

use crate::{
    camera::Camera,
    drawable::Drawable,
    image_view::ImageView,
    light::Light,
    model::{Model, NanoemMaterial},
    pass,
    project::Project,
    shadow_camera::ShadowCamera,
};

// enum UniformBuffer {
//     ModelMatrix = 0,
//     ModelViewMatrix = 4,
//     ModelViewProjectionMatrix = 8,
//     LightViewProjectionMatrix = 12,
//     LightColor = 16,
//     LightDirection,
//     CameraPosition,
//     MaterialAmbient,
//     MaterialDiffuse,
//     MaterialSpecular,
//     EnableVertexColor,
//     DiffuseTextureBlendFactor,
//     SphereTextureBlendFactor,
//     ToonTextureBlendFactor,
//     UseTextureSampler,
//     SphereTextureType,
//     ShadowMapSize,
// }

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ModelParametersUniform {
    model_matrix: [[f32; 4]; 4],
    model_view_matrix: [[f32; 4]; 4],
    model_view_projection_matrix: [[f32; 4]; 4],
    light_view_projection_matrix: [[f32; 4]; 4],
    light_color: [f32; 4],
    light_direction: [f32; 4],
    camera_position: [f32; 4],
    material_ambient: [f32; 4],
    material_diffuse: [f32; 4],
    material_specular: [f32; 4],
    enable_vertex_color: [f32; 4],
    diffuse_texture_blend_factor: [f32; 4],
    sphere_texture_blend_factor: [f32; 4],
    toon_texture_blend_factor: [f32; 4],
    use_texture_sampler: [f32; 4],
    sphere_texture_type: [f32; 4],
    shadow_map_size: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSamplerStage {
    ShadowMapTextureSamplerStage0,
    DiffuseTextureSamplerStage,
    SphereTextureSamplerStage,
    ToonTextureSamplerStage,
}

pub struct CommonPass {
    // TODO: uncompleted
    uniform_buffer: ModelParametersUniform,
    pipelines: HashMap<u32, wgpu::RenderPipeline>,
    bindings: HashMap<TextureSamplerStage, wgpu::TextureView>,
    cull_mode: Option<wgpu::Face>,
    primitive_type: wgpu::PrimitiveTopology,
    opacity: f32,
}

impl CommonPass {
    pub fn new() -> Self {
        Self {
            uniform_buffer: ModelParametersUniform::zeroed(),
            pipelines: HashMap::new(),
            bindings: HashMap::new(),
            cull_mode: None,
            primitive_type: wgpu::PrimitiveTopology::TriangleList,
            opacity: 1.0f32,
        }
    }
}

impl CommonPass {
    pub fn set_global_parameters(&mut self, _drawable: &impl Drawable, _project: &Project) {}

    pub fn set_camera_parameters(
        &mut self,
        camera: &dyn Camera,
        world: &Matrix4<f32>,
        model: &Model,
    ) {
        let (v, p) = camera.get_view_transform();
        let w = model.world_transform(world);
        self.uniform_buffer.model_matrix = w.into();
        self.uniform_buffer.model_view_matrix = (v * w).into();
        self.uniform_buffer.model_view_projection_matrix = (p * v * w).into();
        self.uniform_buffer.camera_position = camera.position().extend(0f32).into();
    }

    pub fn set_light_parameters(&mut self, light: &dyn Light, _adjustment: bool) {
        self.uniform_buffer.light_color = light.color().extend(1f32).into();
        self.uniform_buffer.light_direction = light.direction().extend(0f32).into();
    }

    pub fn set_all_model_parameters(&mut self, model: &Model, _project: &Project) {
        self.opacity = model.opacity();
    }

    pub fn set_material_parameters(
        &mut self,
        nanoem_material: &NanoemMaterial,
        technique_type: TechniqueType,
        fallback: &wgpu::Texture,
    ) {
        if let Some(material) = nanoem_material.get_user_data() {
            let material = material.borrow();
            let color = material.color();
            self.uniform_buffer.material_ambient = color.ambient.extend(1.0f32).into();
            self.uniform_buffer.material_diffuse = color
                .diffuse
                .extend(color.diffuse_opacity * self.opacity)
                .into();
            self.uniform_buffer.material_specular =
                color.specular.extend(color.specular_power).into();
            self.uniform_buffer.diffuse_texture_blend_factor =
                color.diffuse_texture_blend_factor.into();
            self.uniform_buffer.sphere_texture_blend_factor =
                color.sphere_texture_blend_factor.into();
            self.uniform_buffer.toon_texture_blend_factor = color.toon_texture_blend_factor.into();
            let texture_type = if material.sphere_map_image().is_some() {
                nanoem_material.get_spheremap_texture_type()
            } else {
                ModelMaterialSphereMapTextureType::TypeNone
            };
            let sphere_texture_type = [
                if texture_type == ModelMaterialSphereMapTextureType::TypeMultiply {
                    1.0f32
                } else {
                    0.0f32
                },
                if texture_type == ModelMaterialSphereMapTextureType::TypeSubTexture {
                    1.0f32
                } else {
                    0.0f32
                },
                if texture_type == ModelMaterialSphereMapTextureType::TypeAdd {
                    1.0f32
                } else {
                    0.0f32
                },
                0f32,
            ];
            self.uniform_buffer.sphere_texture_type = sphere_texture_type;
            let enable_vertex_color = if nanoem_material.is_vertex_color_enabled() {
                1.0f32
            } else {
                0.0f32
            };
            self.uniform_buffer.enable_vertex_color = Vector4::new(
                enable_vertex_color,
                enable_vertex_color,
                enable_vertex_color,
                enable_vertex_color,
            )
            .into();
            self.uniform_buffer.use_texture_sampler[0] = if self.set_image(
                material.diffuse_image(),
                TextureSamplerStage::DiffuseTextureSamplerStage,
                fallback,
            ) {
                1.0f32
            } else {
                0.0f32
            };
            self.uniform_buffer.use_texture_sampler[1] = if self.set_image(
                material.sphere_map_image(),
                TextureSamplerStage::SphereTextureSamplerStage,
                fallback,
            ) {
                1.0f32
            } else {
                0.0f32
            };
            self.uniform_buffer.use_texture_sampler[2] = if self.set_image(
                material.toon_image(),
                TextureSamplerStage::ToonTextureSamplerStage,
                fallback,
            ) {
                1.0f32
            } else {
                0.0f32
            };
            if nanoem_material.is_line_draw_enabled() {
                self.primitive_type = wgpu::PrimitiveTopology::LineList;
            } else if nanoem_material.is_point_draw_enabled() {
                self.primitive_type = wgpu::PrimitiveTopology::PointList;
            } else {
                self.primitive_type = wgpu::PrimitiveTopology::TriangleList;
            }
            self.cull_mode = match technique_type {
                TechniqueType::Color | TechniqueType::Zplot => {
                    if nanoem_material.is_culling_disabled() {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    }
                }
                TechniqueType::Edge => Some(wgpu::Face::Front),
                TechniqueType::GroundShadow => None,
            }
        }
    }

    pub fn set_edge_parameters(
        &mut self,
        nanoem_material: &NanoemMaterial,
        edge_size: f32,
        fallback: &wgpu::Texture,
    ) {
        if let Some(material) = nanoem_material.get_user_data() {
            let material = material.borrow();
            let edge = material.edge();
            let edge_color = edge.color.extend(edge.opacity);
            self.uniform_buffer.light_color = edge_color.into();
            self.uniform_buffer.light_direction =
                (Vector4::new(1f32, 1f32, 1f32, 1f32) * edge.size * edge_size).into();
            self.bindings.insert(
                TextureSamplerStage::ShadowMapTextureSamplerStage0,
                fallback.create_view(&wgpu::TextureViewDescriptor::default()),
            );
        }
    }

    pub fn set_ground_shadow_parameters(
        &mut self,
        light: &impl Light,
        camera: &impl Camera,
        world: &Matrix4<f32>,
        fallback: &wgpu::Texture,
    ) {
        let (view_matrix, projection_matrix) = camera.get_view_transform();
        let origin_shadow_matrix = light.get_shadow_transform();
        let shadow_matrix = origin_shadow_matrix * world;
        self.uniform_buffer.model_matrix = shadow_matrix.into();
        let shadow_view_matrix = view_matrix * shadow_matrix;
        self.uniform_buffer.model_view_matrix = shadow_view_matrix.into();
        let shadow_view_projection_matrix = projection_matrix * shadow_view_matrix;
        self.uniform_buffer.model_view_projection_matrix = shadow_view_projection_matrix.into();
        self.uniform_buffer.light_color = light
            .ground_shadow_color()
            .extend(
                1.0f32
                    + if light.is_translucent_ground_shadow_enabled() {
                        -0.5f32
                    } else {
                        0f32
                    },
            )
            .into();
        self.bindings.insert(
            TextureSamplerStage::ShadowMapTextureSamplerStage0,
            fallback.create_view(&wgpu::TextureViewDescriptor::default()),
        );
    }

    pub fn set_shadow_map_parameters(
        &mut self,
        shadow_camera: &ShadowCamera,
        world: &Matrix4<f32>,
        project: &Project,
        technique_type: TechniqueType,
        fallback: &wgpu::Texture,
    ) {
        let (view, projection) = shadow_camera.get_view_projection(project);
        let crop = shadow_camera.get_crop_matrix(project.adapter_info().backend);
        let shadow_map_matrix = projection * view * world;
        self.uniform_buffer.light_view_projection_matrix = (crop * shadow_map_matrix).into();
        self.uniform_buffer.shadow_map_size = shadow_camera
            .image_size()
            .map(|x| x as f32)
            .extend(0.005f32)
            .extend(u32::from(shadow_camera.coverage_mode()) as f32)
            .into();
        self.uniform_buffer.use_texture_sampler[3] = if shadow_camera.is_enabled() {
            1.0f32
        } else {
            0f32
        };
        let color_image = match technique_type {
            TechniqueType::Zplot => {
                self.bindings.insert(
                    TextureSamplerStage::DiffuseTextureSamplerStage,
                    fallback.create_view(&wgpu::TextureViewDescriptor::default()),
                );
                self.bindings.insert(
                    TextureSamplerStage::SphereTextureSamplerStage,
                    fallback.create_view(&wgpu::TextureViewDescriptor::default()),
                );
                self.bindings.insert(
                    TextureSamplerStage::ToonTextureSamplerStage,
                    fallback.create_view(&wgpu::TextureViewDescriptor::default()),
                );
                self.uniform_buffer.model_view_projection_matrix = shadow_map_matrix.into();
                fallback.create_view(&wgpu::TextureViewDescriptor::default())
            }
            _ => shadow_camera
                .color_image()
                .create_view(&wgpu::TextureViewDescriptor::default()),
        };
        self.bindings.insert(
            TextureSamplerStage::ShadowMapTextureSamplerStage0,
            color_image,
        );
    }

    // TODO: process with feature
    // #[cfg(target_feature = "enable_blendop_minmax")]
    fn get_add_blend_state(&self) -> (wgpu::BlendState, wgpu::ColorWrites) {
        (
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add, // default
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Max,
                },
            },
            wgpu::ColorWrites::ALL,
        )
    }

    // TODO: process with feature
    // #[cfg(target_feature = "enable_blendop_minmax")]
    fn get_alpha_blend_state(&self) -> (wgpu::BlendState, wgpu::ColorWrites) {
        (
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add, // default
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Max,
                },
            },
            wgpu::ColorWrites::ALL,
        )
    }

    pub fn execute(
        &mut self,
        buffer: &pass::Buffer,
        color_attachment_view: &wgpu::TextureView,
        depth_stencil_attachment_view: Option<&wgpu::TextureView>,
        shader: Option<&wgpu::ShaderModule>,
        technique_type: TechniqueType,
        device: &wgpu::Device,
        model: &Model,
        project: &Project,
    ) {
        if let Some(shader) = shader {
            let vertex_size = mem::size_of::<crate::model::VertexUnit>();
            let is_add_blend = model.is_add_blend_enabled();
            let is_depth_enabled = buffer.is_depth_enabled();

            let texture_bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("ModelProgramBundle/BindGroupLayout/Texture"),
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
                        wgpu::BindGroupLayoutEntry {
                            binding: 6,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 7,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
            let uniform_bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("ModelProgramBundle/BindGroupLayout/Uniform"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

            let render_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("ModelProgramBundle/PipelineLayout"),
                    bind_group_layouts: &[&texture_bind_group_layout, &uniform_bind_group_layout],
                    push_constant_ranges: &[],
                });
            // No Difference between technique type edge and other.
            let vertex_buffer_layout = wgpu::VertexBufferLayout {
                array_stride: vertex_size as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4],
            };
            // Project::setStandardDepthStencilState(desc.depth, desc.stencil);
            // in origin project
            let (blend_state, mut write_mask) = if is_add_blend {
                self.get_add_blend_state()
            } else {
                self.get_alpha_blend_state()
            };
            if project.is_render_pass_viewport() {
                write_mask = wgpu::ColorWrites::ALL
            };
            let color_target_state = wgpu::ColorTargetState {
                format: project.config().format,
                blend: Some(blend_state),
                write_mask,
            };
            let depth_state = if technique_type == TechniqueType::GroundShadow && is_depth_enabled {
                wgpu::DepthStencilState {
                    format: project.config().format, // TODO: set to depth pixel format
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState {
                        front: wgpu::StencilFaceState {
                            compare: wgpu::CompareFunction::Greater,
                            fail_op: wgpu::StencilOperation::default(),
                            depth_fail_op: wgpu::StencilOperation::default(),
                            pass_op: wgpu::StencilOperation::Replace,
                        },
                        back: wgpu::StencilFaceState {
                            compare: wgpu::CompareFunction::Greater,
                            fail_op: wgpu::StencilOperation::default(),
                            depth_fail_op: wgpu::StencilOperation::default(),
                            pass_op: wgpu::StencilOperation::Replace,
                        },
                        read_mask: 0,
                        write_mask: 0, // TODO: there was a ref=2 in original stencil state
                    },
                    bias: wgpu::DepthBiasState::default(),
                }
            } else {
                wgpu::DepthStencilState {
                    format: project.config().format, // TODO: set to depth pixel format
                    depth_write_enabled: is_depth_enabled,
                    depth_compare: if is_depth_enabled {
                        wgpu::CompareFunction::LessEqual
                    } else {
                        wgpu::CompareFunction::Always
                    },
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }
            };
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ModelProgramBundle/Pipelines"),
                layout: Some(&render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: "vs_main",
                    buffers: &[vertex_buffer_layout],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: "fs_main",
                    targets: &[color_target_state],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: self.primitive_type,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: self.cull_mode,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(depth_state),
                multisample: wgpu::MultisampleState {
                    count: 1, // TODO: be configured by pixel format
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
            });
            let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ModelProgramBundle/BindGroupBuffer/Uniform"),
                contents: bytemuck::bytes_of(&[self.uniform_buffer]),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            });
            let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ModelProgramBundle/BindGroup/Uniform"),
                layout: &uniform_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Model Pass Executor Encoder"),
            });
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Model Pass Render Pass"),
                    color_attachments: &[wgpu::RenderPassColorAttachment {
                        view: color_attachment_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: true,
                        },
                    }],
                    depth_stencil_attachment: depth_stencil_attachment_view.map(|view| {
                        wgpu::RenderPassDepthStencilAttachment {
                            view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: true,
                            }),
                            stencil_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: true,
                            }),
                        }
                    }), // TODO: there should be depth view
                });
                // m_lastDrawnRenderPass = handle;
                rpass.set_pipeline(&pipeline);
                rpass.set_bind_group(1, &uniform_bind_group, &[]);
                rpass.set_vertex_buffer(0, buffer.vertex_buffer.slice(..));
                rpass.set_index_buffer(buffer.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            }
        };
    }

    pub fn set_image(
        &mut self,
        value: Option<Rc<RefCell<dyn ImageView>>>,
        stage: TextureSamplerStage,
        fallback: &wgpu::Texture,
    ) -> bool {
        self.bindings.insert(
            stage,
            value.clone().map_or(
                fallback.create_view(&wgpu::TextureViewDescriptor::default()),
                |rc| {
                    rc.borrow()
                        .handle()
                        .create_view(&wgpu::TextureViewDescriptor::default())
                },
            ),
        );
        value.is_some()
    }
}

pub struct ModelProgramBundle {
    // TODO
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TechniqueType {
    Color,
    Edge,
    GroundShadow,
    Zplot,
}

pub struct BaseTechnique {
    technique_type: TechniqueType,
    executed: bool,
    shader: Option<wgpu::ShaderModule>,
    pass: CommonPass,
}

impl BaseTechnique {
    pub fn shader(&self) -> Option<&wgpu::ShaderModule> {
        (&self.shader).as_ref()
    }

    pub fn get_mut_pass_and_shader(&mut self) -> (&mut CommonPass, Option<&wgpu::ShaderModule>) {
        (&mut self.pass, (&self.shader).as_ref())
    }
}

pub struct ObjectTechnique {
    base: BaseTechnique,
    is_point_draw_enabled: bool,
}

impl Deref for ObjectTechnique {
    type Target = BaseTechnique;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl ObjectTechnique {
    pub fn new(is_point_draw_enabled: bool) -> Self {
        Self {
            is_point_draw_enabled,
            base: BaseTechnique {
                technique_type: TechniqueType::Color,
                executed: false,
                shader: None,
                pass: CommonPass::new(),
            },
        }
    }
}

impl ObjectTechnique {
    pub fn technique_type(&self) -> TechniqueType {
        self.base.technique_type
    }

    pub fn execute(
        &mut self,
        device: &wgpu::Device,
    ) -> Option<(&mut CommonPass, Option<&wgpu::ShaderModule>)> {
        if !self.base.executed {
            if self.base.shader.is_none() {
                let sd = &wgpu::ShaderModuleDescriptor {
                    label: Some("ModelProgramBundle/ObjectTechnique/ModelColor"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("resources/shaders/model_color.wgsl").into(),
                    ),
                };
                self.base.shader = Some(device.create_shader_module(sd));
            }
            self.base.executed = true;
            Some(self.base.get_mut_pass_and_shader())
        } else {
            None
        }
    }
}
