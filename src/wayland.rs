//! Wayland event loop, layer-shell surface management, and the per-frame tick.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use calloop::EventLoop;
use calloop::timer::{TimeoutAction, Timer};
use calloop_wayland_source::WaylandSource;
use glyphon::{Color as GlyphonColor, TextArea, TextBounds};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_seat,
    delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler},
};
use tracing::{debug, error, info, warn};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalList, registry_queue_init},
    protocol::{wl_keyboard, wl_output, wl_seat, wl_surface},
};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
};

use crate::audio::Audio;
use crate::color::hsl_to_rgb;
use crate::config::Config;
use crate::effect::{Effect, spawn_arrow, spawn_firework, spawn_letter, spawn_misc, spawn_rainbow};
use crate::gpu::{Gpu, RenderParams};
use crate::keys::{ArrowDir, EXIT_KEYSYM, ExitGate, KeyAction, PENDING_QUEUE_CAP, classify};
use crate::particle::Particle;
use crate::render::{DrawBatch, FrameInputs, FrameText, build_frame};
use crate::text::TextSystem;

/// Frame interval target. 60 Hz is plenty for this app and avoids burning the
/// laptop battery; the surface is set to FIFO/Mailbox so the compositor will
/// pace us regardless.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Maximum simultaneously-living visual effects and particles. The simulation
/// tick aggressively trims to these bounds so even an extended palm-mash
/// session doesn't trash performance.
const MAX_EFFECTS: usize = 60;
const MAX_PARTICLES: usize = 800;
/// Spawn at most this many effects per frame regardless of how many keypresses
/// arrived. Smooths out auto-repeat or simultaneous chord presses.
const MAX_SPAWNS_PER_FRAME: usize = 4;

/// Splash screen duration in seconds.
const SPLASH_TIME: f32 = 2.0;

/// One layer-surface per `wl_output`.
///
/// **Field order is load-bearing.** Rust drops fields in declaration order,
/// and `wgpu::Surface` calls `vkDestroySurfaceKHR` (which dereferences the
/// `wl_surface` pointer) on drop. The `wgpu_surface` *must* therefore drop
/// before the `LayerSurface` that owns the underlying `wl_surface`. The
/// custom [`Drop`] impl ensures the inhibitor proxy gets a clean
/// `destroy` request first.
struct OutputSurface {
    inhibitor: Option<ZwpKeyboardShortcutsInhibitorV1>,
    wgpu_surface: wgpu::Surface<'static>,
    layer: LayerSurface,
    output: wl_output::WlOutput,
    width: u32,
    height: u32,
    configured: bool,
}

impl Drop for OutputSurface {
    fn drop(&mut self) {
        if let Some(i) = self.inhibitor.take() {
            i.destroy();
        }
    }
}

/// Forces surface teardown before the Wayland connection closes. wgpu's
/// `Surface` holds a raw pointer to the `wl_surface`, so we must guarantee
/// every `OutputSurface` is dropped while the connection is still live —
/// even on a `?` early-return from [`App::run`].
impl Drop for App {
    fn drop(&mut self) {
        self.surfaces.clear();
    }
}

pub struct App {
    config: Config,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,

    conn: Connection,
    qh: QueueHandle<Self>,
    gpu: Gpu,
    text_system: TextSystem,
    batch: DrawBatch,
    /// Reused across frames; never freed. Cleared each tick before
    /// [`build_frame`] re-fills it.
    frame_texts: Vec<FrameText>,
    /// Pool of glyphon text buffers, sized on demand and reused. Avoids
    /// allocating + shaping a fresh buffer per glyph per surface per frame.
    frame_buffers: Vec<glyphon::Buffer>,
    audio: Option<Audio>,

    surfaces: Vec<OutputSurface>,
    /// First seat we see — there is virtually never more than one in practice
    /// and the inhibit protocol is per-seat anyway.
    seat: Option<wl_seat::WlSeat>,
    inhibit_manager: Option<ZwpKeyboardShortcutsInhibitManagerV1>,

    effects: Vec<Effect>,
    particles: Vec<Particle>,
    rng: SmallRng,

    start_instant: Instant,
    last_tick: Instant,
    splash_time: f32,

    exit_gate: ExitGate,
    pending_keys: Vec<KeyAction>,
    should_exit: bool,
}

