
use crate::ppu::Ppu;
 
/// GBA Memory Map:
/// 0x00000000 - 0x00003FFF  BIOS ROM (16 KB)
/// 0x02000000 - 0x0203FFFF  EWRAM - External Work RAM (256 KB)
/// 0x03000000 - 0x03007FFF  IWRAM - Internal Work RAM (32 KB)
/// 0x04000000 - 0x040003FE  I/O Registers
/// 0x05000000 - 0x050003FF  Palette RAM (1 KB)
/// 0x06000000 - 0x06017FFF  VRAM (96 KB)
/// 0x07000000 - 0x070003FF  OAM - Object Attribute Memory (1 KB)
/// 0x08000000 - 0x09FFFFFF  ROM - Wait State 0 (32 MB)
/// 0x0A000000 - 0x0BFFFFFF  ROM - Wait State 1 (32 MB)
/// 0x0C000000 - 0x0DFFFFFF  ROM - Wait State 2 (32 MB)
/// 0x0E000000 - 0x0E00FFFF  SRAM (64 KB)
pub struct Bus {
    pub bios: Option<Vec<u8>>,
    pub ewram: Vec<u8>,      // 256 KB
    pub iwram: Vec<u8>,      // 32 KB
    pub io_regs: Vec<u8>,    // I/O registers
    pub rom: Vec<u8>,
    pub sram: Vec<u8>,       // 64 KB
    pub ppu: Ppu,
 
    // Interrupt / DMA / Timer state
    pub ime: bool,           // Interrupt Master Enable
    pub ie: u16,             // Interrupt Enable
    pub irf: u16,            // Interrupt Request Flags
 
    pub halt: bool,
    pub dma: [DmaChannel; 4],
    pub timers: [Timer; 4],
 
    pub keyinput: u16,       // Joypad state (active low)
}
 
#[derive(Clone, Copy, Default)]
pub struct DmaChannel {
    pub src: u32,
    pub dst: u32,
    pub count: u16,
    pub control: u16,
    // Internal latches
    pub internal_src: u32,
    pub internal_dst: u32,
    pub internal_count: u32,
}
 
#[derive(Clone, Copy, Default)]
pub struct Timer {
    pub reload: u16,
    pub counter: u16,
    pub control: u16,
    pub internal: u32,   // Internal prescaler counter
}
 
impl Bus {
    pub fn new(rom: Vec<u8>, bios: Option<Vec<u8>>) -> Self {
        Self {
            bios,
            ewram: vec![0; 256 * 1024],
            iwram: vec![0; 32 * 1024],
            io_regs: vec![0; 0x400],
            rom,
            sram: vec![0; 64 * 1024],
            ppu: Ppu::new(),
            ime: false,
            ie: 0,
            irf: 0,
            halt: false,
            dma: [DmaChannel::default(); 4],
            timers: [Timer::default(); 4],
            keyinput: 0x03FF, // All buttons released (active low)
        }
    }
 
