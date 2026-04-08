use super::arm7tdmi::Cpu;
use crate::memory::Bus;

impl Cpu {
    /// Execute one Thumb instruction, return cycles
    pub fn execute_thumb(&mut self, bus: &mut Bus) -> u32 {
        let instr = self.pipeline[0] as u16;
        self.pipeline[0] = self.pipeline[1];
        self.pipeline[1] = bus.read16(self.regs[15]) as u32;
        self.regs[15] = self.regs[15].wrapping_add(2);

        match instr >> 13 {
            0b000 => {
                if (instr >> 11) & 3 == 3 {
                    // Format 2: ADD/SUB
                    self.thumb_add_sub(instr, bus)
                } else {
                    // Format 1: Move shifted register
                    self.thumb_shift(instr)
                }
            }
            0b001 => {
                // Format 3: MOV/CMP/ADD/SUB immediate
                self.thumb_imm_op(instr)
            }
            0b010 => {
                if instr & (1 << 12) != 0 {
                    // Format 7/8: Load/store register offset
                    self.thumb_load_store_reg(instr, bus)
                } else if instr & (1 << 11) != 0 {
                    // Format 6: PC-relative load
                    self.thumb_pc_relative_load(instr, bus)
                } else if (instr >> 10) & 3 == 0 {
                    // Format 4: ALU operations
                    self.thumb_alu(instr, bus)
                } else {
                    // Format 5: Hi register operations / BX
                    self.thumb_hi_reg_bx(instr, bus)
                }
            }
            0b011 => {
                // Format 9: Load/store with immediate offset
                self.thumb_load_store_imm(instr, bus)
            }
            0b100 => {
                if instr & (1 << 12) != 0 {
                    // Format 11: SP-relative load/store
                    self.thumb_sp_relative_load_store(instr, bus)
                } else {
                    // Format 10: Load/store halfword
                    self.thumb_load_store_halfword(instr, bus)
                }
            }
            0b101 => {
                if instr & (1 << 12) != 0 {
                    // Format 13: Add offset to SP / Format 14: push/pop
                    if instr & (1 << 10) != 0 {
                        self.thumb_push_pop(instr, bus)
                    } else {
                        self.thumb_add_sp(instr)
                    }
                } else {
                    // Format 12: Load address
                    self.thumb_load_address(instr)
                }
            }
            0b110 => {
                if instr & (1 << 12) != 0 {
                    if (instr >> 8) & 0xF == 0xF {
                        // Format 17: SWI
                        self.thumb_swi(bus)
                    } else {
                        // Format 16: Conditional branch
                        self.thumb_cond_branch(instr, bus)
                    }
                } else {
                    // Format 15: Multiple load/store
                    self.thumb_multiple_load_store(instr, bus)
                }
            }
            0b111 => {
                if instr & (1 << 12) != 0 {
                    // Format 19: Long branch with link (second half)
                    self.thumb_long_branch(instr, bus)
                } else {
                    // Format 18: Unconditional branch
                    self.thumb_branch(instr, bus)
                }
            }
            _ => 1,
        }
    }

    // Format 1: Shift by immediate
    fn thumb_shift(&mut self, instr: u16) -> u32 {
        let op = (instr >> 11) & 3;
        let offset = ((instr >> 6) & 0x1F) as u32;
        let rs = ((instr >> 3) & 7) as usize;
        let rd = (instr & 7) as usize;

        let val = self.regs[rs];
        let (result, carry) = if offset == 0 {
            match op {
                0 => (val, self.cpsr.c()),
                1 => (0, val >> 31 != 0),
                2 => { let s = val as i32 >> 31; (s as u32, s != 0) }
                _ => unreachable!(),
            }
        } else {
            self.barrel_shift(op as u32, val, offset, self.cpsr.c())
        };

        self.regs[rd] = result;
        self.cpsr.set_nz(result);
        self.cpsr.set_c(carry);
        1
    }

    // Format 2: ADD/SUB
    fn thumb_add_sub(&mut self, instr: u16, _bus: &Bus) -> u32 {
        let is_sub = instr & (1 << 9) != 0;
        let is_imm = instr & (1 << 10) != 0;
        let rs = ((instr >> 3) & 7) as usize;
        let rd = (instr & 7) as usize;

        let op1 = self.regs[rs];
        let op2 = if is_imm {
            ((instr >> 6) & 7) as u32
        } else {
            self.regs[((instr >> 6) & 7) as usize]
        };

        let carry = self.cpsr.c();
        let opcode = if is_sub { 0x2 } else { 0x4 }; // SUB or ADD
        if let Some(result) = self.alu_op(opcode, rd, op1, op2, true, carry) {
            self.regs[rd] = result;
        }
        1
    }

