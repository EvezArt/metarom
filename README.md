# MetaROM

**Universal game runtime + compatibility planner.**

MetaROM treats emulator cores as arcade cart modules and games as ROM artifacts,
then uses the UCF (Universal Compatibility Fabric) planner to determine the best
execution strategy for any artifact on any target platform.

Every ROM that runs through the emulator becomes a training file. That's the law.

## Repo layout

```
metarom/
├── schemas/
│   ├── capability_graph.schema.json
│   ├── game_requirement.schema.json
│   ├── sts_profile.schema.json
│   └── training_record.schema.json
├── crates/
│   ├── gb-core/
│   │   └── src/
│   │       ├── lib.rs              # Phase 6: CGB hardware, MBC3 RTC, save state
│   │       └── bin/
│   │           ├── letsplay.rs
│   │           ├── letsplay_train.rs
│   │           └── letsplay_batch.rs
│   ├── ucf-planner/
│   └── mrom-ecore-abi/
└── examples/
```

## Phase history

| Phase | What shipped |
|-------|-------------|
| 3 | PPU modes 0-3, STAT, DIV/TIMA, MBC1/3/5, CB-prefix opcodes, framebuffer |
| 4 | OAM sprites, window layer, palette registers, APU channels (Sq1/2/Wave/Noise), training extractor |
| 5 | Full SM83 ISA (251 opcodes), APU frame sequencer, `letsplay_batch` batch runner |
| 6 | CGB VRAM bank switching, double-speed mode, MBC3 RTC, NR51 panning, save state API |

## Phase 6 Features

- **CGB VRAM bank switching** — `vram: [[u8;0x2000]; 2]`, FF4F register, bank 0/1
- **CGB double-speed mode** — KEY1 (FF4D) register, speed_switch_armed flag; subsystems run at half rate in 2x mode
- **MBC3 full RTC** — 5 RTC registers (S/M/H/DL/DH), latch via 0x6000-0x7FFF 0→1 sequence, select via 0x08-0x0C
- **NR51 panning** — FF25 register wired into Apu, default 0xFF (all channels both speakers)
- **Save state API** — `GbCore::save_state() -> Vec<u8>`, `save_state_to_file(path)`, `mrom.sav.v1` JSON format

## Quick start

```bash
cargo build

# Single ROM training (60 frames → JSON)
cargo run --bin letsplay_train -- 60 output.mrom.train.json

# Batch: every .gb/.gbc in roms/ (300 frames each)
cargo run --bin letsplay_batch -- roms/ training_output/ 300

# Save state example (Rust API)
let bytes = core.save_state();
core.save_state_to_file(Path::new("game.mrom.sav")).unwrap();
```

## Training pipeline

```
roms/*.gb/.gbc → letsplay_batch → training_output/*.mrom.train.json
                                   + batch_manifest.json
                                        ↓
                              EVEZ-OS console_war_trainer
                                        ↓
                              epoch progression (gen1_nes → gen2_snes_genesis → ...)
```

## License

AGPL-3.0 (community/free tier). Commercial licenses available.
