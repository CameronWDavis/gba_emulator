mod cpu;
mod memory;
mod ppu;
mod io;

use cpu::Cpu;
use memory::Bus;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: gba-emu <rom.gba> [bios.bin]");
        std::process::exit(1);
    }

    let rom = std::fs::read(&args[1]).expect("Failed to read ROM file");
    let bios = if args.len() > 2 {
        Some(std::fs::read(&args[2]).expect("Failed to read BIOS file"))
    } else {
        None
    };

    let mut bus = Bus::new(rom, bios);
    let mut cpu = Cpu::new();

    if bus.bios.is_none() {
        cpu.skip_bios(&mut bus);
    }

    println!("GBA Emulator started!");
    println!("ROM size: {} bytes", bus.rom.len());

    const CYCLES_PER_FRAME: u32 = 280896;

    loop {
        let mut frame_cycles: u32 = 0;

        while frame_cycles < CYCLES_PER_FRAME {
            let cycles = cpu.step(&mut bus);
            bus.tick(cycles);
            frame_cycles += cycles;
        }

    
        break;
    }

    println!("Emulation complete (1 frame test).");
}