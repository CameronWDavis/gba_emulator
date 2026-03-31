/// GBA display: 240x160 pixels, 15-bit color (BGR555)
pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;

/// PPU timing constants
const HDRAW_CYCLES: u32 = 960;     // H-draw: 240 dots * 4 cycles
const HBLANK_CYCLES: u32 = 272;    // H-blank: 68 dots * 4 cycles
const SCANLINE_CYCLES: u32 = HDRAW_CYCLES + HBLANK_CYCLES; // 1232 cycles per line
const VDRAW_LINES: u16 = 160;
const VBLANK_LINES: u16 = 68;
const TOTAL_LINES: u16 = VDRAW_LINES + VBLANK_LINES; // 228

pub struct Ppu {
    pub vram: Vec<u8>,      // 96 KB
    pub palette: Vec<u8>,   // 1 KB
    pub oam: Vec<u8>,       // 1 KB

    pub vcount: u16,        // Current scanline
    pub vcount_target: u16, // V-counter match value
    pub in_hblank: bool,

    /// Framebuffer: RGBA8888
    pub framebuffer: Vec<u32>,

    // Internal cycle counter
    dot_counter: u32,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: vec![0; 96 * 1024],
            palette: vec![0; 1024],
            oam: vec![0; 1024],
            vcount: 0,
            vcount_target: 0,
            in_hblank: false,
            framebuffer: vec![0xFF000000; SCREEN_WIDTH * SCREEN_HEIGHT],
            dot_counter: 0,
        }
    }

    /// Tick the PPU by the given number of CPU cycles.
    /// Updates scanline counter, h-blank/v-blank status, and fires interrupts.
    pub fn tick(&mut self, cycles: u32, irf: &mut u16) {
        self.dot_counter += cycles;

        while self.dot_counter >= SCANLINE_CYCLES {
            self.dot_counter -= SCANLINE_CYCLES;

            // Render this scanline if in visible range
            if self.vcount < VDRAW_LINES {
                self.render_scanline();
            }

            self.vcount += 1;

            if self.vcount >= TOTAL_LINES {
                self.vcount = 0;
            }

            // V-blank interrupt
            if self.vcount == VDRAW_LINES {
                *irf |= 1; // V-blank IRQ flag
            }

            // V-counter match interrupt
            if self.vcount == self.vcount_target {
                *irf |= 4; // V-counter IRQ flag
            }
        }

        // H-blank tracking (simplified)
        self.in_hblank = self.dot_counter >= HDRAW_CYCLES;
    }

    /// Render one scanline based on current DISPCNT mode
    fn render_scanline(&mut self) {
        let y = self.vcount as usize;
        if y >= SCREEN_HEIGHT { return; }

        // TODO: Read DISPCNT from I/O regs to determine mode
        // For now, render mode 3 (240x160 bitmap, 16-bit color) as a starting point
        self.render_mode3_scanline(y);
    }

    /// Mode 3: 240x160, 16-bit color bitmap
    fn render_mode3_scanline(&mut self, y: usize) {
        let line_offset = y * SCREEN_WIDTH * 2;
        for x in 0..SCREEN_WIDTH {
            let pixel_offset = line_offset + x * 2;
            if pixel_offset + 1 < self.vram.len() {
                let color16 = self.vram[pixel_offset] as u16
                    | ((self.vram[pixel_offset + 1] as u16) << 8);
                self.framebuffer[y * SCREEN_WIDTH + x] = bgr555_to_rgba(color16);
            }
        }
    }

    /// Mode 4: 240x160, 8-bit paletted bitmap (two pages)
    #[allow(dead_code)]
    fn render_mode4_scanline(&mut self, y: usize, page: usize) {
        let base = page * 0xA000;
        let line_offset = base + y * SCREEN_WIDTH;
        for x in 0..SCREEN_WIDTH {
            let palette_idx = self.vram[line_offset + x] as usize;
            let color16 = self.palette[palette_idx * 2] as u16
                | ((self.palette[palette_idx * 2 + 1] as u16) << 8);
            self.framebuffer[y * SCREEN_WIDTH + x] = bgr555_to_rgba(color16);
        }
    }

    /// Get the completed framebuffer (call after rendering a full frame)
    pub fn get_framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }
}

/// Convert GBA BGR555 color to RGBA8888
fn bgr555_to_rgba(color: u16) -> u32 {
    let r = ((color & 0x1F) as u32) << 3;
    let g = (((color >> 5) & 0x1F) as u32) << 3;
    let b = (((color >> 10) & 0x1F) as u32) << 3;
    0xFF000000 | (r << 16) | (g << 8) | b
}