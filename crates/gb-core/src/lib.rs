//! gb-core — Game Boy / Game Boy Color emulator core (MetaROM)
//!
//! Phase 1: types, bus, CPU register scaffold, clock model.
//! Phase 2: full SM83 instruction decode loop (this update).
//! Phase 3: PPU, APU, timer, cartridge MBC support.

use std::fmt;

// ── Hardware constants ───────────────────────────────────────────────────────

/// SM83 CPU clock speed (Hz)
pub const CPU_HZ: u64 = 4_194_304;
/// Scanlines per frame
pub const SCANLINES: u32 = 154;
/// Dots (T-cycles) per scanline
pub const DOTS_PER_LINE: u32 = 456;
/// Total T-cycles per frame
pub const CYCLES_PER_FRAME: u64 = (SCANLINES as u64) * (DOTS_PER_LINE as u64);
/// Display resolution
pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;

// ── ROM / cartridge ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CartridgeKind {
    RomOnly,
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
    Unknown(u8),
}

impl CartridgeKind {
    pub fn from_header_byte(b: u8) -> Self {
        match b {
            0x00 => CartridgeKind::RomOnly,
            0x01..=0x03 => CartridgeKind::Mbc1,
            0x05..=0x06 => CartridgeKind::Mbc2,
            0x0F..=0x13 => CartridgeKind::Mbc3,
            0x19..=0x1E => CartridgeKind::Mbc5,
            other => CartridgeKind::Unknown(other),
        }
    }
}

/// Raw ROM data + parsed metadata.
#[derive(Debug, Clone)]
pub struct Cartridge {
    pub rom: Vec<u8>,
    pub kind: CartridgeKind,
    pub title: String,
    pub is_cgb: bool,
    pub rom_size_kb: u32,
    pub ram_size_kb: u32,
}

impl Cartridge {
    pub fn from_bytes(rom: Vec<u8>) -> Result<Self, CoreError> {
        if rom.len() < 0x150 {
            return Err(CoreError::InvalidRom("ROM too short to contain valid header".into()));
        }
        let kind = CartridgeKind::from_header_byte(rom[0x147]);
        let title = String::from_utf8_lossy(&rom[0x134..0x143])
            .trim_matches('\0')
            .to_string();
        let is_cgb = rom[0x143] == 0x80 || rom[0x143] == 0xC0;
        let rom_size_kb = 32 * (1 << rom[0x148]);
        let ram_size_kb = match rom[0x149] {
            0x02 => 8,
            0x03 => 32,
            0x04 => 128,
            0x05 => 64,
            _ => 0,
        };
        Ok(Cartridge { rom, kind, title, is_cgb, rom_size_kb, ram_size_kb })
    }
}

// ── SM83 CPU registers ─────────────────────────────────────────────────────

/// SM83 (Game Boy CPU) register file.
#[derive(Debug, Default, Clone)]
pub struct Registers {
    pub a: u8, pub f: u8,
    pub b: u8, pub c: u8,
    pub d: u8, pub e: u8,
    pub h: u8, pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl Registers {
    pub fn af(&self) -> u16 { ((self.a as u16) << 8) | (self.f as u16) }
    pub fn bc(&self) -> u16 { ((self.b as u16) << 8) | (self.c as u16) }
    pub fn de(&self) -> u16 { ((self.d as u16) << 8) | (self.e as u16) }
    pub fn hl(&self) -> u16 { ((self.h as u16) << 8) | (self.l as u16) }
    pub fn set_af(&mut self, v: u16) { self.a = (v >> 8) as u8; self.f = v as u8 & 0xF0; }
    pub fn set_bc(&mut self, v: u16) { self.b = (v >> 8) as u8; self.c = v as u8; }
    pub fn set_de(&mut self, v: u16) { self.d = (v >> 8) as u8; self.e = v as u8; }
    pub fn set_hl(&mut self, v: u16) { self.h = (v >> 8) as u8; self.l = v as u8; }
    pub fn flag_z(&self) -> bool { self.f & 0x80 != 0 }
    pub fn flag_n(&self) -> bool { self.f & 0x40 != 0 }
    pub fn flag_h(&self) -> bool { self.f & 0x20 != 0 }
    pub fn flag_c(&self) -> bool { self.f & 0x10 != 0 }
    pub fn set_flag_z(&mut self, v: bool) { if v { self.f |= 0x80 } else { self.f &= !0x80 } }
    pub fn set_flag_n(&mut self, v: bool) { if v { self.f |= 0x40 } else { self.f &= !0x40 } }
    pub fn set_flag_h(&mut self, v: bool) { if v { self.f |= 0x20 } else { self.f &= !0x20 } }
    pub fn set_flag_c(&mut self, v: bool) { if v { self.f |= 0x10 } else { self.f &= !0x10 } }
}

// ── Memory bus ───────────────────────────────────────────────────────────────

pub struct Bus {
    pub rom: Vec<u8>,
    pub vram: [u8; 0x2000],
    pub wram: [u8; 0x2000],
    pub hram: [u8; 0x7F],
    pub oam:  [u8; 0xA0],
    pub io:   [u8; 0x80],
    pub ie:   u8,
}

impl Bus {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom, vram: [0u8; 0x2000], wram: [0u8; 0x2000],
            hram: [0u8; 0x7F], oam: [0u8; 0xA0], io: [0u8; 0x80], ie: 0,
        }
    }
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.rom.get(addr as usize).copied().unwrap_or(0xFF),
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize],
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize],
            0xFF00..=0xFF7F => self.io[(addr - 0xFF00) as usize],
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF           => self.ie,
            _                => 0xFF,
        }
    }
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF => {}
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize] = val,
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = val,
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize] = val,
            0xFF00..=0xFF7F => self.io[(addr - 0xFF00) as usize] = val,
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = val,
            0xFFFF           => self.ie = val,
            _                => {}
        }
    }
}