    // Format 3: Immediate operations
    fn thumb_imm_op(&mut self, instr: u16) -> u32 {
        let op = (instr >> 11) & 3;
        let rd = ((instr >> 8) & 7) as usize;
        let imm = (instr & 0xFF) as u32;

        let op1 = self.regs[rd];
        let carry = self.cpsr.c();

        let opcode = match op {
            0 => 0xD, // MOV
            1 => 0xA, // CMP
            2 => 0x4, // ADD
            3 => 0x2, // SUB
            _ => unreachable!(),
        };

        if let Some(result) = self.alu_op(opcode, rd, op1, imm, true, carry) {
            self.regs[rd] = result;
        }
        1
    }

    // Format 4: ALU operations
    fn thumb_alu(&mut self, instr: u16, bus: &Bus) -> u32 {
        let op = (instr >> 6) & 0xF;
        let rs = ((instr >> 3) & 7) as usize;
        let rd = (instr & 7) as usize;

        let a = self.regs[rd];
        let b = self.regs[rs];
        let carry = self.cpsr.c();

        match op {
            0x0 => { self.regs[rd] = a & b; self.cpsr.set_nz(self.regs[rd]); }  // AND
            0x1 => { self.regs[rd] = a ^ b; self.cpsr.set_nz(self.regs[rd]); }  // EOR
            0x2 => { // LSL
                let (result, c) = self.barrel_shift(0, a, b & 0xFF, carry);
                self.regs[rd] = result;
                self.cpsr.set_nz(result);
                if b & 0xFF != 0 { self.cpsr.set_c(c); }
            }
            0x3 => { // LSR
                let (result, c) = self.barrel_shift(1, a, b & 0xFF, carry);
                self.regs[rd] = result;
                self.cpsr.set_nz(result);
                if b & 0xFF != 0 { self.cpsr.set_c(c); }
            }
            0x4 => { // ASR
                let (result, c) = self.barrel_shift(2, a, b & 0xFF, carry);
                self.regs[rd] = result;
                self.cpsr.set_nz(result);
                if b & 0xFF != 0 { self.cpsr.set_c(c); }
            }
            0x5 => { // ADC
                if let Some(r) = self.alu_op(0x5, rd, a, b, true, carry) { self.regs[rd] = r; }
            }
            0x6 => { // SBC
                if let Some(r) = self.alu_op(0x6, rd, a, b, true, carry) { self.regs[rd] = r; }
            }
            0x7 => { // ROR
                let (result, c) = self.barrel_shift(3, a, b & 0xFF, carry);
                self.regs[rd] = result;
                self.cpsr.set_nz(result);
                if b & 0xFF != 0 { self.cpsr.set_c(c); }
            }
            0x8 => { // TST
                self.cpsr.set_nz(a & b);
            }
            0x9 => { // NEG
                let result = 0u32.wrapping_sub(b);
                self.regs[rd] = result;
                self.cpsr.set_nz(result);
                self.cpsr.set_c(b == 0);
                self.cpsr.set_v((b & result) >> 31 != 0);
            }
            0xA => { // CMP
                self.alu_op(0xA, rd, a, b, true, carry);
            }
            0xB => { // CMN
                self.alu_op(0xB, rd, a, b, true, carry);
            }
            0xC => { self.regs[rd] = a | b; self.cpsr.set_nz(self.regs[rd]); }  // ORR
            0xD => { // MUL
                self.regs[rd] = a.wrapping_mul(b);
                self.cpsr.set_nz(self.regs[rd]);
            }
            0xE => { self.regs[rd] = a & !b; self.cpsr.set_nz(self.regs[rd]); } // BIC
            0xF => { self.regs[rd] = !b; self.cpsr.set_nz(self.regs[rd]); }     // MVN
            _ => unreachable!(),
        }

        let _ = bus;
        1
    }

    // Format 5: Hi register ops / BX
    fn thumb_hi_reg_bx(&mut self, instr: u16, bus: &Bus) -> u32 {
        let op = (instr >> 8) & 3;
        let h1 = (instr >> 7) & 1;
        let h2 = (instr >> 6) & 1;
        let rs = ((h2 << 3) | ((instr >> 3) & 7)) as usize;
        let rd = ((h1 << 3) | (instr & 7)) as usize;

        match op {
            0 => { // ADD
                self.regs[rd] = self.regs[rd].wrapping_add(self.regs[rs]);
                if rd == 15 { self.flush_pipeline(bus); return 3; }
            }
            1 => { // CMP
                let carry = self.cpsr.c();
                self.alu_op(0xA, rd, self.regs[rd], self.regs[rs], true, carry);
            }
            2 => { // MOV
                self.regs[rd] = self.regs[rs];
                if rd == 15 { self.flush_pipeline(bus); return 3; }
            }
            3 => { // BX
                let addr = self.regs[rs];
                if addr & 1 != 0 {
                    self.cpsr.set_t(true);
                    self.regs[15] = addr & !1;
                } else {
                    self.cpsr.set_t(false);
                    self.regs[15] = addr & !3;
                }
                self.flush_pipeline(bus);
                return 3;
            }
            _ => unreachable!(),
        }
        1
    }