impl App {
    pub fn run(config: Config) -> Result<()> {
        info!(
            version = env!("CARGO_PKG_VERSION"),
            mute = config.mute,
            no_flash = config.no_flash,
            volume = config.volume,
            "starting bimbumbam"
        );
        let conn = Connection::connect_to_env()
            .context("failed to connect to Wayland — is WAYLAND_DISPLAY set?")?;
        let (globals, event_queue) =
            registry_queue_init::<App>(&conn).context("failed to initialize Wayland registry")?;
        let qh = event_queue.handle();

        let compositor_state = CompositorState::bind(&globals, &qh)
            .context("compositor does not implement wl_compositor")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context(
            "compositor does not implement wlr-layer-shell — bimbumbam supports Sway, Hyprland, \
             niri, KDE Plasma 6, river, and Wayfire (GNOME's mutter is unsupported)",
        )?;
        let shm = Shm::bind(&globals, &qh).context("compositor does not implement wl_shm")?;
        let seat_state = SeatState::new(&globals, &qh);
        let output_state = OutputState::new(&globals, &qh);

        let inhibit_manager = bind_inhibit_manager(&globals, &qh);
        if inhibit_manager.is_none() {
            warn!(
                "compositor does not implement zwp_keyboard_shortcuts_inhibit_manager_v1; \
                 some compositor key bindings (e.g. workspace switches) may still fire"
            );
        } else {
            info!("keyboard-shortcuts-inhibit manager acquired");
        }

        let gpu = Gpu::new()?;
        let text_system = TextSystem::new(&gpu.device, &gpu.queue, crate::gpu::SURFACE_FORMAT);

        let audio = if config.mute {
            None
        } else {
            Audio::try_new(config.volume)
        };
        if config.mute {
            info!("audio disabled by --mute");
        } else if audio.is_none() {
            warn!("no audio output device available — running silently");
        } else {
            info!(volume = config.volume, "audio engine ready");
        }

        let now = Instant::now();
        let mut app = App {
            config,
            registry_state: RegistryState::new(&globals),
            seat_state,
            output_state,
            compositor_state,
            layer_shell,
            shm,
            conn: conn.clone(),
            qh: qh.clone(),
            gpu,
            text_system,
            batch: DrawBatch::new(),
            frame_texts: Vec::new(),
            frame_buffers: Vec::new(),
            audio,
            surfaces: Vec::new(),
            seat: None,
            inhibit_manager,
            effects: Vec::new(),
            particles: Vec::new(),
            rng: SmallRng::from_os_rng(),
            start_instant: now,
            last_tick: now,
            splash_time: SPLASH_TIME,
            exit_gate: ExitGate::default(),
            pending_keys: Vec::new(),
            should_exit: false,
        };

        let mut event_loop: EventLoop<'_, App> =
            EventLoop::try_new().context("failed to create calloop event loop")?;
        WaylandSource::new(conn, event_queue)
            .insert(event_loop.handle())
            .map_err(|e| anyhow!("failed to insert Wayland source: {e}"))?;
        let timer = Timer::from_duration(FRAME_INTERVAL);
        event_loop
            .handle()
            .insert_source(timer, |_, _, app| {
                app.tick();
                TimeoutAction::ToDuration(FRAME_INTERVAL)
            })
            .map_err(|e| anyhow!("failed to insert frame timer: {e}"))?;

        info!("entering main loop");
        while !app.should_exit {
            event_loop
                .dispatch(FRAME_INTERVAL, &mut app)
                .context("Wayland event loop failed")?;
        }
        info!("exit requested, tearing down");

        // Drop wgpu surfaces and inhibitors before the wayland connection is dropped.
        app.surfaces.clear();
        Ok(())
    }

    fn canvas_size(&self) -> (f32, f32) {
        // Use the largest configured display as the canonical canvas; we
        // letterbox-fit it onto each surface to keep visuals consistent.
        self.surfaces
            .iter()
            .filter(|s| s.configured && s.width > 0 && s.height > 0)
            .map(|s| (s.width as f32, s.height as f32))
            .max_by(|a, b| (a.0 * a.1).total_cmp(&(b.0 * b.1)))
            .unwrap_or((1920.0, 1080.0))
    }

    fn try_inhibit(&self, surface: &mut OutputSurface) {
        if surface.inhibitor.is_some() {
            return;
        }
        let (Some(mgr), Some(seat)) = (&self.inhibit_manager, &self.seat) else {
            return;
        };
        // The protocol forbids two concurrent inhibitors on the same (surface,seat).
        // Since we only call this when `inhibitor.is_none()`, that's fine.
        let inh = mgr.inhibit_shortcuts(surface.layer.wl_surface(), seat, &self.qh, ());
        info!("attached keyboard-shortcuts-inhibitor to layer surface");
        surface.inhibitor = Some(inh);
    }