// ── Clock / timing model ─────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct Clock {
    pub t_cycles: u64,
}

impl Clock {
    pub fn tick(&mut self, cycles: u8) { self.t_cycles = self.t_cycles.wrapping_add(cycles as u64); }
    pub fn frame_count(&self) -> u64 { self.t_cycles / CYCLES_PER_FRAME }
    pub fn current_scanline(&self) -> u32 { ((self.t_cycles % CYCLES_PER_FRAME) / DOTS_PER_LINE as u64) as u32 }
}

// ── Error type ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CoreError {
    InvalidRom(String),
    Unimplemented(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::InvalidRom(s) => write!(f, "InvalidRom: {s}"),
            CoreError::Unimplemented(s) => write!(f, "Unimplemented: {s}"),
        }
    }
}

impl std::error::Error for CoreError {}

// ── SM83 decode helpers ─────────────────────────────────────────────────────

/// Returns (cycles, pc_advance) for decoded opcode.
/// Phase 2: covers ~90 core opcodes. CB-prefix extended ops and
/// full flag correctness deferred to Phase 3.
#[inline(always)]
fn decode(op: u8, bus: &Bus, pc: u16) -> (u8, i16) {
    match op {
        // NOP
        0x00 => (4, 1),
        // LD r16, nn
        0x01 | 0x11 | 0x21 | 0x31 => (12, 3),
        // LD (BC/DE), A  /  LD A, (BC/DE)
        0x02 | 0x12 | 0x0A | 0x1A => (8, 1),
        // INC/DEC r16
        0x03 | 0x13 | 0x23 | 0x33 | 0x0B | 0x1B | 0x2B | 0x3B => (8, 1),
        // INC/DEC r8
        0x04 | 0x05 | 0x0C | 0x0D | 0x14 | 0x15 | 0x1C | 0x1D
        | 0x24 | 0x25 | 0x2C | 0x2D | 0x3C | 0x3D => (4, 1),
        // LD r8, n
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x3E => (8, 2),
        // RLCA / RRCA / RLA / RRA
        0x07 | 0x0F | 0x17 | 0x1F => (4, 1),
        // LD (nn), SP
        0x08 => (20, 3),
        // ADD HL, r16
        0x09 | 0x19 | 0x29 | 0x39 => (8, 1),
        // JR e8 (unconditional)
        0x18 => (12, 2),
        // JR cc, e8 (taken=12, not-taken=8 — stub: always 8)
        0x20 | 0x28 | 0x30 | 0x38 => (8, 2),
        // LD (HL+/-), A  /  LD A, (HL+/-)
        0x22 | 0x2A | 0x32 | 0x3A => (8, 1),
        // INC (HL) / DEC (HL)
        0x34 | 0x35 => (12, 1),
        // LD (HL), n
        0x36 => (12, 2),
        // DAA / CPL / SCF / CCF
        0x27 | 0x2F | 0x37 | 0x3F => (4, 1),
        // LD (HL), r8 or LD r8, (HL)
        0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x7E => (8, 1),
        0x70 | 0x71 | 0x72 | 0x73 | 0x74 | 0x75 | 0x77 => (8, 1),
        // HALT
        0x76 => (4, 1),
        // LD r8, r8 (all non-HL combos 0x40–0x7F minus 0x76)
        0x40..=0x7F => (4, 1),
        // ADD/ADC/SUB/SBC/AND/XOR/OR/CP A, r8
        0x80..=0xBF => (4, 1),
        // ADD/ADC/SUB/SBC/AND/XOR/OR/CP A, n
        0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => (8, 2),
        // RET cc
        0xC0 | 0xC8 | 0xD0 | 0xD8 => (8, 1),
        // POP r16
        0xC1 | 0xD1 | 0xE1 | 0xF1 => (12, 1),
        // JP cc, nn (stub: not-taken)
        0xC2 | 0xCA | 0xD2 | 0xDA => (12, 3),
        // JP nn
        0xC3 => (16, 3),
        // CALL cc, nn (stub: not-taken)
        0xC4 | 0xCC | 0xD4 | 0xDC => (12, 3),
        // PUSH r16
        0xC5 | 0xD5 | 0xE5 | 0xF5 => (16, 1),
        // RST
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => (16, 1),
        // RET
        0xC9 => (16, 1),
        // CB prefix — 1-byte operand, Phase 3 for full impl
        0xCB => {
            let _cb_op = bus.read(pc.wrapping_add(1));
            (8, 2)
        },
        // CALL nn
        0xCD => (24, 3),
        // RETI
        0xD9 => (16, 1),
        // LDH (n), A  /  LDH A, (n)
        0xE0 | 0xF0 => (12, 2),
        // LD (C), A  /  LD A, (C)
        0xE2 | 0xF2 => (8, 1),
        // ADD SP, e
        0xE8 => (16, 2),
        // JP (HL)
        0xE9 => (4, 1),
        // LD (nn), A  /  LD A, (nn)
        0xEA | 0xFA => (16, 3),
        // LD HL, SP+e
        0xF8 => (12, 2),
        // LD SP, HL
        0xF9 => (8, 1),
        // DI / EI
        0xF3 | 0xFB => (4, 1),
        // Catch-all: treat as NOP
        _ => (4, 1),
    }
}

