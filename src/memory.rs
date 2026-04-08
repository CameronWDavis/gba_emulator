
use crate::io::dma_cnt;
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
    pub pending: bool,   // armed, waiting for its trigger event
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
            // DMA registers (0x0B0-0x0DF)
            // Layout per channel i: base = 0x0B0 + i*12
            //   +0..+3: SAD, +4..+7: DAD, +8..+9: CNT_L, +10..+11: CNT_H
            // CNT_H high bytes: 0x0BB (ch0), 0x0C7 (ch1), 0x0D3 (ch2), 0x0DF (ch3)
            0x0B0..=0x0DF => {
                if (reg as usize) < self.io_regs.len() {
                    self.io_regs[reg as usize] = val;
                }
                match reg {
                    0x0BB => self.on_dma_cnt_h_write(0),
                    0x0C7 => self.on_dma_cnt_h_write(1),
                    0x0D3 => self.on_dma_cnt_h_write(2),
                    0x0DF => self.on_dma_cnt_h_write(3),
                    _ => {}
                }
            }
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
        let vcount_before = self.ppu.vcount;
        let hblank_before = self.ppu.in_hblank;

        self.ppu.tick(cycles, &mut self.irf);
        self.tick_timers(cycles);

        let vcount_after = self.ppu.vcount;
        let hblank_after = self.ppu.in_hblank;

        // VBlank rising edge: vcount just crossed into the vblank period
        if vcount_before < 160 && vcount_after >= 160 {
            self.tick_dma_trigger(dma_cnt::TIMING_VBLANK);
        }
        // HBlank rising edge (only during active display lines)
        if !hblank_before && hblank_after && vcount_after < 160 {
            self.tick_dma_trigger(dma_cnt::TIMING_HBLANK);
        }
    }

    /// Fire all pending DMA channels that match the given start timing
    fn tick_dma_trigger(&mut self, timing_val: u16) {
        for ch in 0..4 {
            let ctrl = self.dma[ch].control;
            if ctrl & dma_cnt::ENABLE != 0
                && (ctrl & dma_cnt::TIMING) == timing_val
                && self.dma[ch].pending
            {
                self.run_dma(ch);
            }
        }
    }

    /// Called when the CNT_H high byte of DMA channel `ch` is written.
    /// Latches internal registers on 0->1 enable transition and fires
    /// immediate DMA right away.
    fn on_dma_cnt_h_write(&mut self, ch: usize) {
        let base = 0x0B0 + ch * 12;
        let sad = u32::from_le_bytes([
            self.io_regs[base],
            self.io_regs[base + 1],
            self.io_regs[base + 2],
            self.io_regs[base + 3],
        ]);
        let dad = u32::from_le_bytes([
            self.io_regs[base + 4],
            self.io_regs[base + 5],
            self.io_regs[base + 6],
            self.io_regs[base + 7],
        ]);
        let cnt_l = u16::from_le_bytes([self.io_regs[base + 8], self.io_regs[base + 9]]);
        let cnt_h = u16::from_le_bytes([self.io_regs[base + 10], self.io_regs[base + 11]]);

        let was_enabled = self.dma[ch].control & dma_cnt::ENABLE != 0;
        let now_enabled = cnt_h & dma_cnt::ENABLE != 0;

        self.dma[ch].src     = sad;
        self.dma[ch].dst     = dad;
        self.dma[ch].count   = cnt_l;
        self.dma[ch].control = cnt_h;

        if !was_enabled && now_enabled {
            // Latch internal registers on enable transition
            self.dma[ch].internal_src = sad;
            self.dma[ch].internal_dst = dad;
            // Count of 0 means maximum: 0x4000 for ch0-2, 0x10000 for ch3
            self.dma[ch].internal_count = if cnt_l == 0 {
                if ch == 3 { 0x10000 } else { 0x4000 }
            } else {
                cnt_l as u32
            };
            self.dma[ch].pending = true;

            // Immediate DMA fires right now
            if cnt_h & dma_cnt::TIMING == dma_cnt::TIMING_IMMEDIATE {
                self.run_dma(ch);
            }
        } else if !now_enabled {
            self.dma[ch].pending = false;
        }
    }

    /// Execute a DMA transfer for channel `ch`.
    fn run_dma(&mut self, ch: usize) {
        // Copy channel state to locals — required by borrow checker since
        // read*/write* borrow self while dma[ch] is also part of self.
        let ctrl      = self.dma[ch].control;
        let count     = self.dma[ch].internal_count;
        let raw_count = self.dma[ch].count;
        let mut src   = self.dma[ch].internal_src;
        let mut dst   = self.dma[ch].internal_dst;

        let is_32bit = ctrl & dma_cnt::WORD != 0;
        let unit: u32 = if is_32bit { 4 } else { 2 };

        let dst_ctrl = (ctrl & dma_cnt::DST_CTRL) as u32;
        let src_ctrl = ((ctrl & dma_cnt::SRC_CTRL) >> 2) as u32;

        let dst_step: i32 = match dst_ctrl {
            0 => unit as i32,
            1 => -(unit as i32),
            2 => 0,          // fixed
            _ => unit as i32, // 3 = inc+reload: increment during transfer, reload after
        };
        let src_step: i32 = match src_ctrl {
            0 => unit as i32,
            1 => -(unit as i32),
            _ => 0, // fixed or prohibited
        };

        for _ in 0..count {
            if is_32bit {
                let v = self.read32(src);
                self.write32(dst, v);
            } else {
                let v = self.read16(src);
                self.write16(dst, v);
            }
            src = src.wrapping_add_signed(src_step);
            dst = dst.wrapping_add_signed(dst_step);
        }

        // Write back updated internal addresses
        self.dma[ch].internal_src = src;
        self.dma[ch].internal_dst = dst;

        // Determine next state: repeat (timed only) or complete
        let repeat = ctrl & dma_cnt::REPEAT != 0;
        let timing = ctrl & dma_cnt::TIMING;

        if repeat && timing != dma_cnt::TIMING_IMMEDIATE {
            // Reload count for next trigger
            self.dma[ch].internal_count = if raw_count == 0 {
                if ch == 3 { 0x10000 } else { 0x4000 }
            } else {
                raw_count as u32
            };
            // dst_ctrl == 3 means reload destination address too
            if dst_ctrl == 3 {
                self.dma[ch].internal_dst = self.dma[ch].dst;
            }
            self.dma[ch].pending = true;
        } else {
            // Transfer complete — clear enable bit and sync back to io_regs
            self.dma[ch].control &= !dma_cnt::ENABLE;
            self.dma[ch].pending = false;
            let base = 0x0B0 + ch * 12;
            self.io_regs[base + 10] = self.dma[ch].control as u8;
            self.io_regs[base + 11] = (self.dma[ch].control >> 8) as u8;
        }

        // Fire IRQ if the channel requested it
        if ctrl & dma_cnt::IRQ != 0 {
            self.irf |= 1 << (8 + ch); // DMA0=bit8, DMA1=bit9, DMA2=bit10, DMA3=bit11
        }
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
