//! GPU post-processing backend (`--features gpu`): the post effects of
//! [`crate::postfx`] as WGSL compute passes on wgpu.
//!
//! [`GpuPost::apply`] is a drop-in for [`crate::postfx::apply_post`]: it uploads
//! the finished premultiplied RGBA8 pixmap once, runs every effect in list order
//! as compute passes (ping-pong between two frame buffers, a float pyramid for
//! bloom), and reads the result back once. The CPU implementation stays the
//! reference: each shader ports the same f32 formulas, the same sampling and
//! the same lowbias32 noise (identical u32 arithmetic, so glitch bands and
//! grain make the same random choices), with bilinear weights evaluated in the
//! shader rather than by texture hardware. Parity is checked by
//! `tests/gpu_parity.rs`; see `docs/POST_EFFECTS.md` for guarantees and timings.
//!
//! Frames live in storage buffers (one packed `u32` per pixel) instead of
//! textures: that makes the 8-bit round trip between effects explicit (exactly
//! the CPU's per-effect quantisation) and avoids the 256-byte row alignment of
//! texture readback. Nothing here is semantic; the renderer hands over the
//! `ResolvedPost` list the timeline produced.
//!
//! Resources (pipelines, buffers) are created lazily and reused across frames;
//! buffers are rebuilt only when the canvas size changes. No `unsafe`.

mod plan;

use std::num::NonZeroU64;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use motion_core::timeline::ResolvedPost;
use resvg::tiny_skia::Pixmap;

use plan::{level_dims, plan_posts, Buf, Pass, Pipe, Plan, PARAM_WORDS, PIPE_COUNT};

/// Bytes of one pass's uniform block (`PARAM_WORDS` 32-bit words).
const PARAM_BYTES: u64 = (PARAM_WORDS * 4) as u64;

/// Why the GPU backend could not run.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No usable adapter (no GPU, or the driver/backend is missing).
    #[error("no GPU adapter: {0}")]
    NoAdapter(String),
    /// The adapter refused to create a device.
    #[error("GPU device: {0}")]
    Device(String),
    /// The canvas does not fit the device's buffer limits.
    #[error("{0}x{1} canvas exceeds the GPU buffer limits")]
    TooLarge(u32, u32),
    /// A validation, execution or readback error.
    #[error("GPU error: {0}")]
    Gpu(String),
}

/// Wall-clock split of the last [`GpuPost::apply`] (milliseconds).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuTimings {
    /// Planning, resource setup and queuing the upload.
    pub upload_ms: f64,
    /// Submit to completion: upload transfer, all passes and the copy to the
    /// readback buffer.
    pub compute_ms: f64,
    /// Mapping the readback buffer and copying into the pixmap.
    pub readback_ms: f64,
}

/// Buffers that depend on the canvas size.
struct Resources {
    w: u32,
    h: u32,
    ping: [wgpu::Buffer; 2],
    readback: wgpu::Buffer,
    /// Bloom pyramid levels, grown on demand.
    levels: Vec<wgpu::Buffer>,
}

/// GPU post-effect compositor. Create once, call [`GpuPost::apply`] per frame.
pub struct GpuPost {
    // Kept alive for the lifetime of the device.
    _instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    max_buffer: u64,
    /// uniform, storage ro, storage rw, storage ro (aux).
    layout: wgpu::BindGroupLayout,
    /// uniform, storage rw, storage rw (glitch).
    layout_rw: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipeline_layout_rw: wgpu::PipelineLayout,
    pipelines: [Option<wgpu::ComputePipeline>; PIPE_COUNT],
    res: Option<Resources>,
    params: wgpu::Buffer,
    params_cap: u64,
    param_stride: u64,
    taps: wgpu::Buffer,
    taps_cap: u64,
    dummy: wgpu::Buffer,
    errors: Arc<Mutex<Vec<String>>>,
    timings: GpuTimings,
}

impl std::fmt::Debug for GpuPost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuPost")
            .field("adapter", &self.adapter_name)
            .finish()
    }
}