    fn try_inhibit_all(&mut self) {
        // Borrow gymnastics: take the surfaces out, mutate, put back.
        let mut surfaces = std::mem::take(&mut self.surfaces);
        for s in &mut surfaces {
            if s.configured {
                self.try_inhibit(s);
            }
        }
        self.surfaces = surfaces;
    }

    fn enqueue_action(&mut self, action: KeyAction) {
        if self.pending_keys.len() >= PENDING_QUEUE_CAP {
            return;
        }
        self.pending_keys.push(action);
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;
        let time = now.duration_since(self.start_instant).as_secs_f32();
        self.splash_time -= dt;

        let (sw, sh) = self.canvas_size();

        if self.splash_time <= 0.0 {
            let mut spawns = 0;
            let mut i = 0;
            while i < self.pending_keys.len() && spawns < MAX_SPAWNS_PER_FRAME {
                let action = self.pending_keys[i];
                self.dispatch_action(action, sw, sh);
                spawns += 1;
                i += 1;
            }
            self.pending_keys.drain(0..i);
        } else {
            self.pending_keys.clear();
        }

        for e in &mut self.effects {
            e.update(dt, sw, sh);
        }
        self.effects.retain(|e| e.is_alive());
        for p in &mut self.particles {
            p.update(dt);
        }
        self.particles.retain(|p| p.is_alive());

        if self.effects.len() > MAX_EFFECTS {
            let drop_count = self.effects.len() - MAX_EFFECTS;
            self.effects.drain(0..drop_count);
        }
        if self.particles.len() > MAX_PARTICLES {
            let drop_count = self.particles.len() - MAX_PARTICLES;
            self.particles.drain(0..drop_count);
        }

        let (should_exit, exit_progress) = self.exit_gate.poll(now);
        if should_exit {
            info!("exit chord completed");
            self.should_exit = true;
        }

        // Subtle, slowly-cycling background hue. Saturation/lightness kept low
        // so the foreground reads cleanly.
        let hue = (time * 0.03) % 1.0;
        let (br, bg, bb) = hsl_to_rgb(hue, 0.3, 0.08);

        self.batch.clear();
        self.frame_texts.clear();
        build_frame(
            &mut self.batch,
            &mut self.frame_texts,
            FrameInputs {
                effects: &self.effects,
                particles: &self.particles,
                exit_progress,
                splash_time: self.splash_time,
                canvas_w: sw,
                canvas_h: sh,
            },
        );

        self.render_all_surfaces(sw, sh, [f64::from(br), f64::from(bg), f64::from(bb)]);
    }

    fn dispatch_action(&mut self, action: KeyAction, sw: f32, sh: f32) {
        match action {
            KeyAction::Letter(ch) => {
                spawn_letter(
                    &mut self.rng,
                    ch,
                    sw,
                    sh,
                    &mut self.effects,
                    &mut self.particles,
                );
                if let Some(a) = &mut self.audio {
                    a.play_note(crate::audio::pitch_index_for_char(ch));
                }
            }
            KeyAction::Space => {
                let flash = !self.config.no_flash;
                spawn_firework(
                    &mut self.rng,
                    sw,
                    sh,
                    flash,
                    &mut self.effects,
                    &mut self.particles,
                );
                if let Some(a) = &mut self.audio {
                    a.play_note(15);
                }
            }
            KeyAction::Enter => {
                let flash = !self.config.no_flash;
                spawn_rainbow(
                    &mut self.rng,
                    sw,
                    sh,
                    flash,
                    &mut self.effects,
                    &mut self.particles,
                );
                if let Some(a) = &mut self.audio {
                    a.play_chime();
                }
            }
            KeyAction::Arrow(dir) => {
                let (dx, dy) = dir.vector();
                spawn_arrow(
                    &mut self.rng,
                    dx,
                    dy,
                    sw,
                    sh,
                    &mut self.effects,
                    &mut self.particles,
                );
                if let Some(a) = &mut self.audio {
                    a.play_note(match dir {
                        ArrowDir::Up => 12,
                        ArrowDir::Down => 5,
                        ArrowDir::Left => 7,
                        ArrowDir::Right => 9,
                    });
                }
            }
            KeyAction::Misc(seed) => {
                spawn_misc(
                    &mut self.rng,
                    sw,
                    sh,
                    &mut self.effects,
                    &mut self.particles,
                );
                if let Some(a) = &mut self.audio {
                    a.play_note(seed as usize);
                }
            }
        }
    }