    /// Read a byte from the memory map
    pub fn read8(&self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 => {
                // BIOS
                if let Some(ref bios) = self.bios {
                    let idx = (addr & 0x3FFF) as usize;
                    if idx < bios.len() { bios[idx] } else { 0 }
                } else {
                    0
                }
            }
            0x02 => {
                // EWRAM (mirrored every 256KB)
                self.ewram[(addr & 0x3FFFF) as usize]
            }
            0x03 => {
                // IWRAM (mirrored every 32KB)
                self.iwram[(addr & 0x7FFF) as usize]
            }
            0x04 => {
                // I/O Registers
                self.read_io(addr)
            }
            0x05 => {
                // Palette RAM
                self.ppu.palette[(addr & 0x3FF) as usize]
            }
            0x06 => {
                // VRAM (mirrored, 96KB)
                let offset = (addr & 0x1FFFF) as usize;
                let offset = if offset >= 0x18000 { offset - 0x8000 } else { offset };
                self.ppu.vram[offset]
            }
            0x07 => {
                // OAM
                self.ppu.oam[(addr & 0x3FF) as usize]
            }
            0x08..=0x0D => {
                // ROM (mirrored across wait states)
                let offset = (addr & 0x01FFFFFF) as usize;
                if offset < self.rom.len() { self.rom[offset] } else { 0 }
            }
            0x0E..=0x0F => {
                // SRAM
                self.sram[(addr & 0xFFFF) as usize]
            }
            _ => {
                // Open bus / unused
                0
            }
        }
    }
 
    /// Read a 16-bit halfword (little-endian, force aligned)
    pub fn read16(&self, addr: u32) -> u16 {
        let addr = addr & !1; // Force align
        let lo = self.read8(addr) as u16;
        let hi = self.read8(addr + 1) as u16;
        lo | (hi << 8)
    }
 
    /// Read a 32-bit word (little-endian, force aligned)
    pub fn read32(&self, addr: u32) -> u32 {
        let addr = addr & !3; // Force align
        let b0 = self.read8(addr) as u32;
        let b1 = self.read8(addr + 1) as u32;
        let b2 = self.read8(addr + 2) as u32;
        let b3 = self.read8(addr + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
 
    /// Write a byte
    pub fn write8(&mut self, addr: u32, val: u8) {
        match addr >> 24 {
            0x02 => self.ewram[(addr & 0x3FFFF) as usize] = val,
            0x03 => self.iwram[(addr & 0x7FFF) as usize] = val,
            0x04 => self.write_io(addr, val),
            0x05 => {
                // Palette RAM - 8-bit writes write the byte to both bytes of the halfword
                let aligned = (addr & 0x3FE) as usize;
                self.ppu.palette[aligned] = val;
                self.ppu.palette[aligned + 1] = val;
            }
            0x06 => {
                // VRAM - 8-bit writes similar to palette
                let offset = (addr & 0x1FFFF) as usize;
                let offset = if offset >= 0x18000 { offset - 0x8000 } else { offset };
                let aligned = offset & !1;
                self.ppu.vram[aligned] = val;
                if aligned + 1 < self.ppu.vram.len() {
                    self.ppu.vram[aligned + 1] = val;
                }
            }
            0x07 => { /* OAM ignores 8-bit writes */ }
            0x0E..=0x0F => self.sram[(addr & 0xFFFF) as usize] = val,
            _ => {}
        }
    }
 
    /// Write a 16-bit halfword
    pub fn write16(&mut self, addr: u32, val: u16) {
        let addr = addr & !1;
        self.write8(addr, val as u8);
        self.write8(addr + 1, (val >> 8) as u8);
    }
 
    /// Write a 32-bit word
    pub fn write32(&mut self, addr: u32, val: u32) {
        let addr = addr & !3;
        self.write8(addr, val as u8);
        self.write8(addr + 1, (val >> 8) as u8);
        self.write8(addr + 2, (val >> 16) as u8);
        self.write8(addr + 3, (val >> 24) as u8);
    }
 
    /// Read from I/O registers
    fn read_io(&self, addr: u32) -> u8 {
        let reg = addr & 0x3FF;
        match reg {
            // DISPCNT
            0x000..=0x001 => self.io_regs[reg as usize],
            // DISPSTAT
            0x004 => {
                let mut val = self.io_regs[0x004];
                // V-blank flag
                if self.ppu.vcount >= 160 { val |= 0x01; }
                // H-blank flag (simplified)
                if self.ppu.in_hblank { val |= 0x02; }
                // V-counter flag
                if self.ppu.vcount == self.ppu.vcount_target { val |= 0x04; }
                val
            }
            0x005 => self.io_regs[0x005],
            // VCOUNT
            0x006 => self.ppu.vcount as u8,
            0x007 => 0,
            // KEYINPUT
            0x130 => self.keyinput as u8,
            0x131 => (self.keyinput >> 8) as u8,
            // IE
            0x200 => self.ie as u8,
            0x201 => (self.ie >> 8) as u8,
            // IF
            0x202 => self.irf as u8,
            0x203 => (self.irf >> 8) as u8,
            // IME
            0x208 => self.ime as u8,
            0x209 => 0,
            _ => self.io_regs.get(reg as usize).copied().unwrap_or(0),
        }
    }
 
    /// Write to I/O registers
    fn write_io(&mut self, addr: u32, val: u8) {
        let reg = addr & 0x3FF;
        match reg {
            // DISPCNT
            0x000..=0x001 => self.io_regs[reg as usize] = val,
            // DISPSTAT
            0x004 => {
                // Only bits 3-5 are writable (interrupt enables + vcount target low)
                self.io_regs[0x004] = (self.io_regs[0x004] & 0x07) | (val & 0xF8);
            }
            0x005 => {
                self.io_regs[0x005] = val;
                self.ppu.vcount_target = val as u16;
            }
            // BG control, scroll, etc. - store directly
            0x008..=0x05F => self.io_regs[reg as usize] = val,
            // IE
            0x200 => self.ie = (self.ie & 0xFF00) | val as u16,
            0x201 => self.ie = (self.ie & 0x00FF) | ((val as u16) << 8),
            // IF - write 1 to acknowledge/clear
            0x202 => self.irf &= !(val as u16),
            0x203 => self.irf &= !((val as u16) << 8),
            // IME
            0x208 => self.ime = val & 1 != 0,
            // HALTCNT
            0x301 => {
                self.halt = true;
            }
            _ => {
                if (reg as usize) < self.io_regs.len() {
                    self.io_regs[reg as usize] = val;
                }
            }
        }
    }
 
    /// Tick hardware components (PPU, timers, DMA, etc.)
    pub fn tick(&mut self, cycles: u32) {
        self.ppu.tick(cycles, &mut self.irf);
        self.tick_timers(cycles);
    }
 
    fn tick_timers(&mut self, cycles: u32) {
        let mut overflow = false;
        for i in 0..4 {
            let enabled = self.timers[i].control & 0x80 != 0;
            if !enabled { overflow = false; continue; }
 
            let cascade = self.timers[i].control & 0x04 != 0;
            let prescaler = match self.timers[i].control & 0x03 {
                0 => 1,
                1 => 64,
                2 => 256,
                3 => 1024,
                _ => unreachable!(),
            };
 
            let ticks = if i > 0 && cascade {
                if overflow { 1 } else { 0 }
            } else {
                cycles
            };
 
            overflow = false;
            if ticks == 0 { continue; }
 
            self.timers[i].internal += ticks;
            while self.timers[i].internal >= prescaler {
                self.timers[i].internal -= prescaler;
                let (new_val, did_overflow) = self.timers[i].counter.overflowing_add(1);
                if did_overflow {
                    self.timers[i].counter = self.timers[i].reload;
                    overflow = true;
                    // Timer IRQ
                    if self.timers[i].control & 0x40 != 0 {
                        self.irf |= 1 << (3 + i);
                    }
                } else {
                    self.timers[i].counter = new_val;
                }
            }
        }
    }
}