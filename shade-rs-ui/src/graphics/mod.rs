pub mod backend;

use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::{
        Debug,
        Display,
    },
    num::NonZeroU32,
    sync::{
        atomic::{
            AtomicU32,
            Ordering,
        },
        Arc,
    },
    time::Duration,
};

use bytemuck::{
    Pod,
    Zeroable,
};
use serde::{
    Deserialize,
    Serialize,
};
use tokio::sync::{
    mpsc,
    oneshot,
};
use web_sys::HtmlCanvasElement;

use crate::{
    graphics::backend::{
        Backend,
        BackendType,
    },
    utils::{
        futures::spawn_local_and_handle_error,
        time::{
            interval,
            Instant,
            Interval,
            TicksPerSecond,
        },
    },
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no backends")]
    NoBackends,

    #[error("no adapter")]
    NoAdapter,

    #[error("failed to request device")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub power_preference: wgpu::PowerPreference,
    pub backend_type: SelectBackendType,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectBackendType {
    #[default]
    AutoDetect,
    Select(BackendType),
}

#[derive(Clone, Debug)]
pub struct Graphics {
    tx_command: mpsc::UnboundedSender<Command>,
}

impl Graphics {
    pub fn new(config: Config) -> Self {
        tracing::debug!(?config, "initializing graphics");

        let (tx_command, rx_command) = mpsc::unbounded_channel();

        spawn_local_and_handle_error(async move {
            let reactor = Reactor::new(config, rx_command).await?;
            reactor.run().await
        });

        Self { tx_command }
    }

    fn send_command(&self, command: Command) {
        self.tx_command
            .send(command)
            .expect("graphics reactor died");
    }

    pub fn register_window(
        &self,
        window_id: WindowId,
        surface_size: SurfaceSize,
        on_frame: Box<dyn FnMut(FrameInfo) + 'static>,
    ) -> WindowHandle {
        self.send_command(Command::RegisterWindow {
            window_id,
            surface_size,
            on_frame,
        });

        WindowHandle {
            graphics: self.clone(),
            window_id,
        }
    }
}

struct Reactor {
    config: Config,
    backend_type: BackendType,
    shared_backend: Option<Backend>,
    rx_command: mpsc::UnboundedReceiver<Command>,
    windows: HashMap<WindowId, Window>,
    render_interval: Interval,
}

impl Reactor {
    async fn new(
        config: Config,
        rx_command: mpsc::UnboundedReceiver<Command>,
    ) -> Result<Self, Error> {
        let (backend_type, shared_backend) = match config.backend_type {
            SelectBackendType::AutoDetect => {
                tracing::debug!("trying WEBGPU");
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::BROWSER_WEBGPU,
                    ..Default::default()
                });

                if let Ok(shared_backend) = Backend::new(Arc::new(instance), &config, None).await {
                    (BackendType::WebGpu, Some(shared_backend))
                }
                else {
                    tracing::info!("failed to initialize WEBGPU backend, falling back to WebGL");
                    (BackendType::WebGl, None)
                }
            }
            SelectBackendType::Select(backend_type) => {
                tracing::debug!(?backend_type, "initializing shared backend");
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: backend_type.as_wgpu(),
                    ..Default::default()
                });
                let shared_backend = Backend::new(Arc::new(instance), &config, None).await?;
                (backend_type, Some(shared_backend))
            }
        };

        Ok(Self {
            config,
            backend_type,
            shared_backend,
            rx_command,
            windows: HashMap::new(),
            render_interval: interval(Duration::from_millis(1000 / 60)),
        })
    }

    async fn run(mut self) -> Result<(), Error> {
        loop {
            tokio::select! {
                command_opt = self.rx_command.recv() => {
                    let Some(command) = command_opt else { break; };
                    self.handle_command(command).await?;
                }
                _ = self.render_interval.tick() => {
                    for window in self.windows.values_mut() {
                        if !window.paused {
                            window.update();
                        }
                        if window.visible {
                            window.render();
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_command(&mut self, command: Command) -> Result<(), Error> {
        match command {
            Command::RegisterWindow {
                window_id: window_handle,
                surface_size,
                on_frame,
            } => {
                self.create_window(window_handle, surface_size, on_frame)
                    .await?;
            }
            Command::DestroyWindow { window_id } => {
                self.windows.remove(&window_id);
            }
            Command::Resize {
                window_id,
                surface_size,
            } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.resize(surface_size);
                }
            }
            Command::Run {
                window_id,
                code,
                tx_result,
            } => {
                match compile_shader(&code) {
                    Ok(shader) => {
                        if let Some(window) = self.windows.get_mut(&window_id) {
                            window.create_pipeline(shader);
                            window.paused = false;
                        }
                        let _ = tx_result.send(Ok(()));
                    }
                    Err(error) => {
                        tracing::error!(?error);
                        let _ = tx_result.send(Err(error));
                    }
                }
            }
            Command::SetMousePosition {
                window_id,
                position,
            } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.mouse_position = position;
                }
            }
            Command::SetVisibility { window_id, visible } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.visible = visible;
                }
            }
            Command::SetPaused { window_id, paused } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    if !paused {
                        window.previous_frame_time = Instant::now();
                    }
                    window.paused = paused;
                }
            }
            Command::Reset { window_id } => {
                if let Some(window) = self.windows.get_mut(&window_id) {
                    window.previous_frame_time = Instant::now();
                    window.time = 0.0;
                    window.update();
                }
            }
        }

        Ok(())
    }

    async fn create_window(
        &mut self,
        window_id: WindowId,
        surface_size: SurfaceSize,
        on_frame: Box<dyn FnMut(FrameInfo) + 'static>,
    ) -> Result<(), Error> {
        tracing::info!(?window_id, ?surface_size, "creating surface");

        let (surface, backend) = if self.backend_type.uses_shared_backend() {
            let backend = self
                .shared_backend
                .as_ref()
                .expect("expected a shared backend for WebGPU backend");
            let surface = backend
                .instance
                .create_surface(window_id)
                .expect("failed to create surface");
            (surface, backend.clone())
        }
        else {
            tracing::debug!("creating WebGL instance");
            let instance = Arc::new(wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: self.backend_type.as_wgpu(),
                ..Default::default()
            }));

            let surface = instance
                .create_surface(window_id)
                .expect("failed to create surface");

            let backend = Backend::new(instance, &self.config, Some(&surface))
                .await
                .expect("todo: handle error");

            (surface, backend)
        };

        let surface_capabilities = surface.get_capabilities(&backend.adapter);

        let surface_format = surface_capabilities
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_capabilities.formats[0]);

        let surface_configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: surface_size.width,
            height: surface_size.height,
            present_mode: surface_capabilities.present_modes[0],
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
        };

        surface.configure(&backend.device, &surface_configuration);

        self.windows.insert(
            window_id,
            Window {
                backend,
                surface,
                surface_configuration,
                pipeline: None,
                mouse_position: None,
                visible: true,
                on_frame,
                paused: false,
                previous_frame_time: Instant::now(),
                time: 0.0,
                fps: TicksPerSecond::new(30),
                input_uniform: InputUniform::default(),
            },
        );

        Ok(())
    }
}