    fn render_all_surfaces(&mut self, sw: f32, sh: f32, clear: [f64; 3]) {
        // Snapshot indices that need a reconfigure pass after rendering — we
        // can't call configure_surface inside the for-loop because it would
        // require a mutable borrow of self.surfaces while we're iterating it.
        let mut needs_reconfigure: Vec<usize> = Vec::new();

        for (idx, surface) in self.surfaces.iter().enumerate() {
            if !surface.configured || surface.width == 0 || surface.height == 0 {
                continue;
            }

            let surf_w = surface.width as f32;
            let surf_h = surface.height as f32;
            let scale = (surf_w / sw).min(surf_h / sh);
            let offset_x = (surf_w - sw * scale) / 2.0;
            let offset_y = (surf_h - sh * scale) / 2.0;

            // Pool: ensure we have enough TextBuffers to back this frame's
            // texts. New ones are appended; old ones are reused with
            // `set_text` / `set_size` (no realloc, just a re-shape).
            while self.frame_buffers.len() < self.frame_texts.len() {
                let buf = self.text_system.make_buffer("", 1.0, None);
                self.frame_buffers.push(buf);
            }
            for (buf, ft) in self
                .frame_buffers
                .iter_mut()
                .zip(self.frame_texts.iter())
                .take(self.frame_texts.len())
            {
                let metrics =
                    glyphon::Metrics::new(ft.font_size * scale, ft.font_size * scale * 1.2);
                buf.set_metrics(&mut self.text_system.font_system, metrics);
                let attrs = glyphon::Attrs::new()
                    .family(glyphon::Family::SansSerif)
                    .weight(glyphon::Weight::BOLD);
                buf.set_text(
                    &mut self.text_system.font_system,
                    &ft.text,
                    &attrs,
                    glyphon::Shaping::Advanced,
                    None,
                );
                buf.shape_until_scroll(&mut self.text_system.font_system, false);
            }

            let text_areas: Vec<TextArea<'_>> = self
                .frame_texts
                .iter()
                .zip(self.frame_buffers.iter())
                .map(|(ft, buf)| TextArea {
                    buffer: buf,
                    left: ft.x * scale + offset_x,
                    top: ft.y * scale + offset_y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: surface.width as i32,
                        bottom: surface.height as i32,
                    },
                    default_color: GlyphonColor::rgba(
                        (ft.color.r * 255.0) as u8,
                        (ft.color.g * 255.0) as u8,
                        (ft.color.b * 255.0) as u8,
                        (ft.color.a * 255.0) as u8,
                    ),
                    custom_glyphs: &[],
                })
                .collect();

            let ok = self.gpu.render(RenderParams {
                surface: &surface.wgpu_surface,
                batch: &self.batch,
                text_sys: &mut self.text_system,
                text_areas: &text_areas,
                width: surface.width,
                height: surface.height,
                scale,
                offset: (offset_x, offset_y),
                clear_color: clear,
            });
            if !ok {
                needs_reconfigure.push(idx);
            }
        }

        for idx in needs_reconfigure {
            if let Some(s) = self.surfaces.get(idx) {
                debug!(idx, "reconfiguring surface after Lost/Outdated");
                if let Err(e) = self
                    .gpu
                    .configure_surface(&s.wgpu_surface, s.width, s.height)
                {
                    warn!(error = %e, idx, "surface reconfigure failed");
                }
            }
        }
    }
}

fn bind_inhibit_manager(
    globals: &GlobalList,
    qh: &QueueHandle<App>,
) -> Option<ZwpKeyboardShortcutsInhibitManagerV1> {
    // The protocol XML in wayland-protocols 0.32 only declares v1; binding
    // at a higher version we don't actually understand would deliver events
    // for which we have no Dispatch generated.
    globals
        .bind::<ZwpKeyboardShortcutsInhibitManagerV1, _, _>(qh, 1..=1, ())
        .ok()
}

