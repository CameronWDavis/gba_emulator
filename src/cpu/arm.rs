
use super::arm7tdmi::{Cpu, Mode};
use crate::memory::Bus;

impl Cpu {
    /// Execute one ARM instruction, return cycles
    pub fn execute_arm(&mut self, bus: &mut Bus) -> u32 {
        // Fetch from pipeline
        let instr = self.pipeline[0];
        self.pipeline[0] = self.pipeline[1];
        self.pipeline[1] = bus.read32(self.regs[15]);
        self.regs[15] = self.regs[15].wrapping_add(4);

        // Check condition
        let cond = instr >> 28;
        if !self.check_condition(cond) {
            return 1; // 1S cycle for skipped instruction
        }

        // Decode by bits [27:20] and [7:4]
        let bits_27_20 = (instr >> 20) & 0xFF;
        let bits_7_4 = (instr >> 4) & 0xF;

        match bits_27_20 >> 5 {
            0b000 => {
                if bits_27_20 == 0b00010010 && bits_7_4 == 0b0001 {
                    // BX
                    self.arm_bx(instr, bus)
                } else if (bits_7_4 & 0b1001) == 0b1001 && (bits_27_20 & 0b11100) == 0 {
                    // Multiply / Multiply Long
                    if bits_27_20 & 0b1000 != 0 {
                        self.arm_multiply_long(instr)
                    } else {
                        self.arm_multiply(instr)
                    }
                } else if (bits_7_4 & 0b1001) == 0b1001 && (bits_27_20 & 0b10) == 0 {
                    // Single data swap
                    if bits_27_20 & 0b10000 != 0 {
                        self.arm_swap(instr, bus)
                    } else {
                        self.arm_halfword_transfer(instr, bus)
                    }
                } else if (bits_7_4 & 0b1001) == 0b1001 {
                    self.arm_halfword_transfer(instr, bus)
                } else if bits_27_20 & 0b11011 == 0b10000 && bits_7_4 == 0 {
                    // MRS / MSR (register)
                    self.arm_psr_transfer(instr)
                } else {
                    // Data processing (register shift or immediate shift)
                    self.arm_data_processing(instr, bus)
                }
            }
            0b001 => {
                if bits_27_20 & 0b11011 == 0b10010 {
                    // MSR immediate
                    self.arm_psr_transfer(instr)
                } else {
                    // Data processing immediate
                    self.arm_data_processing(instr, bus)
                }
            }
            0b010 | 0b011 => {
                if bits_27_20 >> 5 == 0b011 && bits_7_4 & 1 != 0 {
                    // Undefined
                    1
                } else {
                    // Single data transfer (LDR/STR)
                    self.arm_single_transfer(instr, bus)
                }
            }
            0b100 => {
                // Block data transfer (LDM/STM)
                self.arm_block_transfer(instr, bus)
            }
            0b101 => {
                // Branch / Branch with Link
                self.arm_branch(instr, bus)
            }
            0b111 => {
                if instr >> 24 & 0xF == 0xF {
                    // SWI
                    self.arm_swi(instr, bus)
                } else {
                    1 // Coprocessor / undefined
                }
            }
            _ => 1,
        }
    }

    fn arm_data_processing(&mut self, instr: u32, bus: &Bus) -> u32 {
        let opcode = (instr >> 21) & 0xF;
        let set_flags = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xF) as usize;
        let rd = ((instr >> 12) & 0xF) as usize;

        let op1 = if rn == 15 { self.regs[15] } else { self.regs[rn] };