enum Command {
    RegisterWindow {
        window_id: WindowId,
        surface_size: SurfaceSize,
        on_frame: Box<dyn FnMut(FrameInfo) + 'static>,
    },
    DestroyWindow {
        window_id: WindowId,
    },
    Resize {
        window_id: WindowId,
        surface_size: SurfaceSize,
    },
    Run {
        window_id: WindowId,
        code: String,
        tx_result: oneshot::Sender<Result<(), CompileError>>,
    },
    SetMousePosition {
        window_id: WindowId,
        position: Option<[f32; 2]>,
    },
    SetVisibility {
        window_id: WindowId,
        visible: bool,
    },
    SetPaused {
        window_id: WindowId,
        paused: bool,
    },
    Reset {
        window_id: WindowId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId {
    id: NonZeroU32,
}

impl WindowId {
    pub fn new() -> Self {
        static IDS: AtomicU32 = AtomicU32::new(1);
        Self {
            id: NonZeroU32::new(IDS.fetch_add(1, Ordering::Relaxed)).unwrap(),
        }
    }

    pub fn id(&self) -> NonZeroU32 {
        self.id
    }
}

impl raw_window_handle::HasWindowHandle for WindowId {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'static>, raw_window_handle::HandleError> {
        let raw = raw_window_handle::RawWindowHandle::Web(raw_window_handle::WebWindowHandle::new(
            self.id.into(),
        ));
        let window_handle = unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) };
        Ok(window_handle)
    }
}

