mod cpu;
mod memory;
mod ppu;
mod io;

use cpu::Cpu;
use memory::Bus;
use minifb::{Key, Scale, Window, WindowOptions};

const SCREEN_WIDTH: usize = 240;
const SCREEN_HEIGHT: usize = 160;
const CYCLES_PER_FRAME: u32 = 280896;

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

    let mut window = Window::new(
        "GBA Emulator",
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        WindowOptions {
            scale: Scale::X4,
            ..WindowOptions::default()
        },
    )
    .expect("Failed to create window");

    // Limit to ~60 fps
    window.set_target_fps(60);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Run one frame worth of CPU cycles
        let mut frame_cycles: u32 = 0;
        while frame_cycles < CYCLES_PER_FRAME {
            let cycles = cpu.step(&mut bus);
            bus.tick(cycles);
            frame_cycles += cycles;
        }

        // Update keyinput register (active-low: 0 = pressed, 1 = released)
        bus.keyinput = 0x03FF; // all released
        let key_map: &[(Key, u16)] = &[
            (Key::Z,         io::keys::A),
            (Key::X,         io::keys::B),
            (Key::Backspace, io::keys::SELECT),
            (Key::Enter,     io::keys::START),
            (Key::Right,     io::keys::RIGHT),
            (Key::Left,      io::keys::LEFT),
            (Key::Up,        io::keys::UP),
            (Key::Down,      io::keys::DOWN),
            (Key::S,         io::keys::R),
            (Key::A,         io::keys::L),
        ];
        for (key, bit) in key_map {
            if window.is_key_down(*key) {
                bus.keyinput &= !bit; // clear bit = pressed
            }
        }

        // Blit the PPU framebuffer to the window
        window
            .update_with_buffer(bus.ppu.get_framebuffer(), SCREEN_WIDTH, SCREEN_HEIGHT)
            .expect("Failed to update window");
    }
}
