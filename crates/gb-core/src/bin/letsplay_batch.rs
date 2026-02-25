
//! letsplay_batch — MetaROM Phase 5 ROM Batch Training Runner
//! Iterates over a directory of .gb/.gbc ROM files and produces one
//! .mrom.train.json per ROM. Every ROM that runs becomes a training file.
//!
//! Usage:
//!   cargo run --bin letsplay_batch -- <roms_dir> <output_dir> [frames_per_rom]
//!
//! Output:
//!   <output_dir>/<rom_filename>.mrom.train.json  — one per ROM
//!   <output_dir>/batch_manifest.json             — summary of all runs

use gb_core::{Cartridge, GbCore, CYCLES_PER_FRAME};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn fnv1a(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for &b in data { h ^= b as u32; h = h.wrapping_mul(0x01000193); }
    h
}

fn epoch_for_cgb(is_cgb: bool) -> &'static str {
    if is_cgb { "gen2_snes_genesis" } else { "gen1_nes" }
}

#[derive(Debug)]
struct RomResult {
    path: String,
    title: String,
    mbc_kind: String,
    epoch: &'static str,
    frames: u64,
    cycles: u64,
    output_path: String,
    elapsed_ms: u128,
    error: Option<String>,
}

fn process_rom(rom_path: &Path, output_dir: &Path, frames: u64) -> RomResult {
    let start = Instant::now();
    let stem = rom_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let out_name = format!("{}.mrom.train.json", stem);
    let out_path = output_dir.join(&out_name);

    let rom_bytes = match std::fs::read(rom_path) {
        Ok(b) => b, Err(e) => return RomResult {
            path: rom_path.to_string_lossy().to_string(), title: stem.clone(),
            mbc_kind: "?".into(), epoch: "unknown", frames: 0, cycles: 0,
            output_path: out_path.to_string_lossy().to_string(),
            elapsed_ms: start.elapsed().as_millis(),
            error: Some(format!("read error: {e}")),
        }
    };

    let cart = match Cartridge::from_bytes(rom_bytes) {
        Ok(c) => c, Err(e) => return RomResult {
            path: rom_path.to_string_lossy().to_string(), title: stem.clone(),
            mbc_kind: "?".into(), epoch: "unknown", frames: 0, cycles: 0,
            output_path: out_path.to_string_lossy().to_string(),
            elapsed_ms: start.elapsed().as_millis(),
            error: Some(format!("cart error: {e}")),
        }
    };

    let title    = cart.title.clone();
    let mbc_kind = format!("{:?}", cart.kind);
    let epoch    = epoch_for_cgb(cart.is_cgb);
    let rom_sha  = format!("{:08x}", fnv1a(&cart.rom));

    let mut core = GbCore::new(cart);
    let mut records: Vec<String> = Vec::with_capacity(frames as usize);
    let mut vblank_count: u64 = 0;

    for frame in 0..frames {
        if core.run_frame().is_err() { break; }
        vblank_count += 1;
        let wh = fnv1a(&core.bus.wram);
        let vh = fnv1a(&core.bus.vram);
        let oh = fnv1a(&core.bus.oam);
        let samp = core.bus.apu.sample_buffer.len() / 2;
        let _ = core.bus.apu.drain_samples();

        records.push(format!(
            concat!(
                "{{\"frame\":{},\"t_cycles\":{},\"pc\":{},\"sp\":{},",
                "\"a\":{},\"f\":{},\"bc\":{},\"de\":{},\"hl\":{},",
                "\"ly\":{},\"lcdc\":{},\"ppu_mode\":{},",
                "\"sq1\":{},\"sq2\":{},\"wave\":{},\"noise\":{},\"samples\":{},",
                "\"rom_bank\":{},\"ram_bank\":{},",
                "\"wh\":{},\"vh\":{},\"oh\":{}}}"
            ),
            frame, core.clock.t_cycles, core.regs.pc, core.regs.sp,
            core.regs.a, core.regs.f,
            core.regs.bc(), core.regs.de(), core.regs.hl(),
            core.bus.ppu.ly, core.bus.ppu.lcdc, core.bus.ppu.mode as u8,
            core.bus.apu.sq1.enabled as u8, core.bus.apu.sq2.enabled as u8,
            core.bus.apu.wave.enabled as u8, core.bus.apu.noise.enabled as u8, samp,
            core.bus.mbc.rom_bank, core.bus.mbc.ram_bank,
            wh, vh, oh
        ));
    }

    let total_cycles = core.clock.t_cycles;
    let frames_done = records.len() as u64;
    let frames_json = records.join(",\n  ");

    let json = format!(
        "{{\n  \"version\": \"mrom.train.v1\",\n  \"rom_title\": \"{}\",\n  \"rom_sha\": \"{}\",\n  \"mbc_kind\": \"{}\",\n  \"epoch\": \"{}\",\n  \"total_frames\": {},\n  \"total_cycles\": {},\n  \"frames\": [\n  {}\n  ]\n}}",
        title, rom_sha, mbc_kind, epoch, frames_done, total_cycles, frames_json
    );

    if let Err(e) = std::fs::write(&out_path, &json) {
        return RomResult {
            path: rom_path.to_string_lossy().to_string(), title, mbc_kind, epoch,
            frames: frames_done, cycles: total_cycles,
            output_path: out_path.to_string_lossy().to_string(),
            elapsed_ms: start.elapsed().as_millis(),
            error: Some(format!("write error: {e}")),
        };
    }

    RomResult {
        path: rom_path.to_string_lossy().to_string(), title, mbc_kind, epoch,
        frames: frames_done, cycles: total_cycles,
        output_path: out_path.to_string_lossy().to_string(),
        elapsed_ms: start.elapsed().as_millis(),
        error: None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let roms_dir    = args.get(1).map(|s| PathBuf::from(s)).unwrap_or_else(|| PathBuf::from("roms"));
    let output_dir  = args.get(2).map(|s| PathBuf::from(s)).unwrap_or_else(|| PathBuf::from("training_output"));
    let frames: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(300); // 5 seconds at 60fps

    println!("MetaROM Batch Training Runner");
    println!("  roms_dir:   {}", roms_dir.display());
    println!("  output_dir: {}", output_dir.display());
    println!("  frames/ROM: {}", frames);

    std::fs::create_dir_all(&output_dir).expect("Cannot create output dir");

    // Collect ROM files
    let rom_files: Vec<PathBuf> = std::fs::read_dir(&roms_dir)
        .expect("Cannot read roms dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            matches!(ext.to_lowercase().as_str(), "gb" | "gbc" | "rom")
        })
        .collect();

    if rom_files.is_empty() {
        // No ROMs? Run the synthetic built-in ROM as smoke test
        println!("No ROMs found in {}. Running synthetic EVEZ-OS-TRAIN ROM...", roms_dir.display());
        // synthetic_rom path handled by letsplay_train; here we just report
        println!("Run: cargo run --bin letsplay_train -- {} output.mrom.train.json", frames);
        println!("Or drop .gb/.gbc files into {} and re-run.", roms_dir.display());
        return;
    }

    println!("Found {} ROM file(s). Processing...\n", rom_files.len());

    let mut results: Vec<RomResult> = Vec::new();
    for (i, path) in rom_files.iter().enumerate() {
        print!("[{}/{}] {} ... ", i+1, rom_files.len(), path.file_name().unwrap_or_default().to_string_lossy());
        let r = process_rom(path, &output_dir, frames);
        match &r.error {
            None    => println!("OK ({} frames, {}ms) → {}", r.frames, r.elapsed_ms, r.output_path),
            Some(e) => println!("FAILED: {e}"),
        }
        results.push(r);
    }

    // Write manifest
    let ok_count   = results.iter().filter(|r| r.error.is_none()).count();
    let fail_count = results.len() - ok_count;
    let total_frames: u64 = results.iter().map(|r| r.frames).sum();

    let manifest_entries: Vec<String> = results.iter().map(|r| format!(
        "  {{\"title\":\"{}\",\"epoch\":\"{}\",\"mbc\":\"{}\",\"frames\":{},\"ok\":{},\"path\":\"{}\"}}",
        r.title, r.epoch, r.mbc_kind, r.frames, r.error.is_none(), r.output_path
    )).collect();

    let manifest = format!(
        "{{\n  \"total_roms\":{},\"ok\":{},\"failed\":{},\"total_frames\":{},\n  \"roms\":[\n{}\n  ]\n}}",
        results.len(), ok_count, fail_count, total_frames,
        manifest_entries.join(",\n")
    );

    let manifest_path = output_dir.join("batch_manifest.json");
    std::fs::write(&manifest_path, &manifest).expect("Cannot write manifest");

    println!("\n=== BATCH COMPLETE ===");
    println!("  ROMs processed: {}", results.len());
    println!("  Succeeded:      {}", ok_count);
    println!("  Failed:         {}", fail_count);
    println!("  Total frames:   {}", total_frames);
    println!("  Manifest:       {}", manifest_path.display());
    println!("\nEvery ROM that ran is now a training file.");
}
