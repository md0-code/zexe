use rustzx_core::host::{
    FrameBuffer, FrameBufferSource, Host, HostContext, StubDebugInterface, StubIoExtender,
    Stopwatch as StopwatchTrait
};
use rustzx_core::zx::video::colors::{ZXBrightness, ZXColor};
use std::time::{Duration, Instant};

// --- Stopwatch ---
pub struct Stopwatch {
    timestamp: Instant,
}

impl StopwatchTrait for Stopwatch {
    fn new() -> Self {
        Self {
            timestamp: Instant::now(),
        }
    }

    fn measure(&self) -> Duration {
        self.timestamp.elapsed()
    }
}

// --- FrameBuffer ---
// 32-bit ARGB buffer (00RRGGBB)
pub struct EmulatorFrameBuffer {
    pub buffer: Vec<u32>,
    pub width: usize,
    pub height: usize,
}

impl EmulatorFrameBuffer {
    pub fn get_buffer(&self) -> &[u32] {
        &self.buffer
    }
}

impl FrameBuffer for EmulatorFrameBuffer {
    type Context = ();

    fn new(width: usize, height: usize, _source: FrameBufferSource, _context: Self::Context) -> Self {
        Self {
            buffer: vec![0; width * height],
            width,
            height,
        }
    }

    fn set_color(&mut self, x: usize, y: usize, color: ZXColor, brightness: ZXBrightness) {
        if x < self.width && y < self.height {
            let color_u32 = zx_color_to_u32(color, brightness);
            self.buffer[y * self.width + x] = color_u32;
        }
    }
}


fn zx_color_to_u32(color: ZXColor, brightness: ZXBrightness) -> u32 {
    let bright = match brightness {
        ZXBrightness::Bright => true,
        ZXBrightness::Normal => false,
    };

    match (color, bright) {
        (ZXColor::Black, _) => 0xFF000000,
        (ZXColor::Blue, false) => 0xFF0000CD,
        (ZXColor::Blue, true) => 0xFF0000FF,
        (ZXColor::Red, false) => 0xFFCD0000,
        (ZXColor::Red, true) => 0xFFFF0000,
        (ZXColor::Purple, false) => 0xFFCD00CD,
        (ZXColor::Purple, true) => 0xFFFF00FF,
        (ZXColor::Green, false) => 0xFF00CD00,
        (ZXColor::Green, true) => 0xFF00FF00,
        (ZXColor::Cyan, false) => 0xFF00CDCD,
        (ZXColor::Cyan, true) => 0xFF00FFFF,
        (ZXColor::Yellow, false) => 0xFFCDCD00,
        (ZXColor::Yellow, true) => 0xFFFFFF00,
        (ZXColor::White, false) => 0xFFCDCDCD,
        (ZXColor::White, true) => 0xFFFFFFFF,
    }
}

// --- Host Implementation ---
pub struct AppHost;

impl Host for AppHost {
    type Context = ();
    type TapeAsset = rustzx_core::host::BufferCursor<Vec<u8>>;
    type FrameBuffer = EmulatorFrameBuffer;
    type EmulationStopwatch = Stopwatch;
    type IoExtender = StubIoExtender;
    type DebugInterface = StubDebugInterface;
}

impl HostContext<AppHost> for () {
    fn frame_buffer_context(&self) {}
}