// === SCTK handler implementations ===

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let wl_surface = self.compositor_state.create_surface(qh);
        let wgpu_surface = match self.gpu.create_surface(&self.conn, &wl_surface) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "failed to create wgpu surface for new output; skipping");
                return;
            }
        };
        let layer = self.layer_shell.create_layer_surface(
            qh,
            wl_surface,
            Layer::Overlay,
            Some("bimbumbam"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        layer.commit();
        info!(
            "new output added; total surfaces = {}",
            self.surfaces.len() + 1
        );
        self.surfaces.push(OutputSurface {
            inhibitor: None,
            wgpu_surface,
            layer,
            output,
            width: 0,
            height: 0,
            configured: false,
        });
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Drop the matching OutputSurface. Drop ordering destroys the inhibitor
        // first (see [`OutputSurface::drop`]) which is what the protocol expects.
        let before = self.surfaces.len();
        self.surfaces.retain(|s| s.output != output);
        info!(
            "output removed; surfaces {} -> {}",
            before,
            self.surfaces.len()
        );
        if self.surfaces.is_empty() {
            // Last screen unplugged — there's nothing left to render onto and
            // no way to recapture keyboard focus. Bail out cleanly.
            warn!("last output gone, exiting");
            self.should_exit = true;
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // A toddler-triggered output unplug can close one of our surfaces. Drop
        // it but only quit if every surface is gone — never auto-exit on a
        // single closure, that would be an escape route.
        self.surfaces
            .retain(|s| s.layer.wl_surface() != layer.wl_surface());
        if self.surfaces.is_empty() {
            self.should_exit = true;
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let mut needs_inhibit = false;
        for surface in &mut self.surfaces {
            if surface.layer.wl_surface() == layer.wl_surface() {
                surface.width = configure.new_size.0.max(1);
                surface.height = configure.new_size.1.max(1);
                if let Err(e) =
                    self.gpu
                        .configure_surface(&surface.wgpu_surface, surface.width, surface.height)
                {
                    warn!(error = %e, "failed to configure GPU surface");
                    continue;
                }
                debug!(
                    width = surface.width,
                    height = surface.height,
                    "surface configured"
                );
                surface.configured = true;
                needs_inhibit = true;
                break;
            }
        }
        if needs_inhibit {
            self.try_inhibit_all();
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.is_none() {
            info!("seat acquired");
            self.seat = Some(seat);
            self.try_inhibit_all();
        }
    }
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard {
            if let Err(e) = self.seat_state.get_keyboard(qh, &seat, None) {
                error!(error = %e, "failed to acquire keyboard from seat");
            }
            // SCTK only calls `new_seat` for seats that appear *at runtime* — the
            // seat that already existed when we bound the registry comes through
            // here instead. Capture it so the inhibit attempt below has a seat
            // to bind against.
            if self.seat.is_none() {
                info!("seat acquired (via keyboard capability)");
                self.seat = Some(seat);
                self.try_inhibit_all();
            }
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard {
            // Without focus events we won't get a `leave` to clear the chord
            // state, so reset eagerly here.
            self.exit_gate.reset();
            self.pending_keys.clear();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.as_ref() == Some(&seat) {
            self.seat = None;
        }
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        // Reset on focus enter — the compositor delivers a fresh modifiers event
        // immediately after, but starting from a clean slate avoids a stuck
        // modifier across focus loss.
        self.exit_gate.reset();
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.exit_gate.reset();
        self.pending_keys.clear();
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let sym = event.keysym.raw();
        if sym == EXIT_KEYSYM {
            self.exit_gate.press_exit_key(Instant::now());
            // Don't spawn an effect for the exit keypress when the chord is in
            // progress — the parent doesn't want a wall of "Q" letters.
            if self.exit_gate.ctrl && self.exit_gate.alt {
                debug!("exit chord engaged; suppressing Q effect");
                return;
            }
        }
        let action = classify(sym);
        debug!(?action, keysym = sym, "key pressed");
        self.enqueue_action(action);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym.raw() == EXIT_KEYSYM {
            self.exit_gate.release_exit_key();
        }
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: u32,
    ) {
        self.exit_gate.set_modifiers(modifiers.ctrl, modifiers.alt);
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers!(OutputState, SeatState);
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_seat!(App);
smithay_client_toolkit::delegate_keyboard!(App);
delegate_shm!(App);
delegate_registry!(App);

// === Manual Dispatch for the keyboard-shortcuts-inhibit protocol ===

impl Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitManagerV1,
        _: <ZwpKeyboardShortcutsInhibitManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // No events on the manager.
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitorV1,
        event: <ZwpKeyboardShortcutsInhibitorV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // We don't change behavior in response to active/inactive — the
        // compositor decides — but we surface them so a user can verify
        // whether the inhibitor is doing anything when running with
        // `RUST_LOG=bimbumbam=info` or higher.
        use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1::Event;
        match event {
            Event::Active => {
                info!("inhibitor ACTIVE — compositor shortcuts suppressed for our surface");
            }
            Event::Inactive => {
                warn!(
                    "inhibitor INACTIVE — compositor will receive its own shortcuts. \
                     Either focus is elsewhere or the compositor declines to inhibit \
                     (e.g. niri binds with allow-inhibiting=false)"
                );
            }
            _ => {}
        }
    }
}
