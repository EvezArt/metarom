//! letsplay -- MetaROM GB emulator letsplay runner
//! Runs a synthetic test ROM for N frames, captures ASCII output + state log.
//! Usage: cargo run --bin letsplay -- [frames]

use gb_core::{Cartridge, GbCore, LCD_WIDTH, LCD_HEIGHT};

fn synthetic_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x0100] = 0x00; // NOP
    rom[0x0101] = 0xC3; // JP nn
    rom[0x0102] = 0x50; // -> 0x0150
    rom[0x0103] = 0x01;
    let title = b"METAROM-TEST";
    for (i, &b) in title.iter().enumerate() { rom[0x0134+i] = b; }
    rom[0x0147] = 0x00; // ROM only
    rom[0x0148] = 0x00;
    rom[0x0149] = 0x00;
    rom[0x014D] = 0xE7;
    // Program: init LCD + spin loop
    let prog: &[u8] = &[
        0x3E, 0x00, 0xE0, 0x40, // LD A,0 / LDH (0x40),A -- LCD off
        0x01, 0x00, 0x80,       // LD BC, 0x8000
        0x3E, 0xAA, 0x02, 0x03, // LD A,0xAA / LD (BC),A / INC BC
        0x3E, 0x55, 0x02, 0x03, // LD A,0x55 / LD (BC),A / INC BC
        0x3E, 0xAA, 0x02, 0x03,
        0x3E, 0x55, 0x02, 0x03,
        0x01, 0x00, 0x98,       // LD BC, 0x9800
        0x3E, 0x00, 0x02,       // LD A,0 / LD (BC),A
        0x3E, 0x91, 0xE0, 0x40, // LCD on, BG on
        0x3E, 0x01, 0xE0, 0xFF, // IE = 1 (VBlank)
        0xFB,                   // EI
        0xC3, 0x86, 0x01,       // JP 0x0186 (spin)
    ];
    for (i, &b) in prog.iter().enumerate() { rom[0x0150+i] = b; }
    rom
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n_frames: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    println!("MetaROM LetsPlay Runner | target_frames={}", n_frames);
    println!("Resolution: {}x{} | CyclesPerFrame: {}", LCD_WIDTH, LCD_HEIGHT, gb_core::CYCLES_PER_FRAME);
    println!("");
    let cart = Cartridge::from_bytes(synthetic_rom()).expect("ROM invalid");
    println!("ROM: {} | MBC: {:?} | {}KB ROM | {}KB RAM",
             cart.title, cart.kind, cart.rom_size_kb, cart.ram_size_kb);
    println!("");
    let mut core = GbCore::new(cart);
    for frame in 0..n_frames {
        core.run_frame().expect("emulator crash");
        if frame < 3 || frame == n_frames-1 {
            println!("--- Frame {} ---", frame);
            println!("{}", core.state_summary());
            for (i, row) in core.frame_to_ascii().lines().take(8).enumerate() {
                println!("  [{}] {}", i, &row[..row.len().min(40)]);
            }
            if frame < n_frames-1 { println!("  ..."); }
            println!("");
        }
    }
    println!("=== LETSPLAY COMPLETE ===");
    println!("Frames: {} | T-cycles: {} | VBlanks: {} | LY: {} | Mode: {:?}",
             n_frames, core.clock.t_cycles, core.clock.frame_count(),
             core.bus.ppu.ly, core.bus.ppu.mode);
    println!("");
    println!("Final frame ({} rows):", LCD_HEIGHT/2);
    print!("{}", core.frame_to_ascii());
}