        let (op2, carry) = if instr & (1 << 25) != 0 {
            // Immediate operand
            let imm = instr & 0xFF;
            let rotate = (instr >> 8) & 0xF;
            if rotate == 0 {
                (imm, self.cpsr.c())
            } else {
                let result = imm.rotate_right(rotate * 2);
                let carry = result >> 31 != 0;
                (result, carry)
            }
        } else {
            // Register operand with shift
            let rm = (instr & 0xF) as usize;
            let rm_val = if rm == 15 { self.regs[15] } else { self.regs[rm] };
            let shift_type = (instr >> 5) & 3;

            if instr & (1 << 4) != 0 {
                // Register-specified shift
                let rs = ((instr >> 8) & 0xF) as usize;
                let shift_amount = self.regs[rs] & 0xFF;
                self.barrel_shift(shift_type, rm_val, shift_amount, self.cpsr.c())
            } else {
                // Immediate shift
                let shift_amount = (instr >> 7) & 0x1F;
                if shift_amount == 0 {
                    match shift_type {
                        0 => (rm_val, self.cpsr.c()), // LSL #0 = no shift
                        1 => (0, rm_val >> 31 != 0),  // LSR #0 = LSR #32
                        2 => {
                            let sign = rm_val as i32 >> 31;
                            (sign as u32, sign != 0)
                        }
                        3 => {
                            // RRX
                            let carry = rm_val & 1 != 0;
                            let result = (rm_val >> 1) | ((self.cpsr.c() as u32) << 31);
                            (result, carry)
                        }
                        _ => unreachable!(),
                    }
                } else {
                    self.barrel_shift(shift_type, rm_val, shift_amount, self.cpsr.c())
                }
            }
        };

        // For logical ops, the carry from the shifter should be used
        let old_carry = self.cpsr.c();
        if set_flags && matches!(opcode, 0x0 | 0x1 | 0x8 | 0x9 | 0xC | 0xD | 0xE | 0xF) {
            self.cpsr.set_c(carry);
        }

        if let Some(result) = self.alu_op(opcode, rd, op1, op2, set_flags, old_carry) {
            self.regs[rd] = result;
            if rd == 15 {
                if set_flags {
                    // Restore CPSR from SPSR
                    let bank = self.cpsr.mode().bank_index();
                    self.cpsr.0 = self.spsr[bank];
                }
                self.flush_pipeline(bus);
                return 3;
            }
        }

