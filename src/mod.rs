// I/O register definitions and helpers


#[allow(dead_code)]
pub mod dispcnt {
    pub const MODE_MASK: u16 = 0x07;
    pub const FRAME_SELECT: u16 = 1 << 4;
    pub const HBLANK_FREE: u16 = 1 << 5;
    pub const OBJ_1D: u16 = 1 << 6;
    pub const FORCED_BLANK: u16 = 1 << 7;
    pub const BG0_ENABLE: u16 = 1 << 8;
    pub const BG1_ENABLE: u16 = 1 << 9;
    pub const BG2_ENABLE: u16 = 1 << 10;
    pub const BG3_ENABLE: u16 = 1 << 11;
    pub const OBJ_ENABLE: u16 = 1 << 12;
    pub const WIN0_ENABLE: u16 = 1 << 13;
    pub const WIN1_ENABLE: u16 = 1 << 14;
    pub const WINOBJ_ENABLE: u16 = 1 << 15;
}

/// Interrupt flag bits
#[allow(dead_code)]
pub mod irq {
    pub const VBLANK: u16 = 1 << 0;
    pub const HBLANK: u16 = 1 << 1;
    pub const VCOUNTER: u16 = 1 << 2;
    pub const TIMER0: u16 = 1 << 3;
    pub const TIMER1: u16 = 1 << 4;
    pub const TIMER2: u16 = 1 << 5;
    pub const TIMER3: u16 = 1 << 6;
    pub const SERIAL: u16 = 1 << 7;
    pub const DMA0: u16 = 1 << 8;
    pub const DMA1: u16 = 1 << 9;
    pub const DMA2: u16 = 1 << 10;
    pub const DMA3: u16 = 1 << 11;
    pub const KEYPAD: u16 = 1 << 12;
    pub const GAMEPAK: u16 = 1 << 13;
}

/// Key input bits (active low in KEYINPUT register)
#[allow(dead_code)]
pub mod keys {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const SELECT: u16 = 1 << 2;
    pub const START: u16 = 1 << 3;
    pub const RIGHT: u16 = 1 << 4;
    pub const LEFT: u16 = 1 << 5;
    pub const UP: u16 = 1 << 6;
    pub const DOWN: u16 = 1 << 7;
    pub const R: u16 = 1 << 8;
    pub const L: u16 = 1 << 9;
}