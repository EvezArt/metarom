//! letsplay_train -- MetaROM Phase 4 Training Runner
//! Plays a ROM (or synthetic test ROM) for N frames and dumps a .mrom.train.json.
//!
//! Usage:
//!   cargo run --bin letsplay_train -- [frames] [output_path]
//!
//! Every frame becomes one FrameRecord in the training file.
//! Run until ROMs are exhausted = run until every ROM produces a complete training file.

use gb_core::{Cartridge, GbCore, FrameRecord, LCD_WIDTH, LCD_HEIGHT, CYCLES_PER_FRAME};
use std::collections::HashMap;

fn fnv1a(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for &b in data { h ^= b as u32; h = h.wrapping_mul(0x01000193); }
    h
}

fn synthetic_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x0100] = 0x00; rom[0x0101] = 0xC3; rom[0x0102] = 0x50; rom[0x0103] = 0x01;
    let title = b"EVEZ-OS-TRAIN";
    for (i, &b) in title.iter().enumerate() { rom[0x0134+i] = b; }
    rom[0x0147] = 0x00; rom[0x0148] = 0x00; rom[0x0149] = 0x00; rom[0x014D] = 0xE7;
    // Program: LCD on, VBlank IRQ enabled, spin
    let prog: &[u8] = &[
        0x3E,0x00,0xE0,0x40,  // LCD off
        0x01,0x00,0x98,       // LD BC,0x9800
        0x3E,0x01,0x02,       // LD A,1 / LD (BC),A
        0x3E,0x91,0xE0,0x40,  // LCD on, BG on
        0x3E,0x01,0xE0,0xFF,  // IE = 1 (VBlank)
        0xFB,                  // EI
        0x3E,0xAA,0xE0,0x40,  // periodic LCDC write to exercise register path
        0xC3,0x50,0x01,       // JP back (spin)
    ];
    for (i,&b) in prog.iter().enumerate() { rom[0x0150+i] = b; }
    rom
}

/// Epoch classifier: DMG=gen1_nes, CGB=gen2_snes_genesis
fn epoch_for(cart: &Cartridge) -> &'static str {
    if cart.is_cgb { "gen2_snes_genesis" } else { "gen1_nes" }
}

/// Run a cart for max_frames and return all FrameRecords as JSON string
fn play_to_json(cart: Cartridge, max_frames: u64) -> String {
    use std::fmt::Write as FmtWrite;

    let rom_title = cart.title.clone();
    let mbc_kind = format!("{:?}", cart.kind);
    let epoch = epoch_for(&cart).to_string();
    let rom_sha = {
        // simple FNV over the whole ROM as a stand-in for sha256 in no-dep build
        let h = fnv1a(&cart.rom);
        format!("{:08x}", h)
    };
    let rom_size = cart.rom.len();
    let mut core = GbCore::new(cart);
    let mut records: Vec<String> = Vec::with_capacity(max_frames as usize);
    let mut vblank_count: u64 = 0;

    for frame in 0..max_frames {
        let _ = core.run_frame();
        vblank_count += 1;
        let fb = core.bus.ppu.framebuffer.clone();
        let wram_hash = fnv1a(&core.bus.wram);
        let vram_hash = fnv1a(&core.bus.vram);
        let oam_hash  = fnv1a(&core.bus.oam);
        let samples = core.bus.apu.sample_buffer.len() / 2;
        let _ = core.bus.apu.drain_samples();

        let rec = format!(
            concat!(
                "{{"frame":{},"t_cycles":{},",
                ""pc":{},"sp":{},"a":{},"f":{},",
                ""bc":{},"de":{},"hl":{},",
                ""halted":{},"ime":{},",
                ""ly":{},"lcdc":{},"ppu_mode":{},",
                ""vblank_count":{},",
                ""sq1_on":{},"sq2_on":{},"wave_on":{},"noise_on":{},",
                ""samples":{},",
                ""rom_bank":{},"ram_bank":{},",
                ""wram_hash":{},"vram_hash":{},"oam_hash":{},",
                ""rom_title":"{}","mbc_kind":"{}","epoch":"{}"}}"
            ),
            frame, core.clock.t_cycles,
            core.regs.pc, core.regs.sp, core.regs.a, core.regs.f,
            core.regs.bc(), core.regs.de(), core.regs.hl(),
            core.halted, core.ime,
            core.bus.ppu.ly, core.bus.ppu.lcdc, core.bus.ppu.mode as u8,
            vblank_count,
            core.bus.apu.sq1.enabled, core.bus.apu.sq2.enabled,
            core.bus.apu.wave.enabled, core.bus.apu.noise.enabled,
            samples,
            core.bus.mbc.rom_bank, core.bus.mbc.ram_bank,
            wram_hash, vram_hash, oam_hash,
            rom_title, mbc_kind, epoch
        );
        records.push(rec);
    }

    let frames_json = records.join(",\n  ");
    format!(
        concat!(
            "{{\n",
            "  \"version\": \"mrom.train.v1\",\n",
            "  \"rom_title\": \"{}\",\n",
            "  \"rom_sha\": \"{}\",\n",
            "  \"rom_size_bytes\": {},\n",
            "  \"mbc_kind\": \"{}\",\n",
            "  \"epoch\": \"{}\",\n",
            "  \"total_frames\": {},\n",
            "  \"total_cycles\": {},\n",
            "  \"frames\": [\n  {}\n  ]\n",
            "}}"
        ),
        rom_title, rom_sha, rom_size, mbc_kind, epoch,
        max_frames, core.clock.t_cycles, frames_json
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let max_frames: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let out_path = args.get(2).cloned().unwrap_or_else(|| "output.mrom.train.json".to_string());

    println!("MetaROM LetsPlay Training Runner");
    println!("frames={} output={}", max_frames, out_path);
    println!("Building synthetic EVEZ-OS-TRAIN ROM...");

    let cart = Cartridge::from_bytes(synthetic_rom()).expect("ROM invalid");
    println!("ROM: {} | MBC: {:?} | {}KB | is_cgb={}", cart.title, cart.kind, cart.rom_size_kb, cart.is_cgb);

    let json = play_to_json(cart, max_frames);

    std::fs::write(&out_path, &json).expect("Failed to write training file");
    println!("Training file written: {} ({} bytes)", out_path, json.len());
    println!("=== TRAINING EXTRACTION COMPLETE ===");
    println!("Every ROM run now produces a .mrom.train.json.");
    println!("Feed these into the EVEZ-OS console_war_trainer for epoch progression.");
}