        1
    }

    fn arm_branch(&mut self, instr: u32, bus: &Bus) -> u32 {
        let link = instr & (1 << 24) != 0;
        let offset = ((instr & 0x00FFFFFF) as i32) << 8 >> 6; // Sign-extend and shift left 2

        if link {
            self.regs[14] = self.regs[15].wrapping_sub(4);
        }

        self.regs[15] = (self.regs[15] as i32).wrapping_add(offset) as u32;
        self.flush_pipeline(bus);
        3
    }

    fn arm_bx(&mut self, instr: u32, bus: &Bus) -> u32 {
        let rm = (instr & 0xF) as usize;
        let addr = self.regs[rm];

        if addr & 1 != 0 {
            self.cpsr.set_t(true);
            self.regs[15] = addr & !1;
        } else {
            self.cpsr.set_t(false);
            self.regs[15] = addr & !3;
        }

        self.flush_pipeline(bus);
        3
    }

    fn arm_single_transfer(&mut self, instr: u32, bus: &mut Bus) -> u32 {
        let pre = instr & (1 << 24) != 0;
        let up = instr & (1 << 23) != 0;
        let byte = instr & (1 << 22) != 0;
        let writeback = instr & (1 << 21) != 0;
        let load = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xF) as usize;
        let rd = ((instr >> 12) & 0xF) as usize;

        let base = self.regs[rn];

        let offset = if instr & (1 << 25) != 0 {
            // Register offset with shift
            let rm = (instr & 0xF) as usize;
            let shift_type = (instr >> 5) & 3;
            let shift_amount = (instr >> 7) & 0x1F;
            let (result, _) = self.barrel_shift(shift_type, self.regs[rm], shift_amount, self.cpsr.c());
            result
        } else {
            // Immediate offset
            instr & 0xFFF
        };

        let addr = if pre {
            if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) }
        } else {
            base
        };

        if load {
            let val = if byte {
                bus.read8(addr) as u32
            } else {
                let val = bus.read32(addr);
                // Misaligned reads rotate
                let rotate = (addr & 3) * 8;
                val.rotate_right(rotate)
            };
            self.regs[rd] = val;
            if rd == 15 {
                self.flush_pipeline(bus);
            }
        } else {
            let val = if rd == 15 { self.regs[15] } else { self.regs[rd] };
            if byte {
                bus.write8(addr, val as u8);
            } else {
                bus.write32(addr, val);
            }
        }

        // Post-index or writeback
        if !pre {
            let final_addr = if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
            self.regs[rn] = final_addr;
        } else if writeback {
            self.regs[rn] = addr;
        }

        if load { 3 } else { 2 }
    }

    fn arm_halfword_transfer(&mut self, instr: u32, bus: &mut Bus) -> u32 {
        let pre = instr & (1 << 24) != 0;
        let up = instr & (1 << 23) != 0;
        let imm_offset = instr & (1 << 22) != 0;
        let writeback = instr & (1 << 21) != 0;
        let load = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xF) as usize;
        let rd = ((instr >> 12) & 0xF) as usize;
        let op = (instr >> 5) & 3;

        let base = self.regs[rn];
        let offset = if imm_offset {
            ((instr >> 4) & 0xF0) | (instr & 0xF)
        } else {
            self.regs[(instr & 0xF) as usize]
        };

        let addr = if pre {
            if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) }
        } else {
            base
        };

        match op {
            1 => { // LDRH / STRH (unsigned halfword)
                if load {
                    self.regs[rd] = bus.read16(addr) as u32;
                } else {
                    bus.write16(addr, self.regs[rd] as u16);
                }
            }
            2 => { // LDRSB (signed byte)
                self.regs[rd] = bus.read8(addr) as i8 as i32 as u32;
            }
            3 => { // LDRSH (signed halfword)
                self.regs[rd] = bus.read16(addr) as i16 as i32 as u32;
            }
            _ => {}
        }

        if !pre {
            let final_addr = if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
            self.regs[rn] = final_addr;
        } else if writeback {
            self.regs[rn] = addr;
        }

        if load { 3 } else { 2 }
    }

    fn arm_block_transfer(&mut self, instr: u32, bus: &mut Bus) -> u32 {
        let pre = instr & (1 << 24) != 0;
        let up = instr & (1 << 23) != 0;
        let psr_force = instr & (1 << 22) != 0;
        let writeback = instr & (1 << 21) != 0;
        let load = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xF) as usize;
        let reg_list = instr & 0xFFFF;

        let _ = psr_force; // TODO: handle user bank transfer

        let count = reg_list.count_ones();
        let base = self.regs[rn];

        let (mut addr, end_addr) = if up {
            (base, base.wrapping_add(count * 4))
        } else {
            (base.wrapping_sub(count * 4), base)
        };

        if !up {
            addr = base.wrapping_sub(count * 4);
        }

        let mut current = if up && pre {
            addr + 4
        } else if up {
            addr
        } else if pre {
            addr
        } else {
            addr + 4
        };

        let mut cycles = 0u32;

        for i in 0..16u32 {
            if reg_list & (1 << i) == 0 { continue; }

            if load {
                self.regs[i as usize] = bus.read32(current);
            } else {
                let val = if i == 15 { self.regs[15] } else { self.regs[i as usize] };
                bus.write32(current, val);
            }
            current = current.wrapping_add(4);
            cycles += 1;
        }

        if writeback {
            self.regs[rn] = if up { end_addr } else { base.wrapping_sub(count * 4) };
        }

        if load && reg_list & (1 << 15) != 0 {
            self.flush_pipeline(bus);
            return cycles + 3;
        }

        cycles + 2
    }

    fn arm_multiply(&mut self, instr: u32) -> u32 {
        let accumulate = instr & (1 << 21) != 0;
        let set_flags = instr & (1 << 20) != 0;
        let rd = ((instr >> 16) & 0xF) as usize;
        let rn = ((instr >> 12) & 0xF) as usize;
        let rs = ((instr >> 8) & 0xF) as usize;
        let rm = (instr & 0xF) as usize;

        let mut result = self.regs[rm].wrapping_mul(self.regs[rs]);
        if accumulate {
            result = result.wrapping_add(self.regs[rn]);
        }

        self.regs[rd] = result;
        if set_flags {
            self.cpsr.set_nz(result);
        }

        if accumulate { 3 } else { 2 }
    }

    fn arm_multiply_long(&mut self, instr: u32) -> u32 {
        let signed = instr & (1 << 22) != 0;
        let accumulate = instr & (1 << 21) != 0;
        let set_flags = instr & (1 << 20) != 0;
        let rd_hi = ((instr >> 16) & 0xF) as usize;
        let rd_lo = ((instr >> 12) & 0xF) as usize;
        let rs = ((instr >> 8) & 0xF) as usize;
        let rm = (instr & 0xF) as usize;

        let mut result: u64 = if signed {
            (self.regs[rm] as i32 as i64 * self.regs[rs] as i32 as i64) as u64
        } else {
            self.regs[rm] as u64 * self.regs[rs] as u64
        };

        if accumulate {
            result = result.wrapping_add(((self.regs[rd_hi] as u64) << 32) | self.regs[rd_lo] as u64);
        }

        self.regs[rd_lo] = result as u32;
        self.regs[rd_hi] = (result >> 32) as u32;

        if set_flags {
            self.cpsr.set_n((result >> 63) != 0);
            self.cpsr.set_z(result == 0);
        }

        if accumulate { 4 } else { 3 }
    }

    fn arm_swap(&mut self, instr: u32, bus: &mut Bus) -> u32 {
        let byte = instr & (1 << 22) != 0;
        let rn = ((instr >> 16) & 0xF) as usize;
        let rd = ((instr >> 12) & 0xF) as usize;
        let rm = (instr & 0xF) as usize;

        let addr = self.regs[rn];
        if byte {
            let tmp = bus.read8(addr);
            bus.write8(addr, self.regs[rm] as u8);
            self.regs[rd] = tmp as u32;
        } else {
            let tmp = bus.read32(addr);
            bus.write32(addr, self.regs[rm]);
            self.regs[rd] = tmp;
        }

        4
    }

    fn arm_psr_transfer(&mut self, instr: u32) -> u32 {
        let use_spsr = instr & (1 << 22) != 0;
        let is_msr = instr & (1 << 21) != 0;

        if is_msr {
            // MSR - write to PSR
            let val = if instr & (1 << 25) != 0 {
                let imm = instr & 0xFF;
                let rotate = (instr >> 8) & 0xF;
                imm.rotate_right(rotate * 2)
            } else {
                self.regs[(instr & 0xF) as usize]
            };

            let mask = {
                let mut m = 0u32;
                if instr & (1 << 19) != 0 { m |= 0xFF000000; } // flags
                if instr & (1 << 16) != 0 { m |= 0x000000FF; } // control
                m
            };

            if use_spsr {
                let bank = self.cpsr.mode().bank_index();
                self.spsr[bank] = (self.spsr[bank] & !mask) | (val & mask);
            } else {
                let new_cpsr = (self.cpsr.0 & !mask) | (val & mask);
                if mask & 0xFF != 0 {
                    let new_mode = Mode::from_bits((new_cpsr & 0x1F) as u8);
                    if self.cpsr.mode() != new_mode {
                        self.switch_mode(new_mode);
                    }
                }
                self.cpsr.0 = new_cpsr;
            }
        } else {
            // MRS - read from PSR
            let rd = ((instr >> 12) & 0xF) as usize;
            self.regs[rd] = if use_spsr {
                self.spsr[self.cpsr.mode().bank_index()]
            } else {
                self.cpsr.0
            };
        }

        1
    }

    fn arm_swi(&mut self, _instr: u32, bus: &Bus) -> u32 {
        self.switch_mode(Mode::Supervisor);
        self.regs[14] = self.regs[15].wrapping_sub(4);
        self.cpsr.set_i(true);
        self.regs[15] = 0x08; // SWI vector
        self.flush_pipeline(bus);
        3
    }
}