    // Format 6: PC-relative load
    fn thumb_pc_relative_load(&mut self, instr: u16, bus: &Bus) -> u32 {
        let rd = ((instr >> 8) & 7) as usize;
        let offset = (instr & 0xFF) as u32 * 4;
        let addr = (self.regs[15] & !2).wrapping_add(offset);
        self.regs[rd] = bus.read32(addr);
        3
    }

    // Format 7/8: Load/store register offset
    fn thumb_load_store_reg(&mut self, instr: u16, bus: &mut Bus) -> u32 {
        let op = (instr >> 10) & 3;
        let ro = ((instr >> 6) & 7) as usize;
        let rb = ((instr >> 3) & 7) as usize;
        let rd = (instr & 7) as usize;

        let addr = self.regs[rb].wrapping_add(self.regs[ro]);

        if instr & (1 << 9) != 0 {
            // Format 8: sign-extended / halfword
            match op {
                0 => bus.write16(addr, self.regs[rd] as u16),  // STRH
                1 => self.regs[rd] = bus.read8(addr) as i8 as i32 as u32, // LDSB
                2 => self.regs[rd] = bus.read16(addr) as u32,  // LDRH
                3 => self.regs[rd] = bus.read16(addr) as i16 as i32 as u32, // LDSH
                _ => unreachable!(),
            }
        } else {
            // Format 7: word/byte
            match op {
                0 => bus.write32(addr, self.regs[rd]),          // STR
                1 => self.regs[rd] = bus.read32(addr),          // LDR
                2 => bus.write8(addr, self.regs[rd] as u8),     // STRB
                3 => self.regs[rd] = bus.read8(addr) as u32,    // LDRB
                _ => unreachable!(),
            }
        }

        2
    }

    // Format 9: Load/store with immediate offset
    fn thumb_load_store_imm(&mut self, instr: u16, bus: &mut Bus) -> u32 {
        let byte = instr & (1 << 12) != 0;
        let load = instr & (1 << 11) != 0;
        let offset = ((instr >> 6) & 0x1F) as u32;
        let rb = ((instr >> 3) & 7) as usize;
        let rd = (instr & 7) as usize;

        let addr = if byte {
            self.regs[rb].wrapping_add(offset)
        } else {
            self.regs[rb].wrapping_add(offset * 4)
        };

        if load {
            self.regs[rd] = if byte {
                bus.read8(addr) as u32
            } else {
                bus.read32(addr)
            };
        } else {
            if byte {
                bus.write8(addr, self.regs[rd] as u8);
            } else {
                bus.write32(addr, self.regs[rd]);
            }
        }

        2
    }

    // Format 10: Load/store halfword
    fn thumb_load_store_halfword(&mut self, instr: u16, bus: &mut Bus) -> u32 {
        let load = instr & (1 << 11) != 0;
        let offset = ((instr >> 6) & 0x1F) as u32 * 2;
        let rb = ((instr >> 3) & 7) as usize;
        let rd = (instr & 7) as usize;

        let addr = self.regs[rb].wrapping_add(offset);

        if load {
            self.regs[rd] = bus.read16(addr) as u32;
        } else {
            bus.write16(addr, self.regs[rd] as u16);
        }

        2
    }

    // Format 11: SP-relative load/store
    fn thumb_sp_relative_load_store(&mut self, instr: u16, bus: &mut Bus) -> u32 {
        let load = instr & (1 << 11) != 0;
        let rd = ((instr >> 8) & 7) as usize;
        let offset = (instr & 0xFF) as u32 * 4;
        let addr = self.regs[13].wrapping_add(offset);

        if load {
            self.regs[rd] = bus.read32(addr);
        } else {
            bus.write32(addr, self.regs[rd]);
        }

        2
    }

    // Format 12: Load address
    fn thumb_load_address(&mut self, instr: u16) -> u32 {
        let sp = instr & (1 << 11) != 0;
        let rd = ((instr >> 8) & 7) as usize;
        let offset = (instr & 0xFF) as u32 * 4;

        self.regs[rd] = if sp {
            self.regs[13].wrapping_add(offset)
        } else {
            (self.regs[15] & !2).wrapping_add(offset)
        };

        1
    }