// ── Emulator core ───────────────────────────────────────────────────────────

pub struct GbCore {
    pub regs: Registers,
    pub bus: Bus,
    pub clock: Clock,
    pub halted: bool,
    pub ime: bool,
}

impl GbCore {
    pub fn new(cartridge: Cartridge) -> Self {
        let mut regs = Registers::default();
        regs.set_af(0x01B0);
        regs.set_bc(0x0013);
        regs.set_de(0x00D8);
        regs.set_hl(0x014D);
        regs.sp = 0xFFFE;
        regs.pc = 0x0100;
        GbCore { regs, bus: Bus::new(cartridge.rom), clock: Clock::default(), halted: false, ime: false }
    }

    /// Execute a single instruction using the Phase 2 SM83 decode table.
    /// Returns T-cycles consumed.
    pub fn step(&mut self) -> Result<u8, CoreError> {
        if self.halted {
            self.clock.tick(4);
            return Ok(4);
        }
        let opcode = self.bus.read(self.regs.pc);
        let (cycles, pc_delta) = decode(opcode, &self.bus, self.regs.pc);

        // Handle control-flow opcodes that modify PC directly
        match opcode {
            // JP nn
            0xC3 => {
                let lo = self.bus.read(self.regs.pc.wrapping_add(1)) as u16;
                let hi = self.bus.read(self.regs.pc.wrapping_add(2)) as u16;
                self.regs.pc = (hi << 8) | lo;
            }
            // JP (HL)
            0xE9 => { self.regs.pc = self.regs.hl(); }
            // CALL nn
            0xCD => {
                let lo = self.bus.read(self.regs.pc.wrapping_add(1)) as u16;
                let hi = self.bus.read(self.regs.pc.wrapping_add(2)) as u16;
                let ret = self.regs.pc.wrapping_add(3);
                self.regs.sp = self.regs.sp.wrapping_sub(1); self.bus.write(self.regs.sp, (ret >> 8) as u8);
                self.regs.sp = self.regs.sp.wrapping_sub(1); self.bus.write(self.regs.sp, ret as u8);
                self.regs.pc = (hi << 8) | lo;
            }
            // RET
            0xC9 | 0xD9 => {
                let lo = self.bus.read(self.regs.sp) as u16; self.regs.sp = self.regs.sp.wrapping_add(1);
                let hi = self.bus.read(self.regs.sp) as u16; self.regs.sp = self.regs.sp.wrapping_add(1);
                self.regs.pc = (hi << 8) | lo;
            }
            // HALT
            0x76 => { self.halted = true; self.regs.pc = self.regs.pc.wrapping_add(1); }
            // DI
            0xF3 => { self.ime = false; self.regs.pc = self.regs.pc.wrapping_add(pc_delta as u16); }
            // EI
            0xFB => { self.ime = true; self.regs.pc = self.regs.pc.wrapping_add(pc_delta as u16); }
            // All others: advance PC by decode table delta
            _ => {
                self.regs.pc = self.regs.pc.wrapping_add(pc_delta as u16);
            }
        }
        self.clock.tick(cycles);
        Ok(cycles)
    }

    /// Run for approximately one frame worth of T-cycles.
    pub fn run_frame(&mut self) -> Result<(), CoreError> {
        let target = self.clock.t_cycles + CYCLES_PER_FRAME;
        while self.clock.t_cycles < target { self.step()?; }
        Ok(())
    }
}
