//! gb-core — Game Boy / Game Boy Color emulator core stub (MetaROM)
//!
//! This crate implements the MetaROM emulation core for GB/GBC hardware.
//! It is designed as a UCF-compatible emulation backend: gap analysis from
//! `ucf-planner` selects Strategy::Emulate, and gb-core provides the loop.
//!
//! Phase 1 (this stub): types, bus, CPU register scaffold, clock model.
//! Phase 2: full SM83 instruction decode loop.
//! Phase 3: PPU, APU, timer, cartridge MBC support.

use std::fmt;

// ── Hardware constants ────────────────────────────────────────────────────────

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
    /// Load from raw ROM bytes. Returns error if ROM is too short to parse header.
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

// ── SM83 CPU registers ────────────────────────────────────────────────────────

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

// ── Memory bus ────────────────────────────────────────────────────────────────

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
            _               => 0xFF,
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
            _               => {}
        }
    }
}

// ── Clock / timing model ──────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct Clock {
    pub t_cycles: u64,
}

impl Clock {
    pub fn tick(&mut self, cycles: u8) { self.t_cycles = self.t_cycles.wrapping_add(cycles as u64); }
    pub fn frame_count(&self) -> u64 { self.t_cycles / CYCLES_PER_FRAME }
    pub fn current_scanline(&self) -> u32 { ((self.t_cycles % CYCLES_PER_FRAME) / DOTS_PER_LINE as u64) as u32 }
}

// ── Error type ────────────────────────────────────────────────────────────────

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

// ── Emulator core ─────────────────────────────────────────────────────────────

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

    /// Execute a single instruction. Returns T-cycles consumed.
    /// Phase 1 stub: all opcodes treated as NOP (4 cycles). Full SM83 decode in Phase 2.
    pub fn step(&mut self) -> Result<u8, CoreError> {
        if self.halted { self.clock.tick(4); return Ok(4); }
        let _opcode = self.bus.read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        let cycles: u8 = 4;
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
