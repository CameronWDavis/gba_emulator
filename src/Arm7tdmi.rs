use crate::memory::Bus;

/// ARM7TDMI operating modes
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Mode {
    User       = 0b10000,
    Fiq        = 0b10001,
    Irq        = 0b10010,
    Supervisor = 0b10011,
    Abort      = 0b10111,
    Undefined  = 0b11011,
    System     = 0b11111,
}

impl Mode {
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0x1F {
            0b10000 => Mode::User,
            0b10001 => Mode::Fiq,
            0b10010 => Mode::Irq,
            0b10011 => Mode::Supervisor,
            0b10111 => Mode::Abort,
            0b11011 => Mode::Undefined,
            0b11111 => Mode::System,
            _ => Mode::User, // fallback
        }
    }

    /// Index into banked register arrays
    pub fn bank_index(self) -> usize {
        match self {
            Mode::User | Mode::System => 0,
            Mode::Fiq => 1,
            Mode::Irq => 2,
            Mode::Supervisor => 3,
            Mode::Abort => 4,
            Mode::Undefined => 5,
        }
    }
}

/// CPSR / SPSR flags
pub struct Psr(pub u32);

impl Psr {
    pub fn n(&self) -> bool { self.0 & (1 << 31) != 0 }
    pub fn z(&self) -> bool { self.0 & (1 << 30) != 0 }
    pub fn c(&self) -> bool { self.0 & (1 << 29) != 0 }
    pub fn v(&self) -> bool { self.0 & (1 << 28) != 0 }
    pub fn i(&self) -> bool { self.0 & (1 << 7) != 0 }  // IRQ disable
    pub fn f(&self) -> bool { self.0 & (1 << 6) != 0 }  // FIQ disable
    pub fn t(&self) -> bool { self.0 & (1 << 5) != 0 }  // Thumb state
    pub fn mode(&self) -> Mode { Mode::from_bits((self.0 & 0x1F) as u8) }

    pub fn set_n(&mut self, v: bool) { self.set_bit(31, v); }
    pub fn set_z(&mut self, v: bool) { self.set_bit(30, v); }
    pub fn set_c(&mut self, v: bool) { self.set_bit(29, v); }
    pub fn set_v(&mut self, v: bool) { self.set_bit(28, v); }
    pub fn set_i(&mut self, v: bool) { self.set_bit(7, v); }
    pub fn set_t(&mut self, v: bool) { self.set_bit(5, v); }

    fn set_bit(&mut self, bit: u32, v: bool) {
        if v { self.0 |= 1 << bit; } else { self.0 &= !(1 << bit); }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.0 = (self.0 & !0x1F) | (mode as u32);
    }

    pub fn set_nz(&mut self, val: u32) {
        self.set_n(val & 0x80000000 != 0);
        self.set_z(val == 0);
    }
}

pub struct Cpu {
    /// General purpose registers r0-r15
    /// r13 = SP, r14 = LR, r15 = PC
    pub regs: [u32; 16],

    /// Current Program Status Register
    pub cpsr: Psr,

    /// Saved PSRs for each mode (indexed by Mode::bank_index)
    pub spsr: [u32; 6],

    /// Banked registers: r13, r14 for each mode
    pub banked_r13: [u32; 6],
    pub banked_r14: [u32; 6],

    /// FIQ has extra banked registers r8-r12
    pub fiq_r8_r12: [u32; 5],
    pub usr_r8_r12: [u32; 5],

    /// Pipeline state
    pub pipeline: [u32; 2], // prefetch buffer
    pub pipeline_valid: bool,
}

impl Cpu {
    pub fn new() -> Self {
        let mut cpu = Self {
            regs: [0; 16],
            cpsr: Psr(Mode::Supervisor as u32 | (1 << 7) | (1 << 6)), // SVC mode, IRQ+FIQ disabled
            spsr: [0; 6],
            banked_r13: [0; 6],
            banked_r14: [0; 6],
            fiq_r8_r12: [0; 5],
            usr_r8_r12: [0; 5],
            pipeline: [0; 2],
            pipeline_valid: false,
        };
        cpu.banked_r13[Mode::Irq.bank_index()] = 0x03007FA0;
        cpu.banked_r13[Mode::Supervisor.bank_index()] = 0x03007FE0;
        cpu.banked_r13[Mode::User.bank_index()] = 0x03007F00;
        cpu.regs[13] = 0x03007FE0; // Start in SVC
        cpu
    }

    /// Skip BIOS initialization - set up state as if BIOS ran
    pub fn skip_bios(&mut self, _bus: &mut Bus) {
        self.regs[15] = 0x08000000; // ROM entry point
        self.cpsr = Psr(Mode::System as u32); // System mode, ARM state
        self.regs[13] = 0x03007F00;
        self.banked_r13[Mode::Irq.bank_index()] = 0x03007FA0;
        self.banked_r13[Mode::Supervisor.bank_index()] = 0x03007FE0;
        self.pipeline_valid = false;
    }

