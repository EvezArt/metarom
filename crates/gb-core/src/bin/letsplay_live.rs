//! letsplay_live — Live replay runner with frame capture + save state
//! Usage: letsplay_live <rom_path> <n_frames> [output_dir] [--save-state]
//!
//! Runs the emulator for N frames, captures mrom.replay.v1 JSON,
//! optionally saves state to .mrom.sav, broadcasts mrom.snap.v1 frames to stdout.

use gb_core::{Cartridge, GbCore, ReplayCapture};
use std::{env, fs, path::Path, time::Instant};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <rom_path> <n_frames> [output_dir] [--save-state] [--broadcast]", args[0]);
        std::process::exit(1);
    }

    let rom_path   = &args[1];
    let n_frames: u64 = args[2].parse().unwrap_or(60);
    let output_dir = if args.len() > 3 && !args[3].starts_with("--") { &args[3] } else { "." };
    let save_state = args.iter().any(|a| a == "--save-state");
    let broadcast  = args.iter().any(|a| a == "--broadcast");

    // Load ROM
    let rom_bytes = fs::read(rom_path).unwrap_or_else(|e| {
        eprintln!("Cannot read ROM: {e}"); std::process::exit(1);
    });
    let cart = Cartridge::from_bytes(rom_bytes).unwrap_or_else(|e| {
        eprintln!("Invalid ROM: {e}"); std::process::exit(1);
    });

    let rom_title = cart.title.clone();
    let mut core = GbCore::new(cart);
    let mut replay = ReplayCapture::new(n_frames as usize, &rom_title);

    let t0 = Instant::now();
    let mut frame_count = 0u64;

    eprintln!("[letsplay_live] ROM: {} | Frames: {} | Save: {} | Broadcast: {}",
              rom_title, n_frames, save_state, broadcast);

    for _ in 0..n_frames {
        if core.run_frame().is_err() { break; }

        // Capture replay frame
        replay.capture(&core);

        // Live broadcast: emit snap JSON to stdout (NDJSON)
        if broadcast {
            println!("{}", core.state_json());
        }

        frame_count += 1;
        if frame_count % 60 == 0 {
            eprintln!("[letsplay_live] Frame {} — {}", frame_count, core.state_summary());
        }
    }

    let elapsed = t0.elapsed().as_secs_f64();
    eprintln!("[letsplay_live] Done: {} frames in {:.2}s ({:.1} fps)",
              frame_count, elapsed, frame_count as f64 / elapsed.max(0.001));

    // Save outputs
    let stem = Path::new(rom_path).file_stem().unwrap_or_default().to_str().unwrap_or("rom");
    fs::create_dir_all(output_dir).ok();

    let replay_path = format!("{}/{}.mrom.replay.json", output_dir, stem);
    replay.save(Path::new(&replay_path)).unwrap_or_else(|e| eprintln!("Replay save error: {e}"));
    eprintln!("[letsplay_live] Replay: {}", replay_path);

    if save_state {
        let sav_path = format!("{}/{}.mrom.sav", output_dir, stem);
        core.save_state_to_file(Path::new(&sav_path))
            .unwrap_or_else(|e| eprintln!("Save state error: {e}"));
        eprintln!("[letsplay_live] State: {}", sav_path);
    }

    // Final summary JSON to stdout (if not broadcasting frames)
    if !broadcast {
        println!("{}", core.state_json());
    }
}