    // Format 13: Add offset to SP
    fn thumb_add_sp(&mut self, instr: u16) -> u32 {
        let offset = (instr & 0x7F) as u32 * 4;
        if instr & (1 << 7) != 0 {
            self.regs[13] = self.regs[13].wrapping_sub(offset);
        } else {
            self.regs[13] = self.regs[13].wrapping_add(offset);
        }
        1
    }

    // Format 14: Push/Pop
    fn thumb_push_pop(&mut self, instr: u16, bus: &mut Bus) -> u32 {
        let load = instr & (1 << 11) != 0;
        let pc_lr = instr & (1 << 8) != 0;
        let reg_list = instr & 0xFF;

        let mut cycles = 0u32;

        if load {
            // POP
            let mut addr = self.regs[13];
            for i in 0..8u32 {
                if reg_list & (1 << i) != 0 {
                    self.regs[i as usize] = bus.read32(addr);
                    addr = addr.wrapping_add(4);
                    cycles += 1;
                }
            }
            if pc_lr {
                self.regs[15] = bus.read32(addr) & !1;
                addr = addr.wrapping_add(4);
                self.flush_pipeline(bus);
                cycles += 3;
            }
            self.regs[13] = addr;
        } else {
            // PUSH
            let count = reg_list.count_ones() + pc_lr as u32;
            let mut addr = self.regs[13].wrapping_sub(count * 4);
            self.regs[13] = addr;

            for i in 0..8u32 {
                if reg_list & (1 << i) != 0 {
                    bus.write32(addr, self.regs[i as usize]);
                    addr = addr.wrapping_add(4);
                    cycles += 1;
                }
            }
            if pc_lr {
                bus.write32(addr, self.regs[14]);
                cycles += 1;
            }
        }

        cycles + 1
    }

    // Format 15: Multiple load/store
    fn thumb_multiple_load_store(&mut self, instr: u16, bus: &mut Bus) -> u32 {
        let load = instr & (1 << 11) != 0;
        let rb = ((instr >> 8) & 7) as usize;
        let reg_list = instr & 0xFF;

        let mut addr = self.regs[rb];
        let mut cycles = 0u32;

        for i in 0..8u32 {
            if reg_list & (1 << i) == 0 { continue; }

            if load {
                self.regs[i as usize] = bus.read32(addr);
            } else {
                bus.write32(addr, self.regs[i as usize]);
            }
            addr = addr.wrapping_add(4);
            cycles += 1;
        }

        // Writeback (always for STMIA, for LDMIA only if rb not in list)
        if !load || (reg_list & (1 << rb as u16)) == 0 {
            self.regs[rb] = addr;
        }

        cycles + 1
    }

    // Format 16: Conditional branch
    fn thumb_cond_branch(&mut self, instr: u16, bus: &Bus) -> u32 {
        let cond = ((instr >> 8) & 0xF) as u32;
        if !self.check_condition(cond) {
            return 1;
        }

        let offset = ((instr & 0xFF) as i8 as i32) << 1;
        self.regs[15] = (self.regs[15] as i32).wrapping_add(offset) as u32;
        self.flush_pipeline(bus);
        3
    }

    // Format 17: SWI
    fn thumb_swi(&mut self, bus: &Bus) -> u32 {
        use super::arm7tdmi::Mode;
        self.switch_mode(Mode::Supervisor);
        self.regs[14] = self.regs[15].wrapping_sub(2);
        self.cpsr.set_i(true);
        self.cpsr.set_t(false);
        self.regs[15] = 0x08;
        self.flush_pipeline(bus);
        3
    }

    // Format 18: Unconditional branch
    fn thumb_branch(&mut self, instr: u16, bus: &Bus) -> u32 {
        let offset = (((instr & 0x7FF) as i32) << 21) >> 20;
        self.regs[15] = (self.regs[15] as i32).wrapping_add(offset) as u32;
        self.flush_pipeline(bus);
        3
    }

    // Format 19: Long branch with link
    fn thumb_long_branch(&mut self, instr: u16, bus: &Bus) -> u32 {
        let offset = (instr & 0x7FF) as u32;
        let is_second = instr & (1 << 11) != 0;

        if !is_second {
            // First instruction: set up high bits of offset in LR
            let offset = (((instr & 0x7FF) as i32) << 21) >> 9;
            self.regs[14] = (self.regs[15] as i32).wrapping_add(offset) as u32;
            1
        } else {
            // Second instruction: combine and branch
            let target = self.regs[14].wrapping_add(offset << 1);
            self.regs[14] = (self.regs[15].wrapping_sub(2)) | 1;
            self.regs[15] = target & !1;
            self.flush_pipeline(bus);
            3
        }
    }
}
