use anyhow::{Context, Result};
use rustzx_core::Emulator;
use rustzx_core::host::{Snapshot, BufferCursor};
use rustzx_core::RustzxSettings;
use rustzx_core::zx::machine::ZXMachine;
use rustzx_core::zx::keys::ZXKey;
use rustzx_core::zx::joy::kempston::KempstonKey;
use rustzx_core::zx::joy::sinclair::{SinclairKey, SinclairJoyNum};
use rustzx_core::poke::{Poke, PokeAction};
use rustzx_core::EmulationMode;
use serde::{Serialize, Deserialize};
use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use flate2::read::ZlibDecoder;
use std::mem;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{WindowEvent, ElementState};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId, Fullscreen};
use winit::keyboard::{KeyCode, PhysicalKey, ModifiersState};
use winit::dpi::LogicalSize;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapRb, HeapProducer};
use glow::HasContext;
use glutin::prelude::*;
use glutin::display::GetGlDisplay;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::surface::{Surface as GlutinSurface, WindowSurface, SurfaceAttributesBuilder};
use winit::raw_window_handle::HasWindowHandle;

mod host;
use host::AppHost;
mod z80_loader;
mod szx_loader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderMode {
    Full,
    Minimal,
    None,
}

impl BorderMode {
    fn next(self) -> Self {
        match self {
            Self::Full => Self::Minimal,
            Self::Minimal => Self::None,
            Self::None => Self::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilteringMode {
    Nearest,
    Linear,
    Scanlines,
    Embedded,
    Custom,
}

impl FilteringMode {
    fn next(self, has_embedded: bool, has_custom: bool) -> Self {
        match self {
            Self::Nearest => Self::Linear,
            Self::Linear => Self::Scanlines,
            Self::Scanlines => {
                if has_embedded { Self::Embedded }
                else if has_custom { Self::Custom }
                else { Self::Nearest }
            }
            Self::Embedded => {
                if has_custom { Self::Custom }
                else { Self::Nearest }
            }
            Self::Custom => Self::Nearest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoystickMode {
    Off,
    Kempston,
    Sinclair1, // 6-0
    Sinclair2, // 1-5
    Cursor,    // 5-8
}

impl JoystickMode {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::Kempston,
            Self::Kempston => Self::Sinclair1,
            Self::Sinclair1 => Self::Sinclair2,
            Self::Sinclair2 => Self::Cursor,
            Self::Cursor => Self::Off,
        }
    }
}

const FOOTER_MAGIC: &[u8; 4] = b"ZXND";

#[derive(Debug, Clone)]
struct PokeEntry {
    addr: u16,
    value: u8,
    original: u8,
}

fn parse_pokes_content(content: &str) -> Vec<PokeEntry> {
    let mut pokes = Vec::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 && (parts[0] == "M" || parts[0] == "Z")
            && let (Ok(addr), Ok(val), Ok(org)) = (
                parts[2].parse::<u16>(),
                parts[3].parse::<u8>(),
                parts[4].parse::<u8>()
            ) 
        {
            pokes.push(PokeEntry { addr, value: val, original: org });
        }
    }
    pokes
}

fn load_pokes() -> Vec<PokeEntry> {
    if let Ok(exe_path) = env::current_exe() {
        let mut pok_path = exe_path.clone();
        pok_path.set_extension("pok");
        
        if let Ok(content) = std::fs::read_to_string(&pok_path) {
            return parse_pokes_content(&content);
        }
    }
    Vec::new()
}

fn load_retro_shader() -> Option<String> {
    if let Ok(exe_path) = env::current_exe() {
        let mut glsl_path = exe_path.clone();
        glsl_path.set_extension("glsl");
        if let Ok(content) = std::fs::read_to_string(&glsl_path) {
            return Some(content);
        }

        // Fallback: try shader.glsl in the same folder
        let mut fallback_path = exe_path.clone();
        fallback_path.set_file_name("shader.glsl");
        if let Ok(content) = std::fs::read_to_string(&fallback_path) {
            return Some(content);
        }
    }

    // Secondary fallback: check current working directory
    if let Ok(cwd) = std::env::current_dir() {
        // try shader.glsl in CWD
        let mut p = cwd.clone();
        p.push("shader.glsl");
        if let Ok(content) = std::fs::read_to_string(&p) {
            return Some(content);
        }

        // try <exe_name>.glsl in CWD
        if let Ok(exe_path) = env::current_exe()
            && let Some(exe_name) = exe_path.file_name() {
                let mut p = cwd.clone();
                p.push(exe_name);
                p.set_extension("glsl");
                if let Ok(content) = std::fs::read_to_string(&p) {
                    return Some(content);
                }
        }
    }
    None
}

struct ManualPoke {
    actions: Vec<PokeAction>,
}

impl Poke for ManualPoke {
    fn actions(&self) -> &[PokeAction] {
        &self.actions
    }
}

// Minimal 4x6 OSD Font (subset: A-Z, 0-9, space, punctuation)
const FONT_WIDTH: usize = 4;
const FONT_HEIGHT: usize = 6;
const FONT_DATA: &[u8] = &[
    0x6, 0x9, 0xF, 0x9, 0x9, 0x0, // A
    0xE, 0x9, 0xE, 0x9, 0xE, 0x0, // B
    0x7, 0x8, 0x8, 0x8, 0x7, 0x0, // C
    0xE, 0x9, 0x9, 0x9, 0xE, 0x0, // D
    0xF, 0x8, 0xE, 0x8, 0xF, 0x0, // E
    0xF, 0x8, 0xE, 0x8, 0x8, 0x0, // F
    0x7, 0x8, 0xB, 0x9, 0x7, 0x0, // G
    0x9, 0x9, 0xF, 0x9, 0x9, 0x0, // H
    0xE, 0x4, 0x4, 0x4, 0xE, 0x0, // I
    0x3, 0x1, 0x1, 0x9, 0x6, 0x0, // J
    0x9, 0xA, 0xC, 0xA, 0x9, 0x0, // K
    0x8, 0x8, 0x8, 0x8, 0xF, 0x0, // L
    0x9, 0xF, 0xF, 0x9, 0x9, 0x0, // M
    0x9, 0xD, 0xB, 0x9, 0x9, 0x0, // N
    0x6, 0x9, 0x9, 0x9, 0x6, 0x0, // O
    0xE, 0x9, 0xE, 0x8, 0x8, 0x0, // P
    0x6, 0x9, 0x9, 0xA, 0x5, 0x0, // Q
    0xE, 0x9, 0xE, 0xA, 0x9, 0x0, // R
    0x7, 0x8, 0x6, 0x1, 0xE, 0x0, // S
    0xF, 0x4, 0x4, 0x4, 0x4, 0x0, // T
    0x9, 0x9, 0x9, 0x9, 0x6, 0x0, // U
    0x9, 0x9, 0x9, 0x5, 0x2, 0x0, // V
    0x9, 0x9, 0xF, 0xF, 0x9, 0x0, // W
    0x9, 0x5, 0x2, 0x5, 0x9, 0x0, // X
    0x9, 0x5, 0x2, 0x2, 0x2, 0x0, // Y
    0xF, 0x1, 0x6, 0x8, 0xF, 0x0, // Z
    0x6, 0x9, 0x9, 0x9, 0x6, 0x0, // 0
    0x2, 0x6, 0x2, 0x2, 0x7, 0x0, // 1
    0x6, 0x9, 0x2, 0x4, 0xF, 0x0, // 2
    0xF, 0x1, 0x6, 0x1, 0xF, 0x0, // 3
    0x8, 0xA, 0xF, 0x2, 0x2, 0x0, // 4
    0xF, 0x8, 0xE, 0x1, 0xE, 0x0, // 5
    0x6, 0x8, 0xE, 0x9, 0x6, 0x0, // 6
    0xF, 0x1, 0x2, 0x4, 0x4, 0x0, // 7
    0x6, 0x9, 0x6, 0x9, 0x6, 0x0, // 8
    0x6, 0x9, 0x7, 0x1, 0x6, 0x0, // 9
    0x0, 0x2, 0x0, 0x2, 0x0, 0x0, // :
    0x0, 0x0, 0xF, 0x0, 0x0, 0x0, // -
    0x0, 0x0, 0x0, 0x0, 0x2, 0x0, // .
    0x2, 0x4, 0x4, 0x4, 0x2, 0x0, // (
    0x4, 0x2, 0x2, 0x2, 0x4, 0x0, // )
];

const VERTEX_SHADER_SOURCE: &str = r#"#version 330 core
layout (location = 0) in vec2 aPos;
layout (location = 1) in vec2 aTex;
out vec2 TexCoord_out;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
    TexCoord_out = aTex;
}"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"#version 330 core
out vec4 FragColor;
in vec2 TexCoord_out;
uniform sampler2D screenTexture;
uniform int filterMode;
void main() {
    vec4 baseColor = texture(screenTexture, TexCoord_out);
    if (filterMode == 2) {
        float scanline = sin(TexCoord_out.y * 1000.0) * 0.15 + 0.85;
        FragColor = vec4(baseColor.rgb * scanline, 1.0);
    } else {
        FragColor = baseColor;
    }
}"#;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Footer {
    magic: [u8; 4],
    snapshot_size: u32,
    shader_size: u32,
    pokes_size: u32,
    config_size: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Config {
    #[serde(default = "default_fullscreen")]
    pub fullscreen: bool,
    pub filtering: Option<String>,
    #[serde(default = "default_joystick")]
    pub joystick: String,
    #[serde(default = "default_border")]
    pub border: String,
    #[serde(default = "default_cheats")]
    pub cheats_enabled: bool,
    #[serde(default = "default_volume")]
    pub volume: u8,
}

fn default_fullscreen() -> bool { true }
fn default_joystick() -> String { "Off".to_string() }
fn default_border() -> String { "Full".to_string() }
fn default_cheats() -> bool { false }
fn default_volume() -> u8 { 100 }

impl Default for Config {
    fn default() -> Self {
        Self {
            fullscreen: true,
            filtering: None,
            joystick: "Off".to_string(),
            border: "Full".to_string(),
            cheats_enabled: false,
            volume: 100,
        }
    }
}

fn decompress_data(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.contains(&"--version".to_string()) || args.contains(&"-V".to_string()) {
        println!("zexe-runner {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let sound_latency = 200; // Hardcoded 200ms for RDP stability

    let exe_path = env::current_exe().context("Failed to get current exe path")?;
    
    let mut file = File::open(&exe_path).context("Failed to open executable")?;
    let footer_size = mem::size_of::<Footer>() as i64;
    let file_len = file.metadata()?.len();
    
    let mut snapshot_data = Vec::new();
    let mut embedded_shader = None;
    let mut embedded_pokes = None;
    let mut embedded_config = None;

    if file_len >= footer_size as u64 {
        file.seek(SeekFrom::End(-footer_size))?;
        let mut footer_buf = [0u8; std::mem::size_of::<Footer>()];
        file.read_exact(&mut footer_buf)?;
        let footer: Footer = unsafe { mem::transmute(footer_buf) };

        if &footer.magic == FOOTER_MAGIC {
            // Read and Decompress Snapshot
            let snapshot_offset = file_len - (footer_size as u64) - (footer.config_size as u64) - (footer.pokes_size as u64) - (footer.shader_size as u64) - (footer.snapshot_size as u64);
            file.seek(SeekFrom::Start(snapshot_offset))?;
            let mut comp_snap_data = vec![0u8; footer.snapshot_size as usize];
            file.read_exact(&mut comp_snap_data)?;
            snapshot_data = decompress_data(&comp_snap_data).unwrap_or_default();

            // Read and Decompress Shader
            if footer.shader_size > 0 {
                let shader_offset = file_len - (footer_size as u64) - (footer.config_size as u64) - (footer.pokes_size as u64) - (footer.shader_size as u64);
                file.seek(SeekFrom::Start(shader_offset))?;
                let mut comp_shader_data = vec![0u8; footer.shader_size as usize];
                file.read_exact(&mut comp_shader_data)?;
                if let Ok(decomp) = decompress_data(&comp_shader_data)
                    && let Ok(s) = String::from_utf8(decomp) {
                        embedded_shader = Some(s);
                }
            }

            // Read and Decompress Pokes
            if footer.pokes_size > 0 {
                let pokes_offset = file_len - (footer_size as u64) - (footer.config_size as u64) - (footer.pokes_size as u64);
                file.seek(SeekFrom::Start(pokes_offset))?;
                let mut comp_pokes_data = vec![0u8; footer.pokes_size as usize];
                file.read_exact(&mut comp_pokes_data)?;
                if let Ok(decomp) = decompress_data(&comp_pokes_data)
                    && let Ok(s) = String::from_utf8(decomp) {
                        embedded_pokes = Some(s);
                }
            }

            // Read and Decompress Config
            if footer.config_size > 0 {
                let config_offset = file_len - (footer_size as u64) - (footer.config_size as u64);
                file.seek(SeekFrom::Start(config_offset))?;
                let mut comp_config_data = vec![0u8; footer.config_size as usize];
                file.read_exact(&mut comp_config_data)?;
                if let Ok(decomp) = decompress_data(&comp_config_data)
                    && let Ok(c) = serde_json::from_slice::<Config>(&decomp) {
                        embedded_config = Some(c);
                }
            }
        }
    }

    run_emulator(&snapshot_data, embedded_shader, embedded_pokes, embedded_config, sound_latency)
}

fn run_emulator(snapshot_data: &[u8], embedded_shader: Option<String>, embedded_pokes: Option<String>, embedded_config: Option<Config>, sound_latency: u32) -> Result<()> {
    let mut app = App::new(snapshot_data, embedded_shader, embedded_pokes, embedded_config, sound_latency)?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct App {
    emulator: Emulator<AppHost>,
    window: Option<Rc<Window>>,
    
    // OpenGL state
    gl: Option<glow::Context>,
    gl_surface: Option<GlutinSurface<WindowSurface>>,
    gl_context: Option<PossiblyCurrentContext>,
    gl_program: Option<glow::Program>,
    gl_texture: Option<glow::Texture>,
    gl_vao: Option<glow::VertexArray>,
    gl_vbo: Option<glow::Buffer>,

    is_fullscreen: bool,
    border_mode: BorderMode,
    filtering_mode: FilteringMode,
    joystick_mode: JoystickMode,
    pokes: Vec<PokeEntry>,
    pokes_enabled: bool,
    osd_message: Option<String>,
    osd_timeout: Option<Instant>,
    
    // Configurable Shaders
    embedded_shader_source: Option<String>,
    embedded_program: Option<glow::Program>,
    retro_shader_source: Option<String>,
    retro_program: Option<glow::Program>,

    // Audio
    _audio_stream: cpal::Stream,
    audio_producer: HeapProducer<f32>,
    audio_channels: u16,

    modifiers: ModifiersState,
    last_frame_time: Instant,
    target_frame_duration: Duration,
    is_full_speed: bool,
    current_volume: u8,
    is_muted: bool,
}

impl App {
    fn new(snapshot_data: &[u8], embedded_shader: Option<String>, embedded_pokes: Option<String>, embedded_config: Option<Config>, sound_latency: u32) -> Result<Self> {
        // Audio Setup
        let audio_host = cpal::default_host();
        let audio_device = audio_host.default_output_device().context("No audio device")?;
        let config = audio_device.default_output_config()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        
        let rb = HeapRb::<f32>::new(sample_rate as usize * channels as usize); // 1.0s buffer for RDP
        let (producer, mut consumer) = rb.split();

        let mut last_sample = 0.0;
        let mut stream_config: cpal::StreamConfig = config.into();
        // Set hardware buffer size based on requested latency
        let buffer_frames = (sample_rate as f32 * (sound_latency as f32 / 1000.0)) as u32;
        stream_config.buffer_size = cpal::BufferSize::Fixed(buffer_frames.max(512));

        let audio_stream = audio_device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _| {
                for sample in data.iter_mut() {
                    if let Some(s) = consumer.pop() {
                        last_sample = s;
                        *sample = s;
                    } else {
                        *sample = last_sample;
                    }
                }
            },
            |err| eprintln!("Audio stream error: {:?}", err),
            None
        )?;

        let config_volume = embedded_config.as_ref().map(|c| c.volume).unwrap_or(100);
        let mut machine = ZXMachine::Sinclair48K;
        let mut loaded_data = snapshot_data.to_vec();

        if !snapshot_data.is_empty() {
            if snapshot_data.len() == 49179 {
                // Sna 48K
            } else if snapshot_data.starts_with(b"ZXST") {
                if let Ok((data, m)) = szx_loader::convert_szx_to_sna(snapshot_data) {
                    loaded_data = data;
                    machine = m;
                }
            } else if let Ok((data, m)) = z80_loader::convert_z80_to_sna(snapshot_data) {
                loaded_data = data;
                machine = m;
            }
        }

        let settings = RustzxSettings {
            machine,
            emulation_mode: EmulationMode::FrameCount(1),
            tape_fastload_enabled: true,
            kempston_enabled: true,
            mouse_enabled: false,
            load_default_rom: true,
            sound_enabled: true,
            sound_sample_rate: sample_rate as usize,
            beeper_enabled: true,
            ay_enabled: true,
            ay_mode: rustzx_core::zx::sound::ay::ZXAYMode::ABC,
            sound_volume: 100,
        };

        let mut emulator: Emulator<AppHost> = Emulator::new(settings, ())
            .map_err(|e| anyhow::anyhow!("Failed to init emulator: {:?}", e))?;

        if !snapshot_data.is_empty() {
            let cursor = BufferCursor::new(loaded_data);
            let snapshot = Snapshot::Sna(cursor);
            let _ = emulator.load_snapshot(snapshot);
        }

        let retro_shader = load_retro_shader();
        let default_filtering = if embedded_shader.is_some() {
            FilteringMode::Embedded
        } else if retro_shader.is_some() {
            FilteringMode::Custom
        } else {
            FilteringMode::Nearest
        };

        let mut app = Self {
            emulator,
            window: None,
            gl: None,
            gl_surface: None,
            gl_context: None,
            gl_program: None,
            gl_texture: None,
            gl_vao: None,
            gl_vbo: None,
            is_fullscreen: true,
            border_mode: BorderMode::Full,
            filtering_mode: default_filtering,
            joystick_mode: JoystickMode::Off,
            pokes: if let Some(p) = embedded_pokes {
                parse_pokes_content(&p)
            } else {
                load_pokes()
            },
            pokes_enabled: embedded_config.as_ref().map(|c| c.cheats_enabled).unwrap_or(false),
            osd_message: None,
            osd_timeout: None,
            embedded_shader_source: embedded_shader,
            embedded_program: None,
            retro_shader_source: retro_shader,
            retro_program: None,
            _audio_stream: audio_stream,
            audio_producer: producer,
            audio_channels: channels,
            modifiers: ModifiersState::default(),
            last_frame_time: Instant::now(),
            target_frame_duration: Duration::from_micros(20000),
            is_full_speed: false,
            current_volume: config_volume,
            is_muted: false,
        };

        // Prime the audio buffer (pre-fill with requested latency)
        let priming_frames = (sound_latency / 20).max(5);
        for _ in 0..priming_frames {
            let _ = app.emulator.emulate_frames(app.target_frame_duration);
            app.push_audio_samples();
        }
        
        // Start audio AFTER priming
        app._audio_stream.play()?;

        // Apply pokes if enabled on startup
        if app.pokes_enabled && !app.pokes.is_empty() {
            let mut actions = Vec::new();
            for p in &app.pokes {
                actions.push(PokeAction::mem(p.addr, p.value));
            }
            app.emulator.execute_poke(ManualPoke { actions });
        }

        // Apply dynamic config
        if let Some(c) = embedded_config {
            app.is_fullscreen = c.fullscreen;
            if let Some(f_mode) = &c.filtering {
                app.filtering_mode = match f_mode.as_str() {
                    "Nearest" => FilteringMode::Nearest,
                    "Linear" => FilteringMode::Linear,
                    "Scanlines" => FilteringMode::Scanlines,
                    "Embedded" => {
                        if app.embedded_shader_source.is_some() {
                            FilteringMode::Embedded
                        } else {
                            FilteringMode::Scanlines
                        }
                    },
                    "Custom" => {
                        if app.retro_shader_source.is_some() {
                            FilteringMode::Custom
                        } else {
                            FilteringMode::Scanlines
                        }
                    },
                    _ => FilteringMode::Scanlines,
                };
            }
            app.joystick_mode = match c.joystick.as_str() {
                "Off" => JoystickMode::Off,
                "Kempston" => JoystickMode::Kempston,
                "Sinclair1" => JoystickMode::Sinclair1,
                "Sinclair2" => JoystickMode::Sinclair2,
                "Cursor" => JoystickMode::Cursor,
                _ => JoystickMode::Off,
            };
            app.border_mode = match c.border.as_str() {
                "Full" => BorderMode::Full,
                "Minimal" => BorderMode::Minimal,
                "None" => BorderMode::None,
                _ => BorderMode::Full,
            };
        }

        Ok(app)
    }
}

impl App {
    fn set_osd(&mut self, text: &str) {
        self.osd_message = Some(text.to_string());
        self.osd_timeout = Some(Instant::now() + Duration::from_secs(2));
    }

    // Volume control helpers
    fn set_volume(&mut self, vol: u8) {
        self.current_volume = vol;
    }

    fn get_volume(&self) -> u8 {
        self.current_volume
    }

    fn toggle_mute(&mut self) {
        self.is_muted = !self.is_muted;
        if self.is_muted {
            self.set_osd("MUTED");
        } else {
            self.set_osd(&format!("VOLUME: {}", self.current_volume));
        }
    }

    fn save_volume_to_config(&self) {
        // Try to update config.json in the parent dir
        let config_path = std::env::current_dir().map(|mut p| { p.push("config.json"); p }).ok();
        if let Some(path) = config_path
            && let Ok(config) = std::fs::read_to_string(&path).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).map_err(std::io::Error::other)) {
                let mut config = config;
                config["volume"] = serde_json::Value::from(self.get_volume());
                let _ = std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap());
        }
    }

    fn push_audio_samples(&mut self) {
        let vol_factor = if self.is_muted { 0.0 } else { self.current_volume as f32 / 100.0 };
        while let Some(sample) = self.emulator.next_audio_sample() {
            if self.audio_channels == 2 {
                let _ = self.audio_producer.push(sample.left * vol_factor);
                let _ = self.audio_producer.push(sample.right * vol_factor);
            } else {
                let val = (sample.left + sample.right) / 2.0 * vol_factor;
                for _ in 0..self.audio_channels {
                    let _ = self.audio_producer.push(val);
                }
            }
        }
    }
}

fn draw_osd_buffer(
    text: &str,
    buffer: &mut [u32],
    window_w: usize,
    window_h: usize,
    scale: usize,
    padding: usize,
) {
    let char_spacing = 1;
    
    for (i, c) in text.chars().enumerate() {
        let offset = match c {
            ' ' => continue,
            'A'..='Z' => (c as usize - 'A' as usize) * 6,
            'a'..='z' => (c as usize - 'a' as usize) * 6, // Handle lowercase if we have them (we don't but let's be safe)
            '0'..='9' => (26 + (c as usize - '0' as usize)) * 6,
            ':' => 36 * 6,
            '-' => 37 * 6,
            '.' => 38 * 6,
            '(' => 39 * 6,
            ')' => 40 * 6,
            _ => continue,
        };
        
        let char_x = padding + i * (FONT_WIDTH + char_spacing) * scale;
        
        for fy in 0..FONT_HEIGHT {
            let row = FONT_DATA[offset + fy];
            for fx in 0..FONT_WIDTH {
                if (row >> (3 - fx)) & 1 != 0 {
                    for py in 0..scale {
                        for px in 0..scale {
                            let x = char_x + fx * scale + px;
                            let y = padding + fy * scale + py;
                            if x < window_w && y < window_h {
                                buffer[y * window_w + x] = 0xFFFFFF00; // Yellow
                            }
                        }
                    }
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let win_attrs = Window::default_attributes()
                .with_title("Zexe (F1-About, F2-Filter, F3-Joy, F4-Border, F5-FS, F9-Mute, F10-Speed, ESC-Exit)")
                .with_inner_size(LogicalSize::new(640, 480));
            
            // 1. Initial Glutin / Windowing
            let template = glutin::config::ConfigTemplateBuilder::new();
            let display_builder = glutin_winit::DisplayBuilder::new().with_window_attributes(Some(win_attrs));
            let (window, gl_config) = display_builder.build(event_loop, template, |configs| {
                configs.reduce(|accum, config| {
                    if config.num_samples() > accum.num_samples() { config } else { accum }
                }).unwrap()
            }).expect("Failed to create window/config");

            let window = Rc::new(window.unwrap());
            self.window = Some(window.clone());

            if self.is_fullscreen {
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }

            // 2. Context creation (Generic Handle acquisition)
            let gl_display = gl_config.display();
            let context_attributes = ContextAttributesBuilder::new()
                .with_context_api(glutin::context::ContextApi::OpenGl(None))
                .build(Some(window.window_handle().unwrap().as_raw()));
            
            let gl_context = unsafe {
                gl_display.create_context(&gl_config, &context_attributes).expect("Failed to create context")
            };

            // 3. Surface creation
            let size = window.inner_size();
            let attrs = SurfaceAttributesBuilder::<WindowSurface>::new()
                .build(window.window_handle().unwrap().as_raw(), NonZeroU32::new(size.width).unwrap(), NonZeroU32::new(size.height).unwrap());
            
            let gl_surface = unsafe {
                gl_config.display().create_window_surface(&gl_config, &attrs).unwrap()
            };
            
            // VSync is handled by the OS/Driver usually. 

            // Make context current
            let gl_context = gl_context.make_current(&gl_surface).unwrap();
            
            // Disable VSync to prevent blocking on RDP/Remote display drivers
            let _ = gl_surface.set_swap_interval(&gl_context, glutin::surface::SwapInterval::DontWait);

            // 4. Glow initialization
            let gl = unsafe {
                glow::Context::from_loader_function(|s| {
                    let s_ptr = std::ffi::CString::new(s).unwrap();
                    gl_display.get_proc_address(s_ptr.as_c_str())
                })
            };

            // 5. Shader / Geometry setup
            unsafe {
                let program = gl.create_program().expect("Cannot create program");
                
                let vs = gl.create_shader(glow::VERTEX_SHADER).expect("Cannot create vertex shader");
                gl.shader_source(vs, VERTEX_SHADER_SOURCE);
                gl.compile_shader(vs);
                if !gl.get_shader_compile_status(vs) { panic!("{}", gl.get_shader_info_log(vs)); }
                
                let fs = gl.create_shader(glow::FRAGMENT_SHADER).expect("Cannot create fragment shader");
                gl.shader_source(fs, FRAGMENT_SHADER_SOURCE);
                gl.compile_shader(fs);
                if !gl.get_shader_compile_status(fs) { panic!("{}", gl.get_shader_info_log(fs)); }
                
                gl.attach_shader(program, vs);
                gl.attach_shader(program, fs);
                gl.link_program(program);
                if !gl.get_program_link_status(program) { panic!("{}", gl.get_program_info_log(program)); }
                
                gl.detach_shader(program, vs);
                gl.detach_shader(program, fs);
                gl.delete_shader(vs);
                gl.delete_shader(fs);
                
                let vao = gl.create_vertex_array().ok();
                let vbo = gl.create_buffer().ok();
                
                gl.bind_vertex_array(vao);
                gl.bind_buffer(glow::ARRAY_BUFFER, vbo);
                
                // Quad: x, y, tx, ty
                let vertices: [f32; 16] = [
                    -1.0,  1.0,  0.0, 0.0,
                     1.0,  1.0,  1.0, 0.0,
                    -1.0, -1.0,  0.0, 1.0,
                     1.0, -1.0,  1.0, 1.0,
                ];
                let v_bytes = std::slice::from_raw_parts(vertices.as_ptr() as *const u8, vertices.len() * 4);
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, v_bytes, glow::STATIC_DRAW);
                
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 4 * 4, 0);
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 4 * 4, 2 * 4);
                gl.enable_vertex_attrib_array(1);
                
                let texture = gl.create_texture().ok();
                gl.bind_texture(glow::TEXTURE_2D, texture);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_SWIZZLE_A, glow::ONE as i32);
                
                // Initialize immutable storage (320x240)
                gl.tex_image_2d(
                    glow::TEXTURE_2D, 0, glow::RGBA as i32, 320, 240, 0,
                    glow::BGRA, glow::UNSIGNED_BYTE, None
                );

                self.gl_program = Some(program);

                // Compile Embedded Shader if exists
                if let Some(source) = &self.embedded_shader_source
                    && let Some(p) = compile_retro_shader_source(&gl, source) {
                        self.embedded_program = Some(p);
                }

                // Compile External Retro Shader if exists
                if let Some(source) = &self.retro_shader_source
                    && let Some(p) = compile_retro_shader_source(&gl, source) {
                        self.retro_program = Some(p);
                }

                self.gl_vao = vao;
                self.gl_vbo = vbo;
                self.gl_texture = texture;
                self.gl = Some(gl);
                self.gl_context = Some(gl_context);
                self.gl_surface = Some(gl_surface);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if let Some(window) = &self.window {
            if window.id() != window_id { return; }

            match event {
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                },
                WindowEvent::Resized(size) => {
                    if let (Some(gl_surface), Some(gl_context), Some(non_zero_w), Some(non_zero_h)) = 
                        (&self.gl_surface, &self.gl_context, NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                         gl_surface.resize(gl_context, non_zero_w, non_zero_h);
                         if let Some(gl) = &self.gl {
                             unsafe { gl.viewport(0, 0, size.width as i32, size.height as i32); }
                         }
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let (Some(gl), Some(gl_surface), Some(gl_context)) = (&self.gl, &self.gl_surface, &self.gl_context) {
                        let size = window.inner_size();
                        
                        let screen_buf = self.emulator.screen_buffer().get_buffer();
                        let border_buf_ptr = self.emulator.border_buffer().get_buffer();

                        // Source viewport
                        let (src_w, src_h, src_x_off, src_y_off) = match self.border_mode {
                            BorderMode::Full => (320, 240, 0, 0),
                            BorderMode::Minimal => (288, 224, 16, 8),
                            BorderMode::None => (256, 192, 32, 24),
                        };

                        // GPU handles the mixing and alpha via Swizzle

                        unsafe {
                            let _ = gl_context.make_current(gl_surface);
                            gl.clear_color(0.0, 0.0, 0.0, 1.0); // Reset to Black
                            gl.clear(glow::COLOR_BUFFER_BIT);

                            let use_retro = (self.filtering_mode == FilteringMode::Custom) && self.retro_program.is_some();
                            let use_embedded = (self.filtering_mode == FilteringMode::Embedded) && self.embedded_program.is_some();
                            
                            let current_program = if use_retro { 
                                self.retro_program.unwrap() 
                            } else if use_embedded {
                                self.embedded_program.unwrap()
                            } else { 
                                self.gl_program.unwrap() 
                            };
                            
                            gl.use_program(Some(current_program));
                            
                            // Filtering
                            let filter = match self.filtering_mode {
                                FilteringMode::Nearest => glow::NEAREST,
                                _ => glow::LINEAR,
                            };
                            
                            gl.bind_texture(glow::TEXTURE_2D, self.gl_texture);
                            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
                            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);
                            
                            // Maintain Aspect Ratio and SCALE to fill window
                            let s = (size.width as f32 / src_w as f32).min(size.height as f32 / src_h as f32);
                            let vis_draw_w = src_w as f32 * s;
                            let vis_draw_h = src_h as f32 * s;
                            let vis_x = (size.width as f32 - vis_draw_w) / 2.0;
                            let vis_y = (size.height as f32 - vis_draw_h) / 2.0;

                            // 1. Upload Border sub-rectangle to (0,0) in texture
                            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 320);
                            let border_offset = (src_y_off as usize * 320 + src_x_off as usize) * 4;
                            let border_buf_u8 = std::slice::from_raw_parts(
                                (border_buf_ptr.as_ptr() as *const u8).add(border_offset),
                                (src_h as usize * 320) * 4 // Over-read but within buffer limits
                            );
                            gl.tex_sub_image_2d(
                                glow::TEXTURE_2D, 0, 0, 0, src_w as i32, src_h as i32,
                                glow::BGRA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(border_buf_u8)
                            );
                            
                            // 2. Overlay Screen (256x192 at relative pos)
                            let screen_rel_x = (32 - src_x_off as i32).max(0);
                            let screen_rel_y = (24 - src_y_off as i32).max(0);
                            
                            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 256);
                            let screen_buf_u8 = std::slice::from_raw_parts(
                                screen_buf.as_ptr() as *const u8,
                                screen_buf.len() * 4
                            );
                            gl.tex_sub_image_2d(
                                glow::TEXTURE_2D, 0, screen_rel_x, screen_rel_y, 256, 192,
                                glow::BGRA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(screen_buf_u8)
                            );
                            
                            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);

                            // 3. Optional OSD Overlay
                            if let (Some(text), Some(timeout)) = (&self.osd_message, &self.osd_timeout)
                                && Instant::now() < *timeout {
                                    let char_spacing = 1;
                                    let scale = 1; 
                                    let padding = 4;
                                    let text_w = text.len() * (FONT_WIDTH + char_spacing) * scale + padding * 2;
                                    let text_h = FONT_HEIGHT * scale + padding * 2;
                                    
                                    // Target relative to visible area (8, 8)
                                    let target_x = 8;
                                    let target_y = 8;
                                    
                                    let mut osd_pixels = vec![0u32; text_w * text_h];
                                    
                                    // Compose semi-transparent background on CPU
                                    for y in 0..text_h {
                                        for x in 0..text_w {
                                            let gx = src_x_off as usize + target_x + x;
                                            let gy = src_y_off as usize + target_y + y;
                                            if gx < 320 && gy < 240 {
                                                let bg = if (32..288).contains(&gx) && (24..216).contains(&gy) {
                                                    screen_buf[(gy - 24) * 256 + (gx - 32)]
                                                } else {
                                                    border_buf_ptr[gy * 320 + gx]
                                                };
                                                // 50% dark overlay
                                                let b = (bg & 0xFF) >> 1;
                                                let g = ((bg >> 8) & 0xFF) >> 1;
                                                let r = ((bg >> 16) & 0xFF) >> 1;
                                                osd_pixels[y * text_w + x] = b | (g << 8) | (r << 16) | 0xFF000000;
                                            }
                                        }
                                    }

                                    draw_osd_buffer(text, &mut osd_pixels, text_w, text_h, scale, padding);
                                    
                                    let osd_buf_u8 = std::slice::from_raw_parts(
                                        osd_pixels.as_ptr() as *const u8,
                                        osd_pixels.len() * 4
                                    );
                                    
                                    gl.tex_sub_image_2d(
                                        glow::TEXTURE_2D, 0, target_x as i32, target_y as i32, text_w as i32, text_h as i32,
                                        glow::BGRA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(osd_buf_u8)
                                    );
                            }

                            // Common Uniforms
                            let identity: [f32; 16] = [
                                1.0, 0.0, 0.0, 0.0,
                                0.0, 1.0, 0.0, 0.0,
                                0.0, 0.0, 1.0, 0.0,
                                0.0, 0.0, 0.0, 1.0
                            ];

                            if use_retro || use_embedded {
                                // Bind RetroArch Uniforms
                                if let Some(loc_mvp) = gl.get_uniform_location(current_program, "MVPMatrix") {
                                    gl.uniform_matrix_4_f32_slice(Some(&loc_mvp), false, &identity);
                                }
                                gl.uniform_2_f32(gl.get_uniform_location(current_program, "InputSize").as_ref(), src_w as f32, src_h as f32);
                                gl.uniform_2_f32(gl.get_uniform_location(current_program, "TextureSize").as_ref(), 320.0, 240.0);
                                gl.uniform_2_f32(gl.get_uniform_location(current_program, "OutputSize").as_ref(), vis_draw_w as f32, vis_draw_h as f32);
                                
                                if let Some(loc_src) = gl.get_uniform_location(current_program, "source") {
                                    gl.uniform_1_i32(Some(&loc_src), 0);
                                }
                                if let Some(loc_txt) = gl.get_uniform_location(current_program, "Texture") {
                                    gl.uniform_1_i32(Some(&loc_txt), 0);
                                }
                                if let Some(loc_mvp) = gl.get_uniform_location(current_program, "modelViewProj") {
                                    gl.uniform_matrix_4_f32_slice(Some(&loc_mvp), false, &identity);
                                }
                            } else {
                                // Internal Uniforms
                                if let Some(loc_mvp) = gl.get_uniform_location(current_program, "MVPMatrix") {
                                    gl.uniform_matrix_4_f32_slice(Some(&loc_mvp), false, &identity);
                                }
                                if let Some(loc_tex) = gl.get_uniform_location(current_program, "screenTexture") {
                                    gl.uniform_1_i32(Some(&loc_tex), 0);
                                }
                                let mode_val = match self.filtering_mode {
                                    FilteringMode::Nearest => 0,
                                    FilteringMode::Linear => 1,
                                    FilteringMode::Scanlines => 2,
                                    FilteringMode::Embedded => 3,
                                    FilteringMode::Custom => 4,
                                };
                                gl.uniform_1_i32(gl.get_uniform_location(current_program, "filterMode").as_ref(), mode_val);
                            }
                            
                            // 4. Update Quad UVs to match visible area in (0,0)-based texture
                            let u_max = src_w as f32 / 320.0;
                            let v_max = src_h as f32 / 240.0;
                            let vertices: [f32; 16] = [
                                -1.0,  1.0,  0.0,   0.0,
                                 1.0,  1.0,  u_max, 0.0,
                                -1.0, -1.0,  0.0,   v_max,
                                 1.0, -1.0,  u_max, v_max,
                            ];
                            let v_bytes = std::slice::from_raw_parts(vertices.as_ptr() as *const u8, vertices.len() * 4);
                            gl.bind_buffer(glow::ARRAY_BUFFER, self.gl_vbo);
                            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, v_bytes);

                            // GL Viewport uses bottom-up Y
                            let v_gl_x = vis_x;
                            let v_gl_y = size.height as f32 - (vis_y + vis_draw_h);

                            gl.viewport(v_gl_x as i32, v_gl_y as i32, vis_draw_w as i32, vis_draw_h as i32);
                            
                            gl.disable(glow::SCISSOR_TEST);

                            gl.bind_vertex_array(self.gl_vao);
                            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                            
                            gl_surface.swap_buffers(gl_context).unwrap();
                        }
                    }
                },
                WindowEvent::ModifiersChanged(new) => {
                    self.modifiers = new.state();
                },
                WindowEvent::KeyboardInput { event: key_event, .. } => {
                    let pressed = key_event.state == ElementState::Pressed;
                    if let PhysicalKey::Code(code) = key_event.physical_key {
                        if pressed && (code == KeyCode::F7 || code == KeyCode::F8) {
                            if !key_event.repeat {
                                let mut vol = self.get_volume() as i16;
                                if code == KeyCode::F7 {
                                    vol = (vol - 10).max(0);
                                } else {
                                    vol = (vol + 10).min(200);
                                }
                                self.set_volume(vol as u8);
                                self.save_volume_to_config();
                                self.set_osd(&format!("VOLUME: {}", vol));
                            }
                        } else if pressed && code == KeyCode::F9 {
                            if !key_event.repeat {
                                self.toggle_mute();
                            }
                        } else if pressed && code == KeyCode::F5 {
                            if !key_event.repeat {
                                self.is_fullscreen = !self.is_fullscreen;
                                if self.is_fullscreen {
                                    window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                                    self.set_osd("FULLSCREEN: ON");
                                } else {
                                    window.set_fullscreen(None);
                                    self.set_osd("FULLSCREEN: OFF");
                                }
                            }
                        } else if pressed && code == KeyCode::F4 {
                            if !key_event.repeat {
                                self.border_mode = self.border_mode.next();
                                self.set_osd(&format!("BORDER: {:?}", self.border_mode).to_uppercase());
                            }
                        } else if pressed && code == KeyCode::F2 {
                            if !key_event.repeat {
                                self.filtering_mode = self.filtering_mode.next(self.embedded_program.is_some(), self.retro_program.is_some());
                                let msg = match self.filtering_mode {
                                    FilteringMode::Nearest => "FILTER: NEAREST",
                                    FilteringMode::Linear => "FILTER: LINEAR",
                                    FilteringMode::Scanlines => "FILTER: SCANLINES",
                                    FilteringMode::Embedded => "FILTER: EMBEDDED SHADER",
                                    FilteringMode::Custom => "FILTER: CUSTOM SHADER",
                                };
                                self.set_osd(msg);
                            }
                        } else if pressed && code == KeyCode::F3 {
                            if !key_event.repeat {
                                self.joystick_mode = self.joystick_mode.next();
                                let msg = match self.joystick_mode {
                                    JoystickMode::Off => "JOYSTICK: OFF",
                                    JoystickMode::Kempston => "JOYSTICK: KEMPSTON",
                                    JoystickMode::Sinclair1 => "JOYSTICK: SINCLAIR 1 (6-0)",
                                    JoystickMode::Sinclair2 => "JOYSTICK: SINCLAIR 2 (1-5)",
                                    JoystickMode::Cursor => "JOYSTICK: CURSOR (5-8)",
                                };
                                self.set_osd(msg);
                            }
                        } else if pressed && code == KeyCode::F6 {
                            if !key_event.repeat {
                                if self.pokes.is_empty() {
                                    self.set_osd("NO POKES FOUND");
                                } else {
                                    self.pokes_enabled = !self.pokes_enabled;
                                    let mut actions = Vec::new();
                                    if self.pokes_enabled {
                                        for p in &self.pokes {
                                            actions.push(PokeAction::mem(p.addr, p.value));
                                        }
                                        self.set_osd("POKES: ON");
                                    } else {
                                        for p in &self.pokes {
                                            actions.push(PokeAction::mem(p.addr, p.original));
                                        }
                                        self.set_osd("POKES: OFF");
                                    }
                                    let p = ManualPoke { actions };
                                    self.emulator.execute_poke(p);
                                }
                            }
                        } else if pressed && code == KeyCode::F1 {
                            if !key_event.repeat {
                                self.set_osd(&format!("ZEXE v{}", env!("CARGO_PKG_VERSION")));
                            }
                        } else if pressed && code == KeyCode::F10 {
                            if !key_event.repeat {
                                self.is_full_speed = !self.is_full_speed;
                                if self.is_full_speed {
                                    self.set_osd("SPEED: FULL");
                                } else {
                                    self.set_osd("SPEED: 1X");
                                }
                            }
                        } else if pressed && code == KeyCode::Escape {
                            event_loop.exit();
                        } else {
                            // Check for Joystick Mapping
                            if self.joystick_mode != JoystickMode::Off {
                                match code {
                                    KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::AltLeft => {
                                        match self.joystick_mode {
                                            JoystickMode::Kempston => {
                                                let k = match code {
                                                    KeyCode::ArrowUp => KempstonKey::Up,
                                                    KeyCode::ArrowDown => KempstonKey::Down,
                                                    KeyCode::ArrowLeft => KempstonKey::Left,
                                                    KeyCode::ArrowRight => KempstonKey::Right,
                                                    _ => KempstonKey::Fire,
                                                };
                                                self.emulator.send_kempston_key(k, pressed);
                                                return;
                                            }
                                            JoystickMode::Sinclair1 => {
                                                let k = match code {
                                                    KeyCode::ArrowUp => SinclairKey::Up,
                                                    KeyCode::ArrowDown => SinclairKey::Down,
                                                    KeyCode::ArrowLeft => SinclairKey::Left,
                                                    KeyCode::ArrowRight => SinclairKey::Right,
                                                    _ => SinclairKey::Fire,
                                                };
                                                self.emulator.send_sinclair_key(SinclairJoyNum::Fist, k, pressed);
                                                return;
                                            }
                                            JoystickMode::Sinclair2 => {
                                                let k = match code {
                                                    KeyCode::ArrowUp => SinclairKey::Up,
                                                    KeyCode::ArrowDown => SinclairKey::Down,
                                                    KeyCode::ArrowLeft => SinclairKey::Left,
                                                    KeyCode::ArrowRight => SinclairKey::Right,
                                                    _ => SinclairKey::Fire,
                                                };
                                                // Interface 2 Joy 2 is usually 1,2,3,4,5
                                                self.emulator.send_sinclair_key(SinclairJoyNum::Second, k, pressed);
                                                return;
                                            }
                                            JoystickMode::Cursor => {
                                                // Protek/AGF/Cursor: 5=L, 6=D, 7=U, 8=R
                                                let k = match code {
                                                    KeyCode::ArrowUp => ZXKey::N7,
                                                    KeyCode::ArrowDown => ZXKey::N6,
                                                    KeyCode::ArrowLeft => ZXKey::N5,
                                                    KeyCode::ArrowRight => ZXKey::N8,
                                                    _ => ZXKey::N0, // Fire is 0
                                                };
                                                self.emulator.send_key(k, pressed);
                                                return;
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            if let Some(zx_key) = map_winit_key(code) {
                                self.emulator.send_key(zx_key, pressed);
                            }
                        }
                    }
                },
                _ => ()
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let window = match &self.window {
            Some(w) => w.clone(),
            None => return,
        };

        let now = Instant::now();
        
        if self.is_full_speed {
             let _ = self.emulator.emulate_frames(self.target_frame_duration);
             self.push_audio_samples();
             self.last_frame_time = now;
             window.request_redraw();
             event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        } else {
             let mut next_frame_time = self.last_frame_time + self.target_frame_duration;
             
             if now >= next_frame_time {
                  // How many frames are we behind? (Max 10 to avoid death spiral/huge lag)
                  let mut frames_to_run = (now.duration_since(self.last_frame_time).as_micros() / 
                                          self.target_frame_duration.as_micros()) as u32;
                  
                  if frames_to_run > 10 {
                      frames_to_run = 10;
                      self.last_frame_time = now - self.target_frame_duration * 10;
                  }
                  
                  for _ in 0..frames_to_run {
                      let _ = self.emulator.emulate_frames(self.target_frame_duration);
                      self.push_audio_samples();
                      self.last_frame_time += self.target_frame_duration;
                  }

                  window.request_redraw();
                  next_frame_time = self.last_frame_time + self.target_frame_duration;
             }
             
             event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(next_frame_time));
        }
    }
}

fn compile_retro_shader_source(gl: &glow::Context, source: &str) -> Option<glow::Program> {
    unsafe {
        let clean_source = if source.trim().starts_with("#version") {
            // Remove the first line if it's a version directive
            source.lines().skip(1).collect::<Vec<_>>().join("\n")
        } else {
            source.to_string()
        };

        let mut final_vs = String::from("#version 330 core\n");
        final_vs.push_str("#define VERTEX\n");
        final_vs.push_str(&clean_source);
        
        let mut final_fs = String::from("#version 330 core\n");
        final_fs.push_str("#define FRAGMENT\n");
        final_fs.push_str(&clean_source);

        let program = gl.create_program().expect("Cannot create retro program");
        
        let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
        gl.shader_source(vs, &final_vs);
        gl.compile_shader(vs);
        if !gl.get_shader_compile_status(vs) {
            eprintln!("Retro VS failed: {}", gl.get_shader_info_log(vs));
        }
        
        let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
        gl.shader_source(fs, &final_fs);
        gl.compile_shader(fs);
        if !gl.get_shader_compile_status(fs) {
            eprintln!("Retro FS failed: {}", gl.get_shader_info_log(fs));
        }

        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        
        gl.bind_attrib_location(program, 0, "VertexCoord");
        gl.bind_attrib_location(program, 1, "TexCoord");
        
        gl.link_program(program);
        
        if !gl.get_program_link_status(program) {
            eprintln!("Retro shader link failed: {}", gl.get_program_info_log(program));
            None
        } else {
            // Pre-bind sampler to Unit 0
            gl.use_program(Some(program));
            if let Some(loc) = gl.get_uniform_location(program, "source") {
                gl.uniform_1_i32(Some(&loc), 0);
            }
            if let Some(loc) = gl.get_uniform_location(program, "Texture") {
                gl.uniform_1_i32(Some(&loc), 0);
            }
            Some(program)
        }
    }
}

fn map_winit_key(code: KeyCode) -> Option<ZXKey> {
    match code {
        KeyCode::KeyA => Some(ZXKey::A),
        KeyCode::KeyB => Some(ZXKey::B),
        KeyCode::KeyC => Some(ZXKey::C),
        KeyCode::KeyD => Some(ZXKey::D),
        KeyCode::KeyE => Some(ZXKey::E),
        KeyCode::KeyF => Some(ZXKey::F),
        KeyCode::KeyG => Some(ZXKey::G),
        KeyCode::KeyH => Some(ZXKey::H),
        KeyCode::KeyI => Some(ZXKey::I),
        KeyCode::KeyJ => Some(ZXKey::J),
        KeyCode::KeyK => Some(ZXKey::K),
        KeyCode::KeyL => Some(ZXKey::L),
        KeyCode::KeyM => Some(ZXKey::M),
        KeyCode::KeyN => Some(ZXKey::N),
        KeyCode::KeyO => Some(ZXKey::O),
        KeyCode::KeyP => Some(ZXKey::P),
        KeyCode::KeyQ => Some(ZXKey::Q),
        KeyCode::KeyR => Some(ZXKey::R),
        KeyCode::KeyS => Some(ZXKey::S),
        KeyCode::KeyT => Some(ZXKey::T),
        KeyCode::KeyU => Some(ZXKey::U),
        KeyCode::KeyV => Some(ZXKey::V),
        KeyCode::KeyW => Some(ZXKey::W),
        KeyCode::KeyX => Some(ZXKey::X),
        KeyCode::KeyY => Some(ZXKey::Y),
        KeyCode::KeyZ => Some(ZXKey::Z),
        KeyCode::Digit0 => Some(ZXKey::N0),
        KeyCode::Digit1 => Some(ZXKey::N1),
        KeyCode::Digit2 => Some(ZXKey::N2),
        KeyCode::Digit3 => Some(ZXKey::N3),
        KeyCode::Digit4 => Some(ZXKey::N4),
        KeyCode::Digit5 => Some(ZXKey::N5),
        KeyCode::Digit6 => Some(ZXKey::N6),
        KeyCode::Digit7 => Some(ZXKey::N7),
        KeyCode::Digit8 => Some(ZXKey::N8),
        KeyCode::Digit9 => Some(ZXKey::N9),
        KeyCode::Enter => Some(ZXKey::Enter),
        KeyCode::Space => Some(ZXKey::Space),
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Some(ZXKey::Shift),
        KeyCode::ControlLeft | KeyCode::ControlRight => Some(ZXKey::SymShift),
        _ => None
    }
}