fn kernel_source(pipe: Pipe) -> &'static str {
    match pipe {
        Pipe::Chroma => include_str!("shaders/chroma.wgsl"),
        Pipe::GlitchCopy | Pipe::GlitchRows => include_str!("shaders/glitch.wgsl"),
        Pipe::Rays => include_str!("shaders/rays.wgsl"),
        Pipe::DBlur => include_str!("shaders/dblur.wgsl"),
        Pipe::Grain => include_str!("shaders/grain.wgsl"),
        Pipe::Vignette => include_str!("shaders/vignette.wgsl"),
        Pipe::BloomBright => include_str!("shaders/bloom_bright.wgsl"),
        Pipe::BloomDown => include_str!("shaders/bloom_down.wgsl"),
        Pipe::BloomMerge => include_str!("shaders/bloom_merge.wgsl"),
        Pipe::BloomFinal => include_str!("shaders/bloom_final.wgsl"),
    }
}

fn kernel_entry(pipe: Pipe) -> &'static str {
    match pipe {
        Pipe::GlitchCopy => "copy",
        Pipe::GlitchRows => "rows",
        _ => "main",
    }
}

fn uses_rw_layout(pipe: Pipe) -> bool {
    matches!(pipe, Pipe::GlitchCopy | Pipe::GlitchRows)
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(PARAM_BYTES),
        },
        count: None,
    }
}

fn gpu_err(what: &str, e: impl std::fmt::Display) -> GpuError {
    GpuError::Gpu(format!("{what}: {e}"))
}