impl raw_window_handle::HasDisplayHandle for WindowId {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'static>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::web())
    }
}

impl leptos::IntoAttribute for WindowId {
    fn into_attribute(self) -> leptos::Attribute {
        leptos::Attribute::String(self.id.to_string().into())
    }

    fn into_attribute_boxed(self: Box<Self>) -> leptos::Attribute {
        self.into_attribute()
    }
}

#[derive(Clone, Debug)]
pub struct WindowHandle {
    graphics: Graphics,
    window_id: WindowId,
}

impl WindowHandle {
    pub async fn run(&self, code: String) -> Result<(), CompileError> {
        let (tx_result, rx_result) = oneshot::channel();
        self.graphics.send_command(Command::Run {
            window_id: self.window_id,
            code,
            tx_result,
        });
        rx_result.await.unwrap()
    }

    pub fn destroy_window(&self) {
        self.graphics.send_command(Command::DestroyWindow {
            window_id: self.window_id,
        });
    }

    pub fn resize(&self, surface_size: SurfaceSize) {
        self.graphics.send_command(Command::Resize {
            window_id: self.window_id,
            surface_size,
        });
    }

    pub fn set_mouse_position(&self, position: Option<[f32; 2]>) {
        self.graphics.send_command(Command::SetMousePosition {
            window_id: self.window_id,
            position,
        });
    }

    pub fn set_visibility(&self, visible: bool) {
        self.graphics.send_command(Command::SetVisibility {
            window_id: self.window_id,
            visible,
        });
    }

    pub fn set_paused(&self, paused: bool) {
        self.graphics.send_command(Command::SetPaused {
            window_id: self.window_id,
            paused,
        });
    }