    /// Get current PC 
    pub fn pc(&self) -> u32 {
        self.regs[15]
    }


    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        // Check for pending interrupts
        if bus.ime && (bus.ie & bus.irf) != 0 && !self.cpsr.i() {
            self.handle_irq(bus);
        }

        if bus.halt {
            // CPU is halted
            if bus.ime && (bus.ie & bus.irf) != 0 {
                bus.halt = false;
            }
            return 1;
        }

        if !self.pipeline_valid {
            self.fill_pipeline(bus);
        }

        if self.cpsr.t() {
            self.execute_thumb(bus)
        } else {
            self.execute_arm(bus)
        }
    }

    fn fill_pipeline(&mut self, bus: &Bus) {
        if self.cpsr.t() {
            let pc = self.regs[15] & !1;
            self.pipeline[0] = bus.read16(pc) as u32;
            self.pipeline[1] = bus.read16(pc + 2) as u32;
            self.regs[15] = pc + 4;
        } else {
            let pc = self.regs[15] & !3;
            self.pipeline[0] = bus.read32(pc);
            self.pipeline[1] = bus.read32(pc + 4);
            self.regs[15] = pc + 8;
        }
        self.pipeline_valid = true;
    }

    /// Flush the pipeline 
    pub fn flush_pipeline(&mut self, bus: &Bus) {
        self.pipeline_valid = false;
        self.fill_pipeline(bus);
    }

    fn handle_irq(&mut self, _bus: &mut Bus) {
        let return_addr = self.regs[15] - if self.cpsr.t() { 2 } else { 4 };
        self.switch_mode(Mode::Irq);
        self.regs[14] = return_addr + 4;
        self.cpsr.set_i(true);
        self.cpsr.set_t(false);
        self.regs[15] = 0x18; // IRQ vector
        self.pipeline_valid = false;
    }

    pub fn switch_mode(&mut self, new_mode: Mode) {
        let old_mode = self.cpsr.mode();
        if old_mode == new_mode { return; }

        // Save current CPSR to new mode's SPSR
        self.spsr[new_mode.bank_index()] = self.cpsr.0;

        // Bank r13, r14
        self.banked_r13[old_mode.bank_index()] = self.regs[13];
        self.banked_r14[old_mode.bank_index()] = self.regs[14];
        self.regs[13] = self.banked_r13[new_mode.bank_index()];
        self.regs[14] = self.banked_r14[new_mode.bank_index()];

        // FIQ banking of r8-r12
        if old_mode == Mode::Fiq {
            self.fiq_r8_r12.copy_from_slice(&self.regs[8..13]);
            self.regs[8..13].copy_from_slice(&self.usr_r8_r12);
        } else if new_mode == Mode::Fiq {
            self.usr_r8_r12.copy_from_slice(&self.regs[8..13]);
            self.regs[8..13].copy_from_slice(&self.fiq_r8_r12);
        }

        self.cpsr.set_mode(new_mode);
    }

    /// Check ARM condition codes
    pub fn check_condition(&self, cond: u32) -> bool {
        match cond {
            0x0 => self.cpsr.z(),                              // EQ
            0x1 => !self.cpsr.z(),                             // NE
            0x2 => self.cpsr.c(),                              // CS/HS
            0x3 => !self.cpsr.c(),                             // CC/LO
            0x4 => self.cpsr.n(),                              // MI
            0x5 => !self.cpsr.n(),                             // PL
            0x6 => self.cpsr.v(),                              // VS
            0x7 => !self.cpsr.v(),                             // VC
            0x8 => self.cpsr.c() && !self.cpsr.z(),            // HI
            0x9 => !self.cpsr.c() || self.cpsr.z(),            // LS
            0xA => self.cpsr.n() == self.cpsr.v(),             // GE
            0xB => self.cpsr.n() != self.cpsr.v(),             // LT
            0xC => !self.cpsr.z() && (self.cpsr.n() == self.cpsr.v()), // GT
            0xD => self.cpsr.z() || (self.cpsr.n() != self.cpsr.v()), // LE
            0xE => true,                                       // AL (always)
            0xF => true,                                       // Reserved / unconditional
            _ => unreachable!(),
        }
    }

    /// ARM data processing
    pub fn barrel_shift(&self, shift_type: u32, operand: u32, amount: u32, carry_in: bool) -> (u32, bool) {
        if amount == 0 {
            return (operand, carry_in);
        }

        match shift_type {
            0 => { // LSL
                if amount >= 32 {
                    let carry = if amount == 32 { operand & 1 != 0 } else { false };
                    (0, carry)
                } else {
                    let carry = (operand >> (32 - amount)) & 1 != 0;
                    (operand << amount, carry)
                }
            }
            1 => { // LSR
                if amount >= 32 {
                    let carry = if amount == 32 { operand >> 31 != 0 } else { false };
                    (0, carry)
                } else {
                    let carry = (operand >> (amount - 1)) & 1 != 0;
                    (operand >> amount, carry)
                }
            }
            2 => { // ASR
                if amount >= 32 {
                    let sign = operand as i32 >> 31;
                    (sign as u32, sign != 0)
                } else {
                    let carry = (operand >> (amount - 1)) & 1 != 0;
                    ((operand as i32 >> amount) as u32, carry)
                }
            }
            3 => { // ROR
                let amount = amount & 31;
                if amount == 0 {
                    // RRX (rotate right extended)
                    let carry = operand & 1 != 0;
                    let result = (operand >> 1) | ((carry_in as u32) << 31);
                    (result, carry)
                } else {
                    let result = operand.rotate_right(amount);
                    let carry = result >> 31 != 0;
                    (result, carry)
                }
            }
            _ => unreachable!(),
        }
    }

    /// ALU operation helper (shared by ARM and Thumb)
    pub fn alu_op(&mut self, opcode: u32, rd: usize, op1: u32, op2: u32, set_flags: bool, carry: bool) -> Option<u32> {
        let result = match opcode {
            0x0 => op1 & op2,                     // AND
            0x1 => op1 ^ op2,                     // EOR
            0x2 => op1.wrapping_sub(op2),         // SUB
            0x3 => op2.wrapping_sub(op1),         // RSB
            0x4 => op1.wrapping_add(op2),         // ADD
            0x5 => op1.wrapping_add(op2).wrapping_add(carry as u32), // ADC
            0x6 => op1.wrapping_sub(op2).wrapping_sub(!carry as u32), // SBC
            0x7 => op2.wrapping_sub(op1).wrapping_sub(!carry as u32), // RSC
            0x8 => { // TST
                if set_flags { self.cpsr.set_nz(op1 & op2); }
                return None;
            }
            0x9 => { // TEQ
                if set_flags { self.cpsr.set_nz(op1 ^ op2); }
                return None;
            }
            0xA => { // CMP
                let result = op1.wrapping_sub(op2);
                if set_flags {
                    self.cpsr.set_nz(result);
                    self.cpsr.set_c(op1 >= op2);
                    self.cpsr.set_v(((op1 ^ op2) & (op1 ^ result)) >> 31 != 0);
                }
                return None;
            }
            0xB => { // CMN
                let result = op1.wrapping_add(op2);
                if set_flags {
                    self.cpsr.set_nz(result);
                    self.cpsr.set_c((op1 as u64 + op2 as u64) > 0xFFFFFFFF);
                    self.cpsr.set_v((!((op1 ^ op2)) & (op1 ^ result)) >> 31 != 0);
                }
                return None;
            }
            0xC => op1 | op2,                     // ORR
            0xD => op2,                           // MOV
            0xE => op1 & !op2,                    // BIC
            0xF => !op2,                          // MVN
            _ => unreachable!(),
        };

        if set_flags {
            self.cpsr.set_nz(result);
            match opcode {
                0x2 | 0x3 => { // SUB / RSB
                    let (a, b) = if opcode == 0x2 { (op1, op2) } else { (op2, op1) };
                    self.cpsr.set_c(a >= b);
                    self.cpsr.set_v(((a ^ b) & (a ^ result)) >> 31 != 0);
                }
                0x4 => { // ADD
                    self.cpsr.set_c((op1 as u64 + op2 as u64) > 0xFFFFFFFF);
                    self.cpsr.set_v((!(op1 ^ op2) & (op1 ^ result)) >> 31 != 0);
                }
                0x5 => { // ADC
                    let full = op1 as u64 + op2 as u64 + carry as u64;
                    self.cpsr.set_c(full > 0xFFFFFFFF);
                    self.cpsr.set_v((!(op1 ^ op2) & (op1 ^ result)) >> 31 != 0);
                }
                0x6 => { // SBC
                    let full = op1 as u64 - op2 as u64 - (!carry) as u64;
                    self.cpsr.set_c(full <= 0xFFFFFFFF && (op1 as i64 - op2 as i64 - (!carry) as i64) >= 0);
                    self.cpsr.set_v(((op1 ^ op2) & (op1 ^ result)) >> 31 != 0);
                }

                0x0 | 0x1 | 0xC | 0xD | 0xE | 0xF => {
                    
                }
                _ => {}
            }
        }

        Some(result)
    }
}