impl GpuPost {
    /// Open the best (high-performance) headless adapter and create the device.
    /// Errors when there is no adapter. Pipelines compile lazily on first use.
    pub fn new() -> Result<GpuPost, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        let adapter_name = adapter.get_info().name;
        let supported = adapter.limits();
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
            max_buffer_size: supported.max_buffer_size,
            ..wgpu::Limits::default()
        };
        let max_buffer = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("motion-post"),
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;

        // Errors never panic: they are collected and surfaced by `apply`.
        let errors: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&errors);
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut v) = sink.lock() {
                v.push(e.to_string());
            }
        }));

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout"),
            entries: &[
                uniform_entry(),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, true),
            ],
        });
        let layout_rw = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout rw"),
            entries: &[
                uniform_entry(),
                storage_entry(1, false),
                storage_entry(2, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline_layout_rw = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post pipeline layout rw"),
            bind_group_layouts: &[Some(&layout_rw)],
            immediate_size: 0,
        });

        let param_stride =
            u64::from(device.limits().min_uniform_buffer_offset_alignment).max(PARAM_BYTES);
        let params = Self::make_params(&device, param_stride * 16);
        let taps = Self::make_storage(&device, "dblur taps", 1024 * 12);
        let dummy = Self::make_storage(&device, "unused binding", 16);

        Ok(GpuPost {
            _instance: instance,
            device,
            queue,
            adapter_name,
            max_buffer,
            layout,
            layout_rw,
            pipeline_layout,
            pipeline_layout_rw,
            pipelines: Default::default(),
            res: None,
            params,
            params_cap: param_stride * 16,
            param_stride,
            taps,
            taps_cap: 1024 * 12,
            dummy,
            errors,
            timings: GpuTimings::default(),
        })
    }

    /// Name of the adapter in use (e.g. "Apple M4").
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Timing split of the most recent [`apply`](Self::apply).
    pub fn last_timings(&self) -> GpuTimings {
        self.timings
    }

    fn make_params(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post params"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Apply `posts` to `pixmap` in order (the GPU twin of
    /// [`crate::postfx::apply_post`]). Effects with strength 0 (or invalid
    /// parameters) are skipped; when nothing remains the pixmap is not touched.
    /// On error the pixmap is left unchanged.
    pub fn apply(
        &mut self,
        pixmap: &mut Pixmap,
        posts: &[ResolvedPost<'_>],
    ) -> Result<(), GpuError> {
        let t0 = Instant::now();
        let plan = plan_posts(pixmap.width(), pixmap.height(), posts);
        if plan.passes.is_empty() {
            self.timings = GpuTimings::default();
            return Ok(());
        }
        self.run(pixmap, &plan, t0)
    }

    /// Upload the pixmap and read it back with no effect (transfer cost only;
    /// used by the timing probe).
    pub fn roundtrip(&mut self, pixmap: &mut Pixmap) -> Result<(), GpuError> {
        let t0 = Instant::now();
        self.run(pixmap, &Plan::default(), t0)
    }

    fn ensure_resources(&mut self, w: u32, h: u32, levels: usize) -> Result<(), GpuError> {
        let bytes = u64::from(w) * u64::from(h) * 4;
        if bytes == 0 || bytes > self.max_buffer {
            return Err(GpuError::TooLarge(w, h));
        }
        let rebuild = self.res.as_ref().is_none_or(|r| r.w != w || r.h != h);
        if rebuild {
            let mk = |label: &str| Self::make_storage(&self.device, label, bytes);
            self.res = Some(Resources {
                w,
                h,
                ping: [mk("frame a"), mk("frame b")],
                readback: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                levels: Vec::new(),
            });
        }
        if levels > 0 {
            let dims = level_dims(w, h, levels);
            if let Some(res) = self.res.as_mut() {
                for (k, &(lw, lh)) in dims.iter().enumerate().skip(res.levels.len()) {
                    // vec4<f32> per texel; the pyramid is float for headroom.
                    let size = u64::from(lw) * u64::from(lh) * 16;
                    if size > self.max_buffer {
                        return Err(GpuError::TooLarge(w, h));
                    }
                    res.levels.push(Self::make_storage(
                        &self.device,
                        &format!("bloom level {k}"),
                        size,
                    ));
                }
            }
        }
        Ok(())
    }

    fn ensure_pipeline(&mut self, pipe: Pipe) {
        if self.pipelines[pipe as usize].is_some() {
            return;
        }
        let source = format!(
            "{}\n{}",
            include_str!("shaders/prelude.wgsl"),
            kernel_source(pipe)
        );
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("post kernel"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let layout = if uses_rw_layout(pipe) {
            &self.pipeline_layout_rw
        } else {
            &self.pipeline_layout
        };
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("post pipeline"),
                layout: Some(layout),
                module: &module,
                entry_point: Some(kernel_entry(pipe)),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        self.pipelines[pipe as usize] = Some(pipeline);
    }

    fn buffer<'a>(&'a self, res: &'a Resources, b: Buf) -> Result<&'a wgpu::Buffer, GpuError> {
        match b {
            Buf::Ping(i) => Ok(&res.ping[i]),
            Buf::Level(k) => res
                .levels
                .get(k)
                .ok_or_else(|| GpuError::Gpu(format!("missing bloom level {k}"))),
            Buf::Taps => Ok(&self.taps),
            Buf::Dummy => Ok(&self.dummy),
        }
    }

    fn take_errors(&self) -> Option<String> {
        let mut v = self.errors.lock().ok()?;
        if v.is_empty() {
            return None;
        }
        let msg = v.join("; ");
        v.clear();
        Some(msg)
    }

    /// Execute `plan` on `pixmap`. After any error the canvas buffers are
    /// dropped (rebuilt on the next call) so no half-finished mapping survives.
    fn run(&mut self, pixmap: &mut Pixmap, plan: &Plan, t0: Instant) -> Result<(), GpuError> {
        let result = self.run_inner(pixmap, plan, t0);
        if result.is_err() {
            self.res = None;
        }
        result
    }

    fn run_inner(&mut self, pixmap: &mut Pixmap, plan: &Plan, t0: Instant) -> Result<(), GpuError> {
        let (w, h) = (pixmap.width(), pixmap.height());
        // Drop any stale error so this frame's report is its own.
        let _ = self.take_errors();
        self.ensure_resources(w, h, plan.levels)?;
        for pass in &plan.passes {
            self.ensure_pipeline(pass.pipe);
        }

        // Uniform blocks: one 64-byte block per pass at `param_stride`.
        if !plan.passes.is_empty() {
            let need = self.param_stride * plan.passes.len() as u64;
            if need > self.params_cap {
                let cap = need.next_power_of_two();
                self.params = Self::make_params(&self.device, cap);
                self.params_cap = cap;
            }
            let mut bytes = vec![0u8; need as usize];
            for (i, pass) in plan.passes.iter().enumerate() {
                let at = i * self.param_stride as usize;
                for (k, word) in pass.params.iter().enumerate() {
                    bytes[at + k * 4..at + k * 4 + 4].copy_from_slice(&word.to_le_bytes());
                }
            }
            self.queue.write_buffer(&self.params, 0, &bytes);
        }
        if !plan.taps.is_empty() {
            let need = (plan.taps.len() * std::mem::size_of::<plan::GpuTap>()) as u64;
            if need > self.taps_cap {
                let cap = need.next_power_of_two();
                self.taps = Self::make_storage(&self.device, "dblur taps", cap);
                self.taps_cap = cap;
            }
            self.queue
                .write_buffer(&self.taps, 0, bytemuck::cast_slice(&plan.taps));
        }

        let res = self
            .res
            .as_ref()
            .ok_or_else(|| GpuError::Gpu("resources missing".into()))?;
        self.queue.write_buffer(&res.ping[0], 0, pixmap.data());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post"),
            });
        if !plan.passes.is_empty() {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("post effects"),
                timestamp_writes: None,
            });
            for (i, pass) in plan.passes.iter().enumerate() {
                let bg = self.bind_group(res, pass)?;
                let pipeline = self.pipelines[pass.pipe as usize]
                    .as_ref()
                    .ok_or_else(|| GpuError::Gpu("pipeline missing".into()))?;
                cpass.set_pipeline(pipeline);
                cpass.set_bind_group(0, &bg, &[(i as u64 * self.param_stride) as u32]);
                cpass.dispatch_workgroups(pass.groups[0], pass.groups[1], 1);
            }
        }
        let out = &res.ping[plan.result];
        encoder.copy_buffer_to_buffer(out, 0, &res.readback, 0, u64::from(w) * u64::from(h) * 4);
        let upload_done = Instant::now();
        self.timings.upload_ms = (upload_done - t0).as_secs_f64() * 1000.0;

        self.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        res.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| gpu_err("poll", e))?;
        let compute_done = Instant::now();
        self.timings.compute_ms = (compute_done - upload_done).as_secs_f64() * 1000.0;

        // The map callback has run by now (the poll waited): `Ok(Ok(()))` means
        // the readback buffer is mapped and must be unmapped on every path.
        let mapped = matches!(rx.try_recv(), Ok(Ok(())));
        if let Some(msg) = self.take_errors() {
            if mapped {
                res.readback.unmap();
            }
            return Err(GpuError::Gpu(msg));
        }
        if !mapped {
            return Err(GpuError::Gpu("readback buffer was not mapped".into()));
        }
        let copied = {
            let view = res
                .readback
                .slice(..)
                .get_mapped_range()
                .map_err(|e| gpu_err("mapped range", e));
            match view {
                Ok(view) if view.len() == pixmap.data().len() => {
                    pixmap.data_mut().copy_from_slice(&view);
                    Ok(())
                }
                Ok(_) => Err(GpuError::Gpu("readback size mismatch".into())),
                Err(e) => Err(e),
            }
        };
        res.readback.unmap();
        copied?;
        self.timings.readback_ms = compute_done.elapsed().as_secs_f64() * 1000.0;
        Ok(())
    }

    fn bind_group(&self, res: &Resources, pass: &Pass) -> Result<wgpu::BindGroup, GpuError> {
        let params = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.params,
            offset: 0,
            size: NonZeroU64::new(PARAM_BYTES),
        });
        let src = self.buffer(res, pass.src)?.as_entire_binding();
        let dst = self.buffer(res, pass.dst)?.as_entire_binding();
        if uses_rw_layout(pass.pipe) {
            Ok(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post bind group rw"),
                layout: &self.layout_rw,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params,
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src,
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: dst,
                    },
                ],
            }))
        } else {
            let aux = self.buffer(res, pass.aux)?.as_entire_binding();
            Ok(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params,
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src,
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: dst,
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: aux,
                    },
                ],
            }))
        }
    }
}