    pub fn reset(&self) {
        self.graphics.send_command(Command::Reset {
            window_id: self.window_id,
        });
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

impl SurfaceSize {
    pub fn from_html_canvas(canvas: &HtmlCanvasElement) -> Self {
        Self {
            width: canvas.width().max(1),
            height: canvas.height().max(1),
        }
    }

    pub fn from_surface_configuration(surface_configuration: &wgpu::SurfaceConfiguration) -> Self {
        Self {
            width: surface_configuration.width,
            height: surface_configuration.height,
        }
    }
}

struct Window {
    backend: Backend,
    surface: wgpu::Surface<'static>,
    surface_configuration: wgpu::SurfaceConfiguration,
    pipeline: Option<Pipeline>,
    mouse_position: Option<[f32; 2]>,
    visible: bool,
    paused: bool,
    previous_frame_time: Instant,
    time: f32,
    fps: TicksPerSecond,
    on_frame: Box<dyn FnMut(FrameInfo) + 'static>,
    input_uniform: InputUniform,
}

impl Window {
    pub fn create_pipeline(&mut self, shader: naga::Module) {
        let input_buffer = self.backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("input buffer"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
            size: wgpu_buffer_size::<InputUniform>(),
        });

        let input_bind_group_layout =
            self.backend
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("input bind group layout"),
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

        let input_bind_group = self
            .backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &input_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                }],
                label: Some("input bind group"),
            });

        let shader = self
            .backend
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shader"),
                source: wgpu::ShaderSource::Naga(Cow::Owned(shader)),
            });

        let pipeline_layout =
            self.backend
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Render3dMeshesWithMaterial pipeline layout"),
                    bind_group_layouts: &[&input_bind_group_layout],
                    push_constant_ranges: &[],
                });

        let pipeline =
            self.backend
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: "vs_main",
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: "fs_main",
                        targets: &[Some(wgpu::ColorTargetState {
                            format: self.surface_configuration.format,
                            blend: Some(wgpu::BlendState::REPLACE),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: Some(wgpu::Face::Back),
                        polygon_mode: wgpu::PolygonMode::Fill,
                        unclipped_depth: false,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState {
                        count: 1,
                        mask: !0,
                        alpha_to_coverage_enabled: false,
                    },
                    multiview: None,
                    cache: None,
                });

        self.pipeline = Some(Pipeline {
            pipeline,
            input_buffer,
            input_bind_group,
        });
    }

    pub fn resize(&mut self, surface_size: SurfaceSize) {
        self.surface_configuration.width = surface_size.width;
        self.surface_configuration.height = surface_size.height;
        self.surface
            .configure(&self.backend.device, &self.surface_configuration);
        self.render();
    }

    pub fn update(&mut self) {
        // update timing information
        let now = Instant::now();
        self.fps.push(now);
        self.time += now.duration_since(self.previous_frame_time).as_secs_f32();
        self.previous_frame_time = now;

        // update input uniform
        let width = self.surface_configuration.width as f32;
        let height = self.surface_configuration.height as f32;
        self.input_uniform = InputUniform {
            time: self.time,
            aspect: width / height,
            mouse: self
                .mouse_position
                .map(|pos| [pos[0] / width * 2.0 - 1.0, pos[1] / height * 2.0 - 1.0])
                .unwrap_or_default(),
        };
    }

    pub fn render(&mut self) {
        if let Some(pipeline) = &mut self.pipeline {
            self.backend.queue.write_buffer(
                &pipeline.input_buffer,
                0,
                bytemuck::bytes_of(&self.input_uniform),
            );

            let target_texture = self
                .surface
                .get_current_texture()
                .expect("could not get target texture");

            let target_view = target_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            let mut encoder =
                self.backend
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("render encoder"),
                    });

            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render3d render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &pipeline.input_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
            drop(render_pass);

            self.backend.queue.submit([encoder.finish()]);
            target_texture.present();

            (self.on_frame)(FrameInfo {
                time: self.time,
                fps: self.fps.tps().unwrap_or_default(),
            });
        }
    }
}

#[derive(Debug)]
struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    input_buffer: wgpu::Buffer,
    input_bind_group: wgpu::BindGroup,
}

pub fn wgpu_buffer_size<T>() -> u64 {
    let unpadded_size: u64 = std::mem::size_of::<T>()
        .try_into()
        .expect("failed to convert usize to u64");
    let align_mask = wgpu::COPY_BUFFER_ALIGNMENT - 1;
    let padded_size = ((unpadded_size + align_mask) & !align_mask).max(wgpu::COPY_BUFFER_ALIGNMENT);
    padded_size
}

#[derive(Clone, Copy, Debug, Pod, Zeroable, Default)]
#[repr(C)]
pub struct InputUniform {
    pub time: f32,
    pub aspect: f32,
    pub mouse: [f32; 2],
}

fn compile_shader(source: &str) -> Result<naga::Module, CompileError> {
    let module = naga::front::wgsl::parse_str(source).map_err(|parse_error| {
        CompileError::Parse {
            parse_error,
            code: source.to_owned(),
        }
    })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let _module_info = validator.validate(&module).map_err(|validation_error| {
        CompileError::Validate {
            validation_error,
            code: source.to_owned(),
        }
    })?;
    Ok(module)
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    Parse {
        #[source]
        parse_error: naga::front::wgsl::ParseError,
        code: String,
    },
    Validate {
        #[source]
        validation_error: naga::WithSpan<naga::valid::ValidationError>,
        code: String,
    },
}

impl Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let error_string = match self {
            CompileError::Parse { parse_error, code } => parse_error.emit_to_string(code),
            CompileError::Validate {
                validation_error,
                code,
            } => validation_error.emit_to_string(&code),
        };
        write!(f, "{error_string}")
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInfo {
    pub time: f32,
    pub fps: f32,
}
