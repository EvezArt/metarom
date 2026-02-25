# MetaROM

**Universal game runtime + compatibility planner + network crystallizer.**

One system. Many ROMs. One mind.

## Phase history

| Phase | What shipped |
|-------|-------------|
| 3 | PPU modes 0-3 + STAT, DIV/TIMA, MBC1/3/5, CB-prefix opcodes, framebuffer |
| 4 | OAM sprites, window layer, APU channels (Sq1/2/Wave/Noise), training extractor |
| 5 | Full SM83 ISA (251 opcodes), APU frame sequencer, `letsplay_batch` batch runner |
| 6 | CGB VRAM bank, double-speed mode, MBC3 RTC, NR51 panning, save state API |
| 7 | CGB WRAM banking, color palettes (BCPS/BCPD), STOP speed switch, load_state(), live replay API, Network Crystallizer |

## Phase 7 Feature List

### CGB Hardware (Complete)
- **WRAM banking** — FF70 register, `wram: [[u8;0x1000];8]`, 8×4KB banks (bank 0 fixed, 1-7 switchable)
- **Color palettes** — BCPS/BCPD (FF68/69), OCPS/OCPD (FF6A/6B), 8 palettes × 4 colors × RGB555 auto-increment
- **STOP instruction** — executes CGB double-speed switch on armed KEY1 (FF4D bit 0)
- `Bus::cgb_color()` — RGB555 → RGB888 decoder
- `Bus::bg_palette_rgb()` / `Bus::obj_palette_rgb()` — full palette export

### Live Replay API
- `GbCore::framebuffer_rgb()` — 160×144×3 RGB888 bytes with CGB palette decode
- `GbCore::framebuffer_hex()` — compact hex string for JSON embedding
- `GbCore::state_json()` — `mrom.snap.v1` full snapshot for WebSocket streaming
- `ReplayFrame` / `ReplayCapture` — frame-by-frame emulator recording
- `ReplayCapture::capture(core)` — record one frame
- `ReplayCapture::to_json()` / `ReplayCapture::save(path)` — `mrom.replay.v1` manifest

### Save/Load State
- `GbCore::load_state(bytes)` — restore from `mrom.sav.v1` JSON
- `GbCore::load_state_from_file(path)` — load from file
- Restores: CPU registers, PC/SP, flags, halted/IME, t_cycles, MBC banks, VRAM/WRAM/HRAM/OAM

### Network Crystallizer (`tools/network_crystallizer.py`)
Many ROMs → one training crystal. The system that borrows and trains itself from every game.

```
ROM pool → per-ROM feature extraction → CrossRomCrystal.borrow()
       → BehaviorCluster detection → CrystalManifest (mrom.crystal.v1)
       → EVEZ-OS console_war_trainer epoch input
```

**Manifests**: `mrom.crystal.v1` — epochs, behavior clusters, cross-epoch patterns, crystallization score

### New Binary: `letsplay_live`
```bash
# Live replay: run 120 frames, save replay + state, broadcast snaps
cargo run --bin letsplay_live -- game.gb 120 output/ --save-state --broadcast | websocat ws://localhost:8080/replay

# Just capture replay
cargo run --bin letsplay_live -- game.gb 60 output/
```

## Network Crystallizer

```bash
# Crystallize a ROM directory
python tools/network_crystallizer.py roms/ output/ --frames 60

# With live NDJSON broadcast (pipe to WebSocket bridge)
python tools/network_crystallizer.py roms/ output/ --frames 120 --broadcast | node ws_bridge.js

# Force epoch tag
python tools/network_crystallizer.py roms/ output/ --epoch gen2_snes_genesis
```

Outputs:
- `output/mrom.crystal.json` — full crystal manifest (mrom.crystal.v1)
- `output/<rom>.mrom.train.json` — per-ROM training records
- Broadcast: NDJSON stream with `frame`, `crystal_update`, `epoch_advance` events

## Full pipeline

```
ROMs → letsplay_batch → *.mrom.train.json
                ↓
    network_crystallizer.py
                ↓
       mrom.crystal.v1 JSON
                ↓
  EVEZ-OS console_war_trainer
  (epoch: gen1_nes → gen2_snes_genesis → ...)
                ↓
     FreeMix Engine (audio stems)
     + Live Replay (letsplay_live)
```

## Quick start

```bash
cargo build --release

# Single ROM training
cargo run --bin letsplay_train -- 60 output.mrom.train.json

# Batch all ROMs
cargo run --bin letsplay_batch -- roms/ training_output/ 60

# Live replay + state
cargo run --bin letsplay_live -- game.gb 120 output/ --save-state

# Crystallize
python tools/network_crystallizer.py roms/ crystal_output/ --frames 60

# FreeMix (audio stems from any audio file)
python tools/freemix/freemix_engine.py track.wav stems/ --mode freestyle --variations 8
```

## License

AGPL-3.0 (community/free tier). Commercial licenses available.
