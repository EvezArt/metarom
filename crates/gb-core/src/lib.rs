//! gb-core — Game Boy emulator core (MetaROM)
//!
//! Phase 3: PPU modes 0-3 + STAT, DIV/TIMA timer, MBC1/3/5 banking,
//!          CB-prefix full decode, APU channel stubs, framebuffer + letsplay.

use std::fmt;

// ── Hardware constants ──────────────────────────────────────────────────────
pub const CPU_HZ: u64 = 4_194_304;
pub const SCANLINES: u32 = 154;
pub const DOTS_PER_LINE: u32 = 456;
pub const CYCLES_PER_FRAME: u64 = (SCANLINES as u64) * (DOTS_PER_LINE as u64);
pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;
pub const PPU_MODE2_CYCLES: u32 = 80;
pub const PPU_MODE3_CYCLES: u32 = 172;
pub const PPU_MODE0_CYCLES: u32 = 204;
pub const PPU_VBLANK_LINE: u32 = 144;

// ── CartridgeKind ────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CartridgeKind { RomOnly, Mbc1, Mbc2, Mbc3, Mbc5, Unknown(u8) }

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

// ── MBC ───────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct Mbc {
    pub kind: CartridgeKind,
    pub rom_bank: u16,
    pub ram_bank: u8,
    pub ram_enable: bool,
    pub mode: u8,
    pub upper_bits: u8,
    pub rtc_reg: [u8; 5], pub rtc_latch: [u8; 5], pub rtc_latch_state: u8, pub rtc_sel: u8,
}

impl Mbc {
    pub fn new(kind: CartridgeKind) -> Self {
        Mbc { kind, rom_bank: 1, ram_bank: 0, ram_enable: false, mode: 0, upper_bits: 0,
              rtc_reg: [0u8;5], rtc_latch: [0u8;5], rtc_latch_state: 0, rtc_sel: 0xFF }
    }
    pub fn write(&mut self, addr: u16, val: u8) -> bool {
        match &self.kind {
            CartridgeKind::RomOnly => false,
            CartridgeKind::Mbc1 => {
                match addr {
                    0x0000..=0x1FFF => { self.ram_enable = (val & 0x0F) == 0x0A; true }
                    0x2000..=0x3FFF => {
                        let b = (val & 0x1F) as u16;
                        self.rom_bank = (self.rom_bank & 0x60) | if b == 0 { 1 } else { b };
                        true
                    }
                    0x4000..=0x5FFF => {
                        self.upper_bits = val & 0x03;
                        if self.mode == 0 {
                            self.rom_bank = (self.rom_bank & 0x1F) | ((self.upper_bits as u16) << 5);
                        } else { self.ram_bank = self.upper_bits; }
                        true
                    }
                    0x6000..=0x7FFF => { self.mode = val & 0x01; true }
                    _ => false,
                }
            }
            CartridgeKind::Mbc3 => {
                match addr {
                    0x0000..=0x1FFF => { self.ram_enable = (val & 0x0F) == 0x0A; true }
                    0x2000..=0x3FFF => {
                        let b = (val & 0x7F) as u16;
                        self.rom_bank = if b == 0 { 1 } else { b };
                        true
                    }
                    0x4000..=0x5FFF => {
                        if val <= 0x07 { self.ram_bank = val; self.rtc_sel = 0xFF; }
                        else if (0x08..=0x0C).contains(&val) { self.rtc_sel = val - 0x08; }
                        true
                    }
                    0x6000..=0x7FFF => {
                        if self.rtc_latch_state == 0 && val == 0 { self.rtc_latch_state = 1; }
                        else if self.rtc_latch_state == 1 && val == 1 { self.rtc_latch = self.rtc_reg; self.rtc_latch_state = 0; }
                        else { self.rtc_latch_state = 0; }
                        true
                    }
                    _ => false,
                }
            }
            CartridgeKind::Mbc5 => {
                match addr {
                    0x0000..=0x1FFF => { self.ram_enable = (val & 0x0F) == 0x0A; true }
                    0x2000..=0x2FFF => {
                        self.rom_bank = (self.rom_bank & 0x100) | (val as u16);
                        true
                    }
                    0x3000..=0x3FFF => {
                        self.rom_bank = (self.rom_bank & 0xFF) | (((val & 0x01) as u16) << 8);
                        true
                    }
                    0x4000..=0x5FFF => { self.ram_bank = val & 0x0F; true }
                    _ => false,
                }
            }
            _ => false,
        }
    }
    pub fn rom_addr(&self, addr: u16) -> usize {
        match addr {
            0x0000..=0x3FFF => addr as usize,
            0x4000..=0x7FFF => (self.rom_bank as usize) * 0x4000 + (addr as usize - 0x4000),
            _ => addr as usize,
        }
    }
}

// ── Cartridge ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct Cartridge {
    pub rom: Vec<u8>, pub ram: Vec<u8>,
    pub kind: CartridgeKind, pub title: String,
    pub is_cgb: bool, pub rom_size_kb: u32, pub ram_size_kb: u32,
}

impl Cartridge {
    pub fn from_bytes(rom: Vec<u8>) -> Result<Self, CoreError> {
        if rom.len() < 0x150 { return Err(CoreError::InvalidRom("ROM too short".into())); }
        let kind = CartridgeKind::from_header_byte(rom[0x147]);
        let title = String::from_utf8_lossy(&rom[0x134..0x143]).trim_matches('\0').to_string();
        let is_cgb = rom[0x143] == 0x80 || rom[0x143] == 0xC0;
        let rom_size_kb = 32 * (1 << rom[0x148]);
        let ram_size_kb = match rom[0x149] { 0x02=>8, 0x03=>32, 0x04=>128, 0x05=>64, _=>0 };
        let ram = vec![0u8; (ram_size_kb as usize) * 1024];
        Ok(Cartridge { rom, ram, kind, title, is_cgb, rom_size_kb, ram_size_kb })
    }
}

// ── Registers ────────────────────────────────────────────────────────────────
#[derive(Debug, Default, Clone)]
pub struct Registers {
    pub a: u8, pub f: u8, pub b: u8, pub c: u8,
    pub d: u8, pub e: u8, pub h: u8, pub l: u8,
    pub sp: u16, pub pc: u16,
}
impl Registers {
    pub fn af(&self) -> u16 { ((self.a as u16) << 8) | (self.f as u16) }
    pub fn bc(&self) -> u16 { ((self.b as u16) << 8) | (self.c as u16) }
    pub fn de(&self) -> u16 { ((self.d as u16) << 8) | (self.e as u16) }
    pub fn hl(&self) -> u16 { ((self.h as u16) << 8) | (self.l as u16) }
    pub fn set_af(&mut self, v: u16) { self.a = (v>>8) as u8; self.f = v as u8 & 0xF0; }
    pub fn set_bc(&mut self, v: u16) { self.b = (v>>8) as u8; self.c = v as u8; }
    pub fn set_de(&mut self, v: u16) { self.d = (v>>8) as u8; self.e = v as u8; }
    pub fn set_hl(&mut self, v: u16) { self.h = (v>>8) as u8; self.l = v as u8; }
    pub fn flag_z(&self) -> bool { self.f & 0x80 != 0 }
    pub fn flag_n(&self) -> bool { self.f & 0x40 != 0 }
    pub fn flag_h(&self) -> bool { self.f & 0x20 != 0 }
    pub fn flag_c(&self) -> bool { self.f & 0x10 != 0 }
    pub fn set_flag_z(&mut self, v: bool) { if v { self.f |= 0x80 } else { self.f &= !0x80 } }
    pub fn set_flag_n(&mut self, v: bool) { if v { self.f |= 0x40 } else { self.f &= !0x40 } }
    pub fn set_flag_h(&mut self, v: bool) { if v { self.f |= 0x20 } else { self.f &= !0x20 } }
    pub fn set_flag_c(&mut self, v: bool) { if v { self.f |= 0x10 } else { self.f &= !0x10 } }
}

// ── PPU ──────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuMode { HBlank = 0, VBlank = 1, OamScan = 2, Drawing = 3 }

#[derive(Debug, Clone)]
pub struct Ppu {
    pub mode: PpuMode, pub dot: u32, pub ly: u8, pub lyc: u8,
    pub lcdc: u8, pub stat: u8, pub scy: u8, pub scx: u8,
    pub framebuffer: Vec<u8>,
    pub frame_ready: bool, pub stat_irq: bool, pub vblank_irq: bool,
}
impl Ppu {
    pub fn new() -> Self {
        Ppu { mode: PpuMode::OamScan, dot: 0, ly: 0, lyc: 0,
              lcdc: 0x91, stat: 0, scy: 0, scx: 0,
              framebuffer: vec![0u8; LCD_WIDTH * LCD_HEIGHT],
              frame_ready: false, stat_irq: false, vblank_irq: false }
    }
    pub fn step(&mut self, cycles: u8, vram: &[u8; 0x2000], _oam: &[u8; 0xA0]) {
        if self.lcdc & 0x80 == 0 { return; }
        self.stat_irq = false; self.vblank_irq = false;
        self.dot += cycles as u32;
        match self.mode {
            PpuMode::OamScan => {
                if self.dot >= PPU_MODE2_CYCLES { self.dot -= PPU_MODE2_CYCLES; self.mode = PpuMode::Drawing; }
            }
            PpuMode::Drawing => {
                if self.dot >= PPU_MODE3_CYCLES {
                    self.dot -= PPU_MODE3_CYCLES;
                    self.render_scanline(vram);
                    self.mode = PpuMode::HBlank;
                    if self.stat & 0x08 != 0 { self.stat_irq = true; }
                }
            }
            PpuMode::HBlank => {
                if self.dot >= PPU_MODE0_CYCLES {
                    self.dot -= PPU_MODE0_CYCLES; self.ly += 1; self.check_lyc();
                    if self.ly >= PPU_VBLANK_LINE as u8 {
                        self.mode = PpuMode::VBlank; self.vblank_irq = true; self.frame_ready = true;
                        if self.stat & 0x10 != 0 { self.stat_irq = true; }
                    } else {
                        self.mode = PpuMode::OamScan;
                        if self.stat & 0x20 != 0 { self.stat_irq = true; }
                    }
                }
            }
            PpuMode::VBlank => {
                if self.dot >= DOTS_PER_LINE {
                    self.dot -= DOTS_PER_LINE; self.ly += 1; self.check_lyc();
                    if self.ly > 153 {
                        self.ly = 0; self.mode = PpuMode::OamScan; self.frame_ready = false;
                        if self.stat & 0x20 != 0 { self.stat_irq = true; }
                    }
                }
            }
        }
        self.stat = (self.stat & 0xFC) | (self.mode as u8);
    }
    fn check_lyc(&mut self) {
        if self.ly == self.lyc { self.stat |= 0x04; if self.stat & 0x40 != 0 { self.stat_irq = true; } }
        else { self.stat &= !0x04; }
    }
    fn render_scanline(&mut self, vram: &[u8; 0x2000]) {
        let ly = self.ly as usize;
        if ly >= LCD_HEIGHT { return; }
        let lcdc = self.lcdc;
        if lcdc & 0x01 == 0 {
            for x in 0..LCD_WIDTH { self.framebuffer[ly * LCD_WIDTH + x] = 0; }
            return;
        }
        let tile_map_base: usize = if lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
        let tile_data_base: usize = if lcdc & 0x10 != 0 { 0x0000 } else { 0x0800 };
        let signed_addressing = lcdc & 0x10 == 0;
        let map_y = (ly + self.scy as usize) & 0xFF;
        let tile_row = map_y / 8; let pixel_row = map_y % 8;
        for x in 0..LCD_WIDTH {
            let map_x = (x + self.scx as usize) & 0xFF;
            let tile_col = map_x / 8; let pixel_col = map_x % 8;
            let tile_idx = vram.get(tile_map_base + tile_row * 32 + tile_col).copied().unwrap_or(0);
            let tile_addr = if signed_addressing {
                ((tile_data_base as i32) + (tile_idx as i8 as i32) * 16 + pixel_row as i32 * 2) as usize
            } else {
                tile_data_base + (tile_idx as usize) * 16 + pixel_row * 2
            };
            let lo = vram.get(tile_addr).copied().unwrap_or(0);
            let hi = vram.get(tile_addr + 1).copied().unwrap_or(0);
            let bit = 7 - pixel_col;
            let color = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
            self.framebuffer[ly * LCD_WIDTH + x] = color;
        }
    }
    pub fn read_reg(&self, r: u8) -> u8 {
        match r { 0x40=>self.lcdc, 0x41=>self.stat|0x80, 0x42=>self.scy,
                  0x43=>self.scx, 0x44=>self.ly, 0x45=>self.lyc, _=>0xFF }
    }
    pub fn write_reg(&mut self, r: u8, v: u8) {
        match r { 0x40=>self.lcdc=v, 0x41=>self.stat=(self.stat&0x87)|(v&0x78),
                  0x42=>self.scy=v, 0x43=>self.scx=v, 0x44=>{}, 0x45=>self.lyc=v, _=>{} }
    }
}

// ── APU defined below (Phase 4/5) ──────────────────────────────────────────

// ── Timer ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct Timer {
    pub div: u8, pub tima: u8, pub tma: u8, pub tac: u8,
    div_counter: u16, tima_counter: u32, pub overflow_irq: bool,
}
impl Timer {
    pub fn step(&mut self, cycles: u8) {
        self.overflow_irq = false;
        self.div_counter = self.div_counter.wrapping_add(cycles as u16);
        self.div = (self.div_counter >> 8) as u8;
        if self.tac & 0x04 == 0 { return; }
        let period: u32 = match self.tac & 0x03 { 0=>1024, 1=>16, 2=>64, 3=>256, _=>1024 };
        self.tima_counter += cycles as u32;
        while self.tima_counter >= period {
            self.tima_counter -= period;
            let (t, ov) = self.tima.overflowing_add(1);
            if ov { self.tima = self.tma; self.overflow_irq = true; } else { self.tima = t; }
        }
    }
    pub fn write(&mut self, r: u8, v: u8) {
        match r { 0x04=>{self.div_counter=0;self.div=0;} 0x05=>self.tima=v, 0x06=>self.tma=v, 0x07=>self.tac=v&0x07, _=>{} }
    }
    pub fn read(&self, r: u8) -> u8 {
        match r { 0x04=>self.div, 0x05=>self.tima, 0x06=>self.tma, 0x07=>self.tac, _=>0xFF }
    }
}

// ── Bus ───────────────────────────────────────────────────────────────────────
pub struct Bus {
    pub rom: Vec<u8>, pub ram: Vec<u8>,
    pub vram: [[u8; 0x2000]; 2], pub vram_bank: u8,
    pub wram: [[u8; 0x1000]; 8], pub wram_bank: u8,
    pub hram: [u8; 0x7F], pub oam: [u8; 0xA0],
    pub io: [u8; 0x80], pub ie: u8, pub if_reg: u8,
    pub mbc: Mbc, pub ppu: Ppu, pub apu: Apu, pub timer: Timer,
    pub joypad: u8,
    pub double_speed: bool, pub speed_switch_armed: bool,
    // CGB color palettes: [palette_idx][color_idx*2 | byte_offset] = 64 bytes each
    pub bg_cpal:  [u8; 64], pub bg_cps:  u8,  // BCPS index register
    pub obj_cpal: [u8; 64], pub obj_cps: u8,  // OCPS index register
}
impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        let mbc = Mbc::new(cart.kind.clone());
        Bus { rom: cart.rom, ram: cart.ram, vram: [[0u8;0x2000]; 2], vram_bank: 0,
              wram: [[0u8;0x1000]; 8], wram_bank: 1,
              hram: [0u8;0x7F], oam: [0u8;0xA0], io: [0u8;0x80], ie: 0, if_reg: 0,
              mbc, ppu: Ppu::new(), apu: Apu::default(), timer: Timer::default(), joypad: 0xFF,
              double_speed: false, speed_switch_armed: false,
              bg_cpal: [0xFFu8; 64], bg_cps: 0,
              obj_cpal: [0u8; 64],   obj_cps: 0 }
    }
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => { let m=self.mbc.rom_addr(addr); self.rom.get(m).copied().unwrap_or(0xFF) }
            0x8000..=0x9FFF => self.vram[self.vram_bank as usize][(addr-0x8000) as usize],
            0xA000..=0xBFFF => {
                if self.mbc.ram_enable {
                    if matches!(self.mbc.kind, CartridgeKind::Mbc3) && self.mbc.rtc_sel != 0xFF {
                        self.mbc.rtc_latch[self.mbc.rtc_sel as usize]
                    } else {
                        let off = self.mbc.ram_bank as usize * 0x2000 + (addr-0xA000) as usize;
                        self.ram.get(off).copied().unwrap_or(0xFF)
                    }
                } else { 0xFF }
            }
            0xC000..=0xCFFF => self.wram[0][(addr-0xC000) as usize],
            0xD000..=0xDFFF => self.wram[self.wram_bank as usize][(addr-0xD000) as usize],
            0xE000..=0xEFFF => self.wram[0][(addr-0xE000) as usize],
            0xF000..=0xFDFF => self.wram[self.wram_bank as usize][(addr-0xF000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr-0xFE00) as usize],
            0xFF00 => self.joypad,
            0xFF01..=0xFF03 => self.io[(addr-0xFF00) as usize],
            0xFF04..=0xFF07 => self.timer.read((addr-0xFF00) as u8),
            0xFF0F => self.if_reg,
            0xFF10..=0xFF3F => 0xFF,
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.read_reg((addr-0xFF00) as u8),
            0xFF46 => 0xFF,
            0xFF4D => (if self.double_speed {0x80} else {0}) | (if self.speed_switch_armed {0x01} else {0}),
            0xFF4F => 0xFE | self.vram_bank,
            0xFF68 => self.bg_cps,
            0xFF69 => self.bg_cpal[(self.bg_cps & 0x3F) as usize],
            0xFF6A => self.obj_cps,
            0xFF6B => self.obj_cpal[(self.obj_cps & 0x3F) as usize],
            0xFF70 => 0xF8 | self.wram_bank,
            0xFF80..=0xFFFE => self.hram[(addr-0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }
    pub fn write(&mut self, addr: u16, val: u8) {
        if self.mbc.write(addr, val) { return; }
        match addr {
            0x8000..=0x9FFF => self.vram[self.vram_bank as usize][(addr-0x8000) as usize] = val,
            0xA000..=0xBFFF => {
                if self.mbc.ram_enable {
                    if matches!(self.mbc.kind, CartridgeKind::Mbc3) && self.mbc.rtc_sel != 0xFF {
                        self.mbc.rtc_reg[self.mbc.rtc_sel as usize] = val;
                    } else {
                        let off = self.mbc.ram_bank as usize * 0x2000 + (addr-0xA000) as usize;
                        if off < self.ram.len() { self.ram[off] = val; }
                    }
                }
            }
            0xC000..=0xCFFF => self.wram[0][(addr-0xC000) as usize] = val,
            0xD000..=0xDFFF => self.wram[self.wram_bank as usize][(addr-0xD000) as usize] = val,
            0xFE00..=0xFE9F => self.oam[(addr-0xFE00) as usize] = val,
            0xFF00 => self.joypad = val,
            0xFF04..=0xFF07 => self.timer.write((addr-0xFF00) as u8, val),
            0xFF0F => self.if_reg = val,
            0xFF10..=0xFF3F => self.apu.write_reg((addr-0xFF00) as u8, val),
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.write_reg((addr-0xFF00) as u8, val),
            0xFF4D => self.speed_switch_armed = val & 0x01 != 0,
            0xFF4F => self.vram_bank = val & 0x01,
            0xFF68 => self.bg_cps = val & 0xBF,  // bit 6 reserved
            0xFF69 => {
                let idx = (self.bg_cps & 0x3F) as usize;
                self.bg_cpal[idx] = val;
                if self.bg_cps & 0x80 != 0 { self.bg_cps = (self.bg_cps & 0x80) | ((idx as u8 + 1) & 0x3F); }
            }
            0xFF6A => self.obj_cps = val & 0xBF,
            0xFF6B => {
                let idx = (self.obj_cps & 0x3F) as usize;
                self.obj_cpal[idx] = val;
                if self.obj_cps & 0x80 != 0 { self.obj_cps = (self.obj_cps & 0x80) | ((idx as u8 + 1) & 0x3F); }
            }
            0xFF70 => self.wram_bank = if val & 0x07 == 0 { 1 } else { val & 0x07 },
            0xFF46 => { let src=(val as u16)<<8; for i in 0..0xA0u16 { let b=self.read(src+i); self.oam[i as usize]=b; } }
            0xFF80..=0xFFFE => self.hram[(addr-0xFF80) as usize] = val,
            0xFFFF => self.ie = val,
            _ => {}
        }
    }

    /// Decode a CGB palette entry (2-byte little-endian RGB555) to (r8,g8,b8)
    pub fn cgb_color(cpal: &[u8; 64], palette: u8, color: u8) -> (u8, u8, u8) {
        let idx = (palette as usize) * 8 + (color as usize) * 2;
        let lo = cpal[idx] as u16;
        let hi = cpal[idx + 1] as u16;
        let rgb = lo | (hi << 8);
        let r = ((rgb & 0x001F) as u8) << 3;
        let g = (((rgb >> 5) & 0x001F) as u8) << 3;
        let b = (((rgb >> 10) & 0x001F) as u8) << 3;
        (r, g, b)
    }

    /// Get full BG color palette as RGB888 array [palette][color] = (r,g,b)
    pub fn bg_palette_rgb(&self) -> [[(u8,u8,u8); 4]; 8] {
        let mut out = [[(0u8,0u8,0u8); 4]; 8];
        for p in 0..8 {
            for c in 0..4 {
                out[p][c] = Self::cgb_color(&self.bg_cpal, p as u8, c as u8);
            }
        }
        out
    }

    /// Get full OBJ color palette as RGB888 array [palette][color] = (r,g,b)
    pub fn obj_palette_rgb(&self) -> [[(u8,u8,u8); 4]; 8] {
        let mut out = [[(0u8,0u8,0u8); 4]; 8];
        for p in 0..8 {
            for c in 0..4 {
                out[p][c] = Self::cgb_color(&self.obj_cpal, p as u8, c as u8);
            }
        }
        out
    }

    pub fn step_subsystems(&mut self, cycles: u8) {
        // In double-speed mode CPU runs 2x; PPU/APU/Timer stay at 1x speed
        let sub_cycles = if self.double_speed { (cycles + 1) / 2 } else { cycles };
        let vram = self.vram[self.vram_bank as usize]; let oam = self.oam;
        self.ppu.step(sub_cycles, &vram, &oam);
        if self.ppu.vblank_irq { self.if_reg |= 0x01; }
        if self.ppu.stat_irq   { self.if_reg |= 0x02; }
        self.timer.step(sub_cycles);
        if self.timer.overflow_irq { self.if_reg |= 0x04; }
        self.apu.step_with_fs(sub_cycles);
    }
}


// ── ReplayCapture ──────────────────────────────────────────────────────────────
/// Captures live replay frames from a running GbCore.
/// Feeds both live streaming (state_json) and training record generation.
#[derive(Debug, Default, Clone)]
pub struct ReplayFrame {
    pub frame_idx: u64,
    pub t_cycles:  u64,
    pub pc:        u16,
    pub ly:        u8,
    pub snapshot:  String, // mrom.snap.v1 JSON
}

#[derive(Debug, Default)]
pub struct ReplayCapture {
    pub frames:      Vec<ReplayFrame>,
    pub max_frames:  usize,
    pub rom_title:   String,
}

impl ReplayCapture {
    pub fn new(max_frames: usize, rom_title: &str) -> Self {
        ReplayCapture { frames: Vec::with_capacity(max_frames), max_frames, rom_title: rom_title.to_string() }
    }

    /// Record one frame from a live GbCore. Call after run_frame().
    pub fn capture(&mut self, core: &GbCore) {
        if self.frames.len() >= self.max_frames { return; }
        self.frames.push(ReplayFrame {
            frame_idx: core.clock.frame_count(),
            t_cycles:  core.clock.t_cycles,
            pc:        core.regs.pc,
            ly:        core.bus.ppu.ly,
            snapshot:  core.state_json(),
        });
    }

    /// Export all captured frames as a replay manifest JSON (mrom.replay.v1)
    pub fn to_json(&self) -> String {
        let frames: Vec<String> = self.frames.iter().map(|f|
            format!("{{"fi":{},"tc":{},"pc":{},"snap":{}}}", 
                    f.frame_idx, f.t_cycles, f.pc, f.snapshot)
        ).collect();
        format!(
            "{{"version":"mrom.replay.v1","rom":"{}","frame_count":{},"frames":[{}]}}",
            self.rom_title, self.frames.len(), frames.join(",")
        )
    }

    /// Write replay manifest to file
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_json())
    }
}

// ── Clock ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Default, Clone)]
pub struct Clock { pub t_cycles: u64 }
impl Clock {
    pub fn tick(&mut self, c: u8) { self.t_cycles = self.t_cycles.wrapping_add(c as u64); }
    pub fn frame_count(&self) -> u64 { self.t_cycles / CYCLES_PER_FRAME }
    pub fn current_scanline(&self) -> u32 { ((self.t_cycles % CYCLES_PER_FRAME) / DOTS_PER_LINE as u64) as u32 }
}

// ── Error ─────────────────────────────────────────────────────────────────────
#[derive(Debug)]
pub enum CoreError { InvalidRom(String), Unimplemented(String) }
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::InvalidRom(s) => write!(f, "InvalidRom: {s}"),
            CoreError::Unimplemented(s) => write!(f, "Unimplemented: {s}"),
        }
    }
}
impl std::error::Error for CoreError {}

// ── CB-prefix (full 256-op) ───────────────────────────────────────────────────
fn exec_cb(regs: &mut Registers, bus: &mut Bus) -> u8 {
    let op = bus.read(regs.pc.wrapping_add(1));
    regs.pc = regs.pc.wrapping_add(2);
    let r = op & 0x07;
    let kind = op >> 6;
    let bit_n = (op >> 3) & 0x07;
    let cycles = if r == 6 { 16 } else { 8 };
    let val = match r {
        0=>regs.b, 1=>regs.c, 2=>regs.d, 3=>regs.e,
        4=>regs.h, 5=>regs.l, 6=>bus.read(regs.hl()), 7=>regs.a, _=>unreachable!(),
    };
    let result: Option<u8> = match kind {
        0 => Some(match bit_n {
            0 => { let c=val>>7; let r=(val<<1)|c; regs.set_flag_c(c!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            1 => { let c=val&1; let r=(val>>1)|(c<<7); regs.set_flag_c(c!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            2 => { let oc=if regs.flag_c(){1}else{0}; let nc=val>>7; let r=(val<<1)|oc; regs.set_flag_c(nc!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            3 => { let oc=if regs.flag_c(){0x80}else{0}; let nc=val&1; let r=(val>>1)|oc; regs.set_flag_c(nc!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            4 => { let c=val>>7; let r=val<<1; regs.set_flag_c(c!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            5 => { let c=val&1; let r=(val>>1)|(val&0x80); regs.set_flag_c(c!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            6 => { let r=(val<<4)|(val>>4); regs.set_flag_z(r==0); regs.set_flag_c(false); regs.set_flag_n(false); regs.set_flag_h(false); r }
            7 => { let c=val&1; let r=val>>1; regs.set_flag_c(c!=0); regs.set_flag_z(r==0); regs.set_flag_n(false); regs.set_flag_h(false); r }
            _ => unreachable!()
        }),
        1 => { let b=(val>>bit_n)&1; regs.set_flag_z(b==0); regs.set_flag_n(false); regs.set_flag_h(true); None }
        2 => Some(val & !(1<<bit_n)),
        3 => Some(val | (1<<bit_n)),
        _ => unreachable!(),
    };
    if let Some(res) = result {
        match r {
            0=>regs.b=res, 1=>regs.c=res, 2=>regs.d=res, 3=>regs.e=res,
            4=>regs.h=res, 5=>regs.l=res, 6=>bus.write(regs.hl(),res), 7=>regs.a=res, _=>unreachable!(),
        }
    }
    cycles
}

// ── SM83 decode table ─────────────────────────────────────────────────────────

// ── Sprite ────────────────────────────────────────────────────────────────────
#[derive(Debug, Default, Clone, Copy)]
pub struct Sprite {
    pub y: u8, pub x: u8, pub tile: u8, pub flags: u8,
}
impl Sprite {
    pub fn from_oam(oam: &[u8; 0xA0], idx: usize) -> Self {
        let b = idx * 4;
        Sprite { y: oam[b], x: oam[b+1], tile: oam[b+2], flags: oam[b+3] }
    }
    pub fn screen_y(&self) -> i32 { self.y as i32 - 16 }
    pub fn screen_x(&self) -> i32 { self.x as i32 - 8 }
    pub fn bg_priority(&self) -> bool { self.flags & 0x80 != 0 }
    pub fn y_flip(&self)    -> bool { self.flags & 0x40 != 0 }
    pub fn x_flip(&self)    -> bool { self.flags & 0x20 != 0 }
    pub fn palette(&self)   -> u8   { (self.flags >> 4) & 0x01 }
}

fn apply_palette(pal: u8, c: u8) -> u8 { (pal >> (c * 2)) & 0x03 }

// ── PPU (Phase 4) ─────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuMode { HBlank = 0, VBlank = 1, OamScan = 2, Drawing = 3 }

#[derive(Debug, Clone)]
pub struct Ppu {
    pub mode: PpuMode, pub dot: u32, pub ly: u8, pub lyc: u8,
    pub lcdc: u8, pub stat: u8, pub scy: u8, pub scx: u8,
    pub wy: u8, pub wx: u8, pub wlc: u8,
    pub pal_bg: u8, pub pal_obj0: u8, pub pal_obj1: u8,
    pub framebuffer: Vec<u8>,
    pub frame_ready: bool, pub stat_irq: bool, pub vblank_irq: bool,
}
impl Ppu {
    pub fn new() -> Self {
        Ppu { mode: PpuMode::OamScan, dot: 0, ly: 0, lyc: 0,
               lcdc: 0x91, stat: 0, scy: 0, scx: 0,
               wy: 0, wx: 0, wlc: 0,
               pal_bg: 0xFC, pal_obj0: 0xFF, pal_obj1: 0xFF,
               framebuffer: vec![0u8; LCD_WIDTH * LCD_HEIGHT],
               frame_ready: false, stat_irq: false, vblank_irq: false }
    }
    pub fn step(&mut self, cycles: u8, vram: &[u8; 0x2000], oam: &[u8; 0xA0]) {
        if self.lcdc & 0x80 == 0 { return; }
        self.stat_irq = false; self.vblank_irq = false;
        self.dot += cycles as u32;
        match self.mode {
            PpuMode::OamScan => {
                if self.dot >= PPU_MODE2_CYCLES { self.dot -= PPU_MODE2_CYCLES; self.mode = PpuMode::Drawing; }
            }
            PpuMode::Drawing => {
                if self.dot >= PPU_MODE3_CYCLES {
                    self.dot -= PPU_MODE3_CYCLES;
                    self.render_scanline(vram, oam);
                    self.mode = PpuMode::HBlank;
                    if self.stat & 0x08 != 0 { self.stat_irq = true; }
                }
            }
            PpuMode::HBlank => {
                if self.dot >= PPU_MODE0_CYCLES {
                    self.dot -= PPU_MODE0_CYCLES; self.ly += 1; self.check_lyc();
                    if self.ly >= PPU_VBLANK_LINE as u8 {
                        self.mode = PpuMode::VBlank; self.vblank_irq = true; self.frame_ready = true;
                        if self.stat & 0x10 != 0 { self.stat_irq = true; }
                    } else {
                        self.mode = PpuMode::OamScan;
                        if self.stat & 0x20 != 0 { self.stat_irq = true; }
                    }
                }
            }
            PpuMode::VBlank => {
                if self.dot >= DOTS_PER_LINE {
                    self.dot -= DOTS_PER_LINE; self.ly += 1; self.check_lyc();
                    if self.ly > 153 {
                        self.ly = 0; self.wlc = 0; self.mode = PpuMode::OamScan; self.frame_ready = false;
                        if self.stat & 0x20 != 0 { self.stat_irq = true; }
                    }
                }
            }
        }
        self.stat = (self.stat & 0xFC) | (self.mode as u8);
    }
    fn check_lyc(&mut self) {
        if self.ly == self.lyc { self.stat |= 0x04; if self.stat & 0x40 != 0 { self.stat_irq = true; } }
        else { self.stat &= !0x04; }
    }
    fn render_scanline(&mut self, vram: &[u8; 0x2000], oam: &[u8; 0xA0]) {
        let ly = self.ly as usize;
        if ly >= LCD_HEIGHT { return; }
        let lcdc = self.lcdc;
        let row_base = ly * LCD_WIDTH;
        let mut bg_col = [0u8; LCD_WIDTH];
        let mut bg_opaque = [false; LCD_WIDTH];

        // BG layer
        if lcdc & 0x01 != 0 {
            let map_base: usize  = if lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
            let data_base: usize = if lcdc & 0x10 != 0 { 0x0000 } else { 0x0800 };
            let signed = lcdc & 0x10 == 0;
            let map_y = (ly.wrapping_add(self.scy as usize)) & 0xFF;
            let tile_row = map_y >> 3; let prow = map_y & 7;
            for x in 0..LCD_WIDTH {
                let map_x = (x.wrapping_add(self.scx as usize)) & 0xFF;
                let tc = map_x >> 3; let pc = map_x & 7;
                let idx = vram[map_base + tile_row * 32 + tc];
                let ta = if signed {
                    (data_base as i32 + idx as i8 as i32 * 16 + prow as i32 * 2) as usize
                } else { data_base + idx as usize * 16 + prow * 2 };
                let lo = *vram.get(ta).unwrap_or(&0);
                let hi = *vram.get(ta+1).unwrap_or(&0);
                let bit = 7 - pc;
                let c = ((hi>>bit)&1)<<1 | ((lo>>bit)&1);
                bg_col[x] = apply_palette(self.pal_bg, c);
                bg_opaque[x] = c != 0;
            }
        }

        // Window layer
        let wx7 = self.wx.saturating_sub(7) as usize;
        if lcdc & 0x20 != 0 && ly >= self.wy as usize && wx7 < LCD_WIDTH {
            let wmap: usize  = if lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 };
            let data_base: usize = if lcdc & 0x10 != 0 { 0x0000 } else { 0x0800 };
            let signed = lcdc & 0x10 == 0;
            let wly = self.wlc as usize;
            let tile_row = wly >> 3; let prow = wly & 7;
            for x in wx7..LCD_WIDTH {
                let tc = (x - wx7) >> 3; let pc = (x - wx7) & 7;
                let idx = *vram.get(wmap + tile_row * 32 + tc).unwrap_or(&0);
                let ta = if signed {
                    (data_base as i32 + idx as i8 as i32 * 16 + prow as i32 * 2) as usize
                } else { data_base + idx as usize * 16 + prow * 2 };
                let lo = *vram.get(ta).unwrap_or(&0);
                let hi = *vram.get(ta+1).unwrap_or(&0);
                let bit = 7 - pc;
                let c = ((hi>>bit)&1)<<1 | ((lo>>bit)&1);
                bg_col[x] = apply_palette(self.pal_bg, c);
                bg_opaque[x] = c != 0;
            }
            self.wlc = self.wlc.wrapping_add(1);
        }

        // OAM sprites
        if lcdc & 0x02 != 0 {
            let sh: i32 = if lcdc & 0x04 != 0 { 16 } else { 8 };
            let mut visible: Vec<(i32, Sprite)> = Vec::with_capacity(10);
            for i in 0..40 {
                let s = Sprite::from_oam(oam, i);
                let sy = s.screen_y();
                if (ly as i32) >= sy && (ly as i32) < sy + sh {
                    visible.push((s.screen_x(), s));
                    if visible.len() == 10 { break; }
                }
            }
            visible.sort_by_key(|&(x,_)| x);
            for (_,s) in visible.iter().rev() {
                let sy = s.screen_y();
                let mut row = (ly as i32 - sy) as usize;
                if s.y_flip() { row = (sh as usize) - 1 - row; }
                let tile = if sh == 16 { if row < 8 { s.tile & 0xFE } else { s.tile | 0x01 } } else { s.tile };
                let ta = tile as usize * 16 + (row & 7) * 2;
                let lo = *vram.get(ta).unwrap_or(&0);
                let hi = *vram.get(ta+1).unwrap_or(&0);
                let pal = if s.palette() == 0 { self.pal_obj0 } else { self.pal_obj1 };
                for bi in 0..8usize {
                    let sx = s.screen_x() + bi as i32;
                    if sx < 0 || sx >= LCD_WIDTH as i32 { continue; }
                    let bit = if s.x_flip() { bi } else { 7 - bi };
                    let c = ((hi>>bit)&1)<<1 | ((lo>>bit)&1);
                    if c == 0 { continue; }
                    let px = sx as usize;
                    if s.bg_priority() && bg_opaque[px] { continue; }
                    bg_col[px] = apply_palette(pal, c);
                }
            }
        }

        for x in 0..LCD_WIDTH { self.framebuffer[row_base + x] = bg_col[x]; }
    }
    pub fn read_reg(&self, r: u8) -> u8 {
        match r {
            0x40=>self.lcdc, 0x41=>self.stat|0x80, 0x42=>self.scy, 0x43=>self.scx,
            0x44=>self.ly, 0x45=>self.lyc, 0x47=>self.pal_bg,
            0x48=>self.pal_obj0, 0x49=>self.pal_obj1, 0x4A=>self.wy, 0x4B=>self.wx, _=>0xFF
        }
    }
    pub fn write_reg(&mut self, r: u8, v: u8) {
        match r {
            0x40=>self.lcdc=v, 0x41=>self.stat=(self.stat&0x87)|(v&0x78),
            0x42=>self.scy=v, 0x43=>self.scx=v, 0x44=>{}, 0x45=>self.lyc=v,
            0x47=>self.pal_bg=v, 0x48=>self.pal_obj0=v, 0x49=>self.pal_obj1=v,
            0x4A=>self.wy=v, 0x4B=>self.wx=v, _=>{}
        }
    }
}

// ── APU (Phase 4) ─────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct Square {
    pub nr0: u8, pub nr1: u8, pub nr2: u8, pub nr3: u8, pub nr4: u8,
    pub enabled: bool, pub freq_timer: u32, pub duty_pos: u8,
    pub volume: u8, pub env_timer: u8, pub len_timer: u16,
    pub sweep_shadow: u16, pub sweep_timer: u8, pub sweep_enabled: bool,
}
impl Square {
    fn period(&self) -> u32 {
        let freq = ((self.nr4 as u32 & 0x07) << 8) | self.nr3 as u32;
        (2048 - freq) * 4
    }
    fn duty_hi(&self) -> u8 {
        match (self.nr1 >> 6) & 3 { 0=>0b00000001, 1=>0b10000001, 2=>0b10000111, _=>0b01111110 }
    }
    pub fn tick(&mut self) {
        if self.freq_timer == 0 { self.freq_timer = self.period(); self.duty_pos = (self.duty_pos+1)&7; }
        else { self.freq_timer -= 1; }
    }
    pub fn sample(&self) -> i16 {
        if !self.enabled { return 0; }
        if (self.duty_hi() >> (7-self.duty_pos)) & 1 != 0 { self.volume as i16 * 256 } else { 0 }
    }
    pub fn trigger(&mut self) {
        self.enabled = true; self.freq_timer = self.period();
        self.volume = (self.nr2 >> 4) & 0x0F; self.env_timer = self.nr2 & 0x07;
        self.len_timer = if self.nr1 & 0x3F == 0 { 64 } else { 64 - (self.nr1 as u16 & 0x3F) };
    }
}

#[derive(Debug, Clone, Default)]
pub struct WaveChannel {
    pub enabled: bool, pub nr0: u8, pub nr1: u8, pub nr2: u8, pub nr3: u8, pub nr4: u8,
    pub wave_ram: [u8; 16], pub pos: u8, pub freq_timer: u32,
}
impl WaveChannel {
    pub fn tick(&mut self) {
        if self.freq_timer == 0 {
            let freq = ((self.nr4 as u32 & 0x07) << 8) | self.nr3 as u32;
            self.freq_timer = (2048 - freq) * 2; self.pos = (self.pos+1) & 31;
        } else { self.freq_timer -= 1; }
    }
    pub fn sample(&self) -> i16 {
        if !self.enabled || self.nr0 & 0x80 == 0 { return 0; }
        let byte = self.wave_ram[(self.pos >> 1) as usize];
        let nib = if self.pos & 1 == 0 { byte >> 4 } else { byte & 0x0F };
        let shift = match (self.nr2>>5)&3 { 0=>4, 1=>0, 2=>1, _=>2 };
        ((nib >> shift) as i16) * 512
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoiseChannel {
    pub enabled: bool, pub nr1: u8, pub nr2: u8, pub nr3: u8, pub nr4: u8,
    pub lfsr: u16, pub freq_timer: u32, pub volume: u8, pub env_timer: u8,
}
impl NoiseChannel {
    pub fn tick(&mut self) {
        if self.freq_timer == 0 {
            let r = self.nr3 & 7; let s = (self.nr3 >> 4) & 0x0F;
            let div: u32 = if r==0 {8} else {r as u32 * 16};
            self.freq_timer = div << s;
            let xor = (self.lfsr & 1) ^ ((self.lfsr>>1) & 1);
            self.lfsr = (self.lfsr >> 1) | (xor << 14);
            if self.nr3 & 0x08 != 0 { self.lfsr = (self.lfsr & !0x40) | (xor << 6); }
        } else { self.freq_timer -= 1; }
    }
    pub fn sample(&self) -> i16 {
        if !self.enabled { return 0; }
        if self.lfsr & 1 == 0 { self.volume as i16 * 256 } else { 0 }
    }
    pub fn trigger(&mut self) { self.enabled = true; self.lfsr = 0x7FFF; self.volume = (self.nr2>>4)&0x0F; }
}

#[derive(Debug, Clone)]
pub struct Apu {
    pub power: bool, pub master_vol: u8, pub nr51: u8,
    pub sq1: Square, pub sq2: Square, pub wave: WaveChannel, pub noise: NoiseChannel,
    pub sample_buffer: Vec<i16>,
    sample_timer: u32,
    pub fs_counter: u8, pub wave_len: u16, pub noise_len: u16, pub fs_div: u32,
}
impl Default for Apu {
    fn default() -> Self {
        Apu { power:false, master_vol:0, sq1:Square::default(), sq2:Square::default(),
               wave:WaveChannel::default(), noise:NoiseChannel::default(),
               sample_buffer: Vec::with_capacity(APU_SAMPLES_PER_FRAME * 2),
               sample_timer: (CPU_HZ / APU_SAMPLE_RATE as u64) as u32,
               fs_counter: 0, wave_len: 256, noise_len: 64, fs_div: 0, nr51: 0xFF }
    }
}
impl Apu {
    pub fn step(&mut self, cycles: u8) {
        for _ in 0..cycles {
            self.sq1.tick(); self.sq2.tick(); self.wave.tick(); self.noise.tick();
            if self.sample_timer == 0 {
                self.sample_timer = (CPU_HZ / APU_SAMPLE_RATE as u64) as u32;
                let mix = (self.sq1.sample() + self.sq2.sample() + self.wave.sample() + self.noise.sample()) / 4;
                if self.sample_buffer.len() < APU_SAMPLES_PER_FRAME * 2 {
                    self.sample_buffer.push(mix); self.sample_buffer.push(mix);
                }
            } else { self.sample_timer -= 1; }
        }
    }
    pub fn drain_samples(&mut self) -> Vec<i16> {
        let out = self.sample_buffer.clone(); self.sample_buffer.clear(); out
    }
    pub fn write_reg(&mut self, r: u8, v: u8) {
        match r {
            0x10=>self.sq1.nr0=v, 0x11=>self.sq1.nr1=v, 0x12=>self.sq1.nr2=v,
            0x13=>self.sq1.nr3=v, 0x14=>{ self.sq1.nr4=v; if v&0x80!=0 {self.sq1.trigger();} }
            0x16=>self.sq2.nr1=v, 0x17=>self.sq2.nr2=v, 0x18=>self.sq2.nr3=v,
            0x19=>{ self.sq2.nr4=v; if v&0x80!=0 {self.sq2.trigger();} }
            0x1A=>self.wave.nr0=v, 0x1B=>self.wave.nr1=v, 0x1C=>self.wave.nr2=v,
            0x1D=>self.wave.nr3=v, 0x1E=>{ self.wave.nr4=v; if v&0x80!=0 {self.wave.enabled=true;} }
            0x20=>self.noise.nr1=v, 0x21=>self.noise.nr2=v, 0x22=>self.noise.nr3=v,
            0x23=>{ self.noise.nr4=v; if v&0x80!=0 {self.noise.trigger();} }
            0x24=>self.master_vol=v, 0x25=>self.nr51=v, 0x26=>self.power=v&0x80!=0,
            0x30..=0x3F=>self.wave.wave_ram[(r-0x30) as usize]=v,
            _=>{}
        }
    }
    pub fn step_with_fs(&mut self, cycles: u8) {
        self.step(cycles);
        self.fs_div += cycles as u32;
        while self.fs_div >= 8192 {
            self.fs_div -= 8192;
            self.frame_seq_step();
        }
    }
    pub fn frame_seq_step(&mut self) {
        self.fs_counter = (self.fs_counter + 1) & 7;
        let s = self.fs_counter;
        if s & 1 == 0 { self.clock_len(); }
        if s == 2 || s == 6 { self.clock_sweep(); }
        if s == 7 { self.clock_env(); }
    }
    fn clock_len(&mut self) {
        if self.sq1.nr4&0x40!=0&&self.sq1.len_timer>0{self.sq1.len_timer-=1;if self.sq1.len_timer==0{self.sq1.enabled=false;}}
        if self.sq2.nr4&0x40!=0&&self.sq2.len_timer>0{self.sq2.len_timer-=1;if self.sq2.len_timer==0{self.sq2.enabled=false;}}
        if self.wave.nr4&0x40!=0&&self.wave_len>0{self.wave_len-=1;if self.wave_len==0{self.wave.enabled=false;}}
        if self.noise.nr4&0x40!=0&&self.noise_len>0{self.noise_len-=1;if self.noise_len==0{self.noise.enabled=false;}}
    }
    fn clock_sweep(&mut self) {
        let period=(self.sq1.nr0>>4)&7; let shift=self.sq1.nr0&7;
        if period==0{return;}
        if self.sq1.sweep_timer>0{self.sq1.sweep_timer-=1;}
        if self.sq1.sweep_timer==0{
            self.sq1.sweep_timer=if period!=0{period}else{8};
            if shift!=0&&self.sq1.sweep_enabled{
                let freq=self.sq1.sweep_shadow; let delta=freq>>shift;
                let nf=if self.sq1.nr0&8!=0{freq.wrapping_sub(delta)}else{freq+delta};
                if nf<=2047{self.sq1.sweep_shadow=nf;self.sq1.nr4=(self.sq1.nr4&0xF8)|((nf>>8)as u8&7);self.sq1.nr3=(nf&0xFF)as u8;}
                else{self.sq1.enabled=false;}
            }
        }
    }
    fn clock_env(&mut self) {
        fn tick(v:&mut u8,t:&mut u8,nr2:u8){if *t>0{*t-=1;}if *t==0{let p=nr2&7;*t=if p!=0{p}else{8};if nr2&8!=0{if *v<15{*v+=1;}}else{if *v>0{*v-=1;}}}}
        tick(&mut self.sq1.volume,&mut self.sq1.env_timer,self.sq1.nr2);
        tick(&mut self.sq2.volume,&mut self.sq2.env_timer,self.sq2.nr2);
        tick(&mut self.noise.volume,&mut self.noise.env_timer,self.noise.nr2);
    }
}

// ── Timer ─────────────────────────────────────────────────────────────────────

// ── Timer ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct Timer {
    pub div: u8, pub tima: u8, pub tma: u8, pub tac: u8,
    div_counter: u16, tima_counter: u32, pub overflow_irq: bool,
}
impl Timer {
    pub fn step(&mut self, cycles: u8) {
        self.overflow_irq = false;
        self.div_counter = self.div_counter.wrapping_add(cycles as u16);
        self.div = (self.div_counter >> 8) as u8;
        if self.tac & 0x04 == 0 { return; }
        let period: u32 = match self.tac & 0x03 { 0=>1024, 1=>16, 2=>64, 3=>256, _=>1024 };
        self.tima_counter += cycles as u32;
        while self.tima_counter >= period {
            self.tima_counter -= period;
            let (t, ov) = self.tima.overflowing_add(1);
            if ov { self.tima = self.tma; self.overflow_irq = true; } else { self.tima = t; }
        }
    }
    pub fn write(&mut self, r: u8, v: u8) {
        match r { 0x04=>{self.div_counter=0;self.div=0;} 0x05=>self.tima=v, 0x06=>self.tma=v, 0x07=>self.tac=v&0x07, _=>{} }
    }
    pub fn read(&self, r: u8) -> u8 {
        match r { 0x04=>self.div, 0x05=>self.tima, 0x06=>self.tma, 0x07=>self.tac, _=>0xFF }
    }
}

// ── Bus ───────────────────────────────────────────────────────────────────────

// ── Bus (Phase 4) ─────────────────────────────────────────────────────────────
pub struct Bus {
    pub rom: Vec<u8>, pub ram: Vec<u8>,
    pub vram: [[u8; 0x2000]; 2], pub vram_bank: u8,
    pub wram: [[u8; 0x1000]; 8], pub wram_bank: u8,
    pub hram: [u8; 0x7F], pub oam: [u8; 0xA0],
    pub io: [u8; 0x80], pub ie: u8, pub if_reg: u8,
    pub mbc: Mbc, pub ppu: Ppu, pub apu: Apu, pub timer: Timer,
    pub joypad: u8,
    pub double_speed: bool, pub speed_switch_armed: bool,
    // CGB color palettes: [palette_idx][color_idx*2 | byte_offset] = 64 bytes each
    pub bg_cpal:  [u8; 64], pub bg_cps:  u8,  // BCPS index register
    pub obj_cpal: [u8; 64], pub obj_cps: u8,  // OCPS index register
}
impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        let mbc = Mbc::new(cart.kind.clone());
        Bus { rom: cart.rom, ram: cart.ram, vram: [[0u8;0x2000]; 2], vram_bank: 0,
              wram: [[0u8;0x1000]; 8], wram_bank: 1,
              hram: [0u8;0x7F], oam: [0u8;0xA0], io: [0u8;0x80], ie: 0, if_reg: 0,
              mbc, ppu: Ppu::new(), apu: Apu::default(), timer: Timer::default(), joypad: 0xFF,
              double_speed: false, speed_switch_armed: false,
              bg_cpal: [0xFFu8; 64], bg_cps: 0,
              obj_cpal: [0u8; 64],   obj_cps: 0 }
    }
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => { let m=self.mbc.rom_addr(addr); self.rom.get(m).copied().unwrap_or(0xFF) }
            0x8000..=0x9FFF => self.vram[self.vram_bank as usize][(addr-0x8000) as usize],
            0xA000..=0xBFFF => {
                if self.mbc.ram_enable {
                    let off = self.mbc.ram_bank as usize * 0x2000 + (addr-0xA000) as usize;
                    self.ram.get(off).copied().unwrap_or(0xFF)
                } else { 0xFF }
            }
            0xC000..=0xCFFF => self.wram[0][(addr-0xC000) as usize],
            0xD000..=0xDFFF => self.wram[self.wram_bank as usize][(addr-0xD000) as usize],
            0xE000..=0xEFFF => self.wram[0][(addr-0xE000) as usize],
            0xF000..=0xFDFF => self.wram[self.wram_bank as usize][(addr-0xF000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr-0xFE00) as usize],
            0xFF00 => self.joypad,
            0xFF01..=0xFF03 => self.io[(addr-0xFF00) as usize],
            0xFF04..=0xFF07 => self.timer.read((addr-0xFF00) as u8),
            0xFF0F => self.if_reg,
            0xFF10..=0xFF3F => 0xFF,
            0xFF40..=0xFF4B => self.ppu.read_reg((addr-0xFF00) as u8),
            0xFF46 => 0xFF,
            0xFF80..=0xFFFE => self.hram[(addr-0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }
    pub fn write(&mut self, addr: u16, val: u8) {
        if self.mbc.write(addr, val) { return; }
        match addr {
            0x8000..=0x9FFF => self.vram[self.vram_bank as usize][(addr-0x8000) as usize] = val,
            0xA000..=0xBFFF => {
                if self.mbc.ram_enable {
                    if matches!(self.mbc.kind, CartridgeKind::Mbc3) && self.mbc.rtc_sel != 0xFF {
                        self.mbc.rtc_reg[self.mbc.rtc_sel as usize] = val;
                    } else {
                        let off = self.mbc.ram_bank as usize * 0x2000 + (addr-0xA000) as usize;
                        if off < self.ram.len() { self.ram[off] = val; }
                    }
                }
            }
            0xC000..=0xCFFF => self.wram[0][(addr-0xC000) as usize] = val,
            0xD000..=0xDFFF => self.wram[self.wram_bank as usize][(addr-0xD000) as usize] = val,
            0xFE00..=0xFE9F => self.oam[(addr-0xFE00) as usize] = val,
            0xFF00 => self.joypad = val,
            0xFF04..=0xFF07 => self.timer.write((addr-0xFF00) as u8, val),
            0xFF0F => self.if_reg = val,
            0xFF10..=0xFF3F => self.apu.write_reg((addr-0xFF00) as u8, val),
            0xFF40..=0xFF4B => self.ppu.write_reg((addr-0xFF00) as u8, val),
            0xFF46 => { let src=(val as u16)<<8; for i in 0..0xA0u16 { let b=self.read(src+i); self.oam[i as usize]=b; } }
            0xFF80..=0xFFFE => self.hram[(addr-0xFF80) as usize] = val,
            0xFFFF => self.ie = val,
            _ => {}
        }
    }

    /// Decode a CGB palette entry (2-byte little-endian RGB555) to (r8,g8,b8)
    pub fn cgb_color(cpal: &[u8; 64], palette: u8, color: u8) -> (u8, u8, u8) {
        let idx = (palette as usize) * 8 + (color as usize) * 2;
        let lo = cpal[idx] as u16;
        let hi = cpal[idx + 1] as u16;
        let rgb = lo | (hi << 8);
        let r = ((rgb & 0x001F) as u8) << 3;
        let g = (((rgb >> 5) & 0x001F) as u8) << 3;
        let b = (((rgb >> 10) & 0x001F) as u8) << 3;
        (r, g, b)
    }

    /// Get full BG color palette as RGB888 array [palette][color] = (r,g,b)
    pub fn bg_palette_rgb(&self) -> [[(u8,u8,u8); 4]; 8] {
        let mut out = [[(0u8,0u8,0u8); 4]; 8];
        for p in 0..8 {
            for c in 0..4 {
                out[p][c] = Self::cgb_color(&self.bg_cpal, p as u8, c as u8);
            }
        }
        out
    }

    /// Get full OBJ color palette as RGB888 array [palette][color] = (r,g,b)
    pub fn obj_palette_rgb(&self) -> [[(u8,u8,u8); 4]; 8] {
        let mut out = [[(0u8,0u8,0u8); 4]; 8];
        for p in 0..8 {
            for c in 0..4 {
                out[p][c] = Self::cgb_color(&self.obj_cpal, p as u8, c as u8);
            }
        }
        out
    }

    pub fn step_subsystems(&mut self, cycles: u8) {
        // In double-speed mode CPU runs 2x; PPU/APU/Timer stay at 1x speed
        let sub_cycles = if self.double_speed { (cycles + 1) / 2 } else { cycles };
        let vram = self.vram[self.vram_bank as usize]; let oam = self.oam;
        self.ppu.step(sub_cycles, &vram, &oam);
        if self.ppu.vblank_irq { self.if_reg |= 0x01; }
        if self.ppu.stat_irq   { self.if_reg |= 0x02; }
        self.timer.step(cycles);
        if self.timer.overflow_irq { self.if_reg |= 0x04; }
    }
}


// ── ReplayCapture ──────────────────────────────────────────────────────────────
/// Captures live replay frames from a running GbCore.
/// Feeds both live streaming (state_json) and training record generation.
#[derive(Debug, Default, Clone)]
pub struct ReplayFrame {
    pub frame_idx: u64,
    pub t_cycles:  u64,
    pub pc:        u16,
    pub ly:        u8,
    pub snapshot:  String, // mrom.snap.v1 JSON
}

#[derive(Debug, Default)]
pub struct ReplayCapture {
    pub frames:      Vec<ReplayFrame>,
    pub max_frames:  usize,
    pub rom_title:   String,
}

impl ReplayCapture {
    pub fn new(max_frames: usize, rom_title: &str) -> Self {
        ReplayCapture { frames: Vec::with_capacity(max_frames), max_frames, rom_title: rom_title.to_string() }
    }

    /// Record one frame from a live GbCore. Call after run_frame().
    pub fn capture(&mut self, core: &GbCore) {
        if self.frames.len() >= self.max_frames { return; }
        self.frames.push(ReplayFrame {
            frame_idx: core.clock.frame_count(),
            t_cycles:  core.clock.t_cycles,
            pc:        core.regs.pc,
            ly:        core.bus.ppu.ly,
            snapshot:  core.state_json(),
        });
    }

    /// Export all captured frames as a replay manifest JSON (mrom.replay.v1)
    pub fn to_json(&self) -> String {
        let frames: Vec<String> = self.frames.iter().map(|f|
            format!("{{"fi":{},"tc":{},"pc":{},"snap":{}}}", 
                    f.frame_idx, f.t_cycles, f.pc, f.snapshot)
        ).collect();
        format!(
            "{{"version":"mrom.replay.v1","rom":"{}","frame_count":{},"frames":[{}]}}",
            self.rom_title, self.frames.len(), frames.join(",")
        )
    }

    /// Write replay manifest to file
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_json())
    }
}

// ── Clock ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Default, Clone)]
pub struct Clock { pub t_cycles: u64 }
impl Clock {
    pub fn tick(&mut self, c: u8) { self.t_cycles = self.t_cycles.wrapping_add(c as u64); }
    pub fn frame_count(&self) -> u64 { self.t_cycles / CYCLES_PER_FRAME }
    pub fn current_scanline(&self) -> u32 { ((self.t_cycles % CYCLES_PER_FRAME) / DOTS_PER_LINE as u64) as u32 }
}

// ── Training data ─────────────────────────────────────────────────────────────
fn fnv1a(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for &b in data { h ^= b as u32; h = h.wrapping_mul(0x01000193); }
    h
}

#[derive(Debug, Clone)]
pub struct FrameRecord {
    pub frame: u64, pub t_cycles: u64,
    pub pc: u16, pub sp: u16, pub a: u8, pub f: u8,
    pub bc: u16, pub de: u16, pub hl: u16,
    pub halted: bool, pub ime: bool,
    pub ly: u8, pub lcdc: u8, pub ppu_mode: u8,
    pub vblank_count: u64,
    pub framebuffer: Vec<u8>,
    pub sq1_on: bool, pub sq2_on: bool, pub wave_on: bool, pub noise_on: bool,
    pub rom_bank: u16, pub ram_bank: u8,
    pub wram_hash: u32, pub vram_hash: u32, pub oam_hash: u32,
    pub rom_title: String,
}

// ── CPU decode ────────────────────────────────────────────────────────────────
fn decode(op: u8, bus: &Bus, pc: u16) -> (u8, i16) {
    match op {
        0x00=>(4,1), 0x01|0x11|0x21|0x31=>(12,3), 0x02|0x12|0x0A|0x1A=>(8,1),
        0x03|0x13|0x23|0x33|0x0B|0x1B|0x2B|0x3B=>(8,1),
        0x04|0x05|0x0C|0x0D|0x14|0x15|0x1C|0x1D|0x24|0x25|0x2C|0x2D|0x3C|0x3D=>(4,1),
        0x06|0x0E|0x16|0x1E|0x26|0x2E|0x3E=>(8,2), 0x07|0x0F|0x17|0x1F=>(4,1),
        0x08=>(20,3), 0x09|0x19|0x29|0x39=>(8,1), 0x18=>(12,2),
        0x20|0x28|0x30|0x38=>(8,2), 0x22|0x2A|0x32|0x3A=>(8,1),
        0x34|0x35=>(12,1), 0x36=>(12,2), 0x27|0x2F|0x37|0x3F=>(4,1),
        0x46|0x4E|0x56|0x5E|0x66|0x6E|0x7E=>(8,1), 0x70|0x71|0x72|0x73|0x74|0x75|0x77=>(8,1),
        0x76=>(4,1), 0x40..=0x7F=>(4,1), 0x80..=0xBF=>(4,1),
        0xC6|0xCE|0xD6|0xDE|0xE6|0xEE|0xF6|0xFE=>(8,2),
        0xC0|0xC8|0xD0|0xD8=>(8,1), 0xC1|0xD1|0xE1|0xF1=>(12,1),
        0xC2|0xCA|0xD2|0xDA=>(12,3), 0xC3=>(16,3),
        0xC4|0xCC|0xD4|0xDC=>(12,3), 0xC5|0xD5|0xE5|0xF5=>(16,1),
        0xC7|0xCF|0xD7|0xDF|0xE7|0xEF|0xF7|0xFF=>(16,1),
        0xC9=>(16,1), 0xCB=>{ let _=bus.read(pc.wrapping_add(1)); (8,2) }
        0xCD=>(24,3), 0xD9=>(16,1), 0xE0|0xF0=>(12,2), 0xE2|0xF2=>(8,1),
        0xE8=>(16,2), 0xE9=>(4,1), 0xEA|0xFA=>(16,3), 0xF8=>(12,2),
        0xF9=>(8,1), 0xF3|0xFB=>(4,1), _=>(4,1),
    }
}


// ── SM83 full instruction set (Phase 5) ──────────────────────────────────────
// Called from GbCore::step() in the match op { ... } block.
// Returns cycle count (u8). PC has already been advanced by delta from decode().

fn exec_op(op: u8, regs: &mut Registers, bus: &mut Bus, default_cyc: u8) -> u8 {
    // Helper: read immediate byte after opcode
    macro_rules! imm8 {
        () => {{ let v = bus.read(regs.pc.wrapping_sub(1)); v }}
    }
    // We need the PC *before* decode advanced it. Caller passes pre-exec pc.
    // Instead, use the decode-table delta. Easier: regs.pc was already advanced,
    // so imm8 is at pc-1 (1-byte imm after 1-byte opcode), imm16 lo at pc-2, hi at pc-1.

    let cyc = default_cyc;

    // Read immediates from pre-instruction positions
    // After decode() advanced pc, pc points PAST the instruction.
    // For a 2-byte instr (opcode + imm8): imm8 = bus.read(pc - 1)
    // For a 3-byte instr (opcode + imm16): lo = bus.read(pc - 2), hi = bus.read(pc - 1)
    let pre_pc = regs.pc;   // PC *after* delta advance (next instr)

    // Macros using pre-calc offsets
    let imm8_val: u8 = bus.read(pre_pc.wrapping_sub(1));
    let imm16_val: u16 = {
        let lo = bus.read(pre_pc.wrapping_sub(2)) as u16;
        let hi = bus.read(pre_pc.wrapping_sub(1)) as u16;
        (hi << 8) | lo
    };

    // ── Helper closures ──────────────────────────────────────────────────────
    // ADD A, r
    let add_a = |regs: &mut Registers, val: u8| {
        let a = regs.a; let r = a.wrapping_add(val);
        regs.set_flag_z(r == 0); regs.set_flag_n(false);
        regs.set_flag_h((a & 0x0F) + (val & 0x0F) > 0x0F);
        regs.set_flag_c(a as u16 + val as u16 > 0xFF); regs.a = r;
    };
    let adc_a = |regs: &mut Registers, val: u8| {
        let a = regs.a; let c = if regs.flag_c() { 1u8 } else { 0 };
        let r = a.wrapping_add(val).wrapping_add(c);
        regs.set_flag_z(r == 0); regs.set_flag_n(false);
        regs.set_flag_h((a & 0x0F) + (val & 0x0F) + c > 0x0F);
        regs.set_flag_c(a as u16 + val as u16 + c as u16 > 0xFF); regs.a = r;
    };
    let sub_a = |regs: &mut Registers, val: u8| {
        let a = regs.a; let r = a.wrapping_sub(val);
        regs.set_flag_z(r == 0); regs.set_flag_n(true);
        regs.set_flag_h((a & 0x0F) < (val & 0x0F));
        regs.set_flag_c(a < val); regs.a = r;
    };
    let sbc_a = |regs: &mut Registers, val: u8| {
        let a = regs.a; let c = if regs.flag_c() { 1u8 } else { 0 };
        let r = a.wrapping_sub(val).wrapping_sub(c);
        regs.set_flag_z(r == 0); regs.set_flag_n(true);
        regs.set_flag_h((a & 0x0F) < (val & 0x0F) + c);
        regs.set_flag_c((a as u16) < (val as u16 + c as u16)); regs.a = r;
    };
    let and_a = |regs: &mut Registers, val: u8| {
        regs.a &= val;
        regs.set_flag_z(regs.a == 0); regs.set_flag_n(false);
        regs.set_flag_h(true); regs.set_flag_c(false);
    };
    let xor_a = |regs: &mut Registers, val: u8| {
        regs.a ^= val;
        regs.set_flag_z(regs.a == 0); regs.set_flag_n(false);
        regs.set_flag_h(false); regs.set_flag_c(false);
    };
    let or_a = |regs: &mut Registers, val: u8| {
        regs.a |= val;
        regs.set_flag_z(regs.a == 0); regs.set_flag_n(false);
        regs.set_flag_h(false); regs.set_flag_c(false);
    };
    let cp_a = |regs: &mut Registers, val: u8| {
        let a = regs.a;
        regs.set_flag_z(a == val); regs.set_flag_n(true);
        regs.set_flag_h((a & 0x0F) < (val & 0x0F));
        regs.set_flag_c(a < val);
    };
    let inc8 = |regs: &mut Registers, val: u8| -> u8 {
        let r = val.wrapping_add(1);
        regs.set_flag_z(r == 0); regs.set_flag_n(false);
        regs.set_flag_h((val & 0x0F) == 0x0F); r
    };
    let dec8 = |regs: &mut Registers, val: u8| -> u8 {
        let r = val.wrapping_sub(1);
        regs.set_flag_z(r == 0); regs.set_flag_n(true);
        regs.set_flag_h((val & 0x0F) == 0x00); r
    };
    let add_hl = |regs: &mut Registers, val: u16| {
        let hl = regs.hl(); let r = hl.wrapping_add(val);
        regs.set_flag_n(false);
        regs.set_flag_h((hl & 0x0FFF) + (val & 0x0FFF) > 0x0FFF);
        regs.set_flag_c(hl as u32 + val as u32 > 0xFFFF);
        regs.set_hl(r);
    };

    match op {
        // ── NOP ────────────────────────────────────────────────────────────
        0x00 => {}

        // ── LD r16, d16 ────────────────────────────────────────────────────
        0x01 => regs.set_bc(imm16_val),
        0x11 => regs.set_de(imm16_val),
        0x21 => regs.set_hl(imm16_val),
        0x31 => regs.sp = imm16_val,

        // ── LD (r16), A / LD A, (r16) ──────────────────────────────────────
        0x02 => bus.write(regs.bc(), regs.a),
        0x12 => bus.write(regs.de(), regs.a),
        0x0A => regs.a = bus.read(regs.bc()),
        0x1A => regs.a = bus.read(regs.de()),

        // ── INC/DEC r16 ────────────────────────────────────────────────────
        0x03 => regs.set_bc(regs.bc().wrapping_add(1)),
        0x13 => regs.set_de(regs.de().wrapping_add(1)),
        0x23 => regs.set_hl(regs.hl().wrapping_add(1)),
        0x33 => regs.sp = regs.sp.wrapping_add(1),
        0x0B => regs.set_bc(regs.bc().wrapping_sub(1)),
        0x1B => regs.set_de(regs.de().wrapping_sub(1)),
        0x2B => regs.set_hl(regs.hl().wrapping_sub(1)),
        0x3B => regs.sp = regs.sp.wrapping_sub(1),

        // ── INC r8 ─────────────────────────────────────────────────────────
        0x04 => { let v=inc8(regs,regs.b); regs.b=v; }
        0x0C => { let v=inc8(regs,regs.c); regs.c=v; }
        0x14 => { let v=inc8(regs,regs.d); regs.d=v; }
        0x1C => { let v=inc8(regs,regs.e); regs.e=v; }
        0x24 => { let v=inc8(regs,regs.h); regs.h=v; }
        0x2C => { let v=inc8(regs,regs.l); regs.l=v; }
        0x34 => { let a=regs.hl(); let v=inc8(regs,bus.read(a)); bus.write(a,v); }
        0x3C => { let v=inc8(regs,regs.a); regs.a=v; }

        // ── DEC r8 ─────────────────────────────────────────────────────────
        0x05 => { let v=dec8(regs,regs.b); regs.b=v; }
        0x0D => { let v=dec8(regs,regs.c); regs.c=v; }
        0x15 => { let v=dec8(regs,regs.d); regs.d=v; }
        0x1D => { let v=dec8(regs,regs.e); regs.e=v; }
        0x25 => { let v=dec8(regs,regs.h); regs.h=v; }
        0x2D => { let v=dec8(regs,regs.l); regs.l=v; }
        0x35 => { let a=regs.hl(); let v=dec8(regs,bus.read(a)); bus.write(a,v); }
        0x3D => { let v=dec8(regs,regs.a); regs.a=v; }

        // ── LD r8, d8 ──────────────────────────────────────────────────────
        0x06 => regs.b = imm8_val,
        0x0E => regs.c = imm8_val,
        0x16 => regs.d = imm8_val,
        0x1E => regs.e = imm8_val,
        0x26 => regs.h = imm8_val,
        0x2E => regs.l = imm8_val,
        0x36 => { let a=regs.hl(); bus.write(a, imm8_val); }
        0x3E => regs.a = imm8_val,

        // ── RLCA / RRCA / RLA / RRA ────────────────────────────────────────
        0x07 => { let c=regs.a>>7; regs.a=(regs.a<<1)|c; regs.f=if c!=0{0x10}else{0}; }
        0x0F => { let c=regs.a&1; regs.a=(regs.a>>1)|(c<<7); regs.f=if c!=0{0x10}else{0}; }
        0x17 => { let oc=if regs.flag_c(){1}else{0}; let nc=regs.a>>7; regs.a=(regs.a<<1)|oc; regs.f=if nc!=0{0x10}else{0}; }
        0x1F => { let oc=if regs.flag_c(){0x80}else{0}; let nc=regs.a&1; regs.a=(regs.a>>1)|oc; regs.f=if nc!=0{0x10}else{0}; }

        // ── LD (a16), SP ───────────────────────────────────────────────────
        0x08 => { let a=imm16_val; bus.write(a,regs.sp as u8); bus.write(a.wrapping_add(1),(regs.sp>>8) as u8); }

        // ── ADD HL, r16 ────────────────────────────────────────────────────
        0x09 => { let v=regs.bc(); add_hl(regs,v); }
        0x19 => { let v=regs.de(); add_hl(regs,v); }
        0x29 => { let v=regs.hl(); add_hl(regs,v); }
        0x39 => { let v=regs.sp; add_hl(regs,v); }

        // ── JR e8 (always) ─────────────────────────────────────────────────
        0x18 => { /* PC already advanced; imm was signed offset after opcode */
            let e = imm8_val as i8 as i16;
            regs.pc = regs.pc.wrapping_add_signed(e);
        }

        // ── JR cc, e8 ──────────────────────────────────────────────────────
        // decode gave cyc=8 (not-taken); taken = 12 (handled by returning 12)
        0x20 => { if !regs.flag_z() { let e=imm8_val as i8 as i16; regs.pc=regs.pc.wrapping_add_signed(e); return 12; } }
        0x28 => { if  regs.flag_z() { let e=imm8_val as i8 as i16; regs.pc=regs.pc.wrapping_add_signed(e); return 12; } }
        0x30 => { if !regs.flag_c() { let e=imm8_val as i8 as i16; regs.pc=regs.pc.wrapping_add_signed(e); return 12; } }
        0x38 => { if  regs.flag_c() { let e=imm8_val as i8 as i16; regs.pc=regs.pc.wrapping_add_signed(e); return 12; } }

        // ── LDI / LDD (HL+/-), A and A, (HL+/-) ──────────────────────────
        0x22 => { bus.write(regs.hl(), regs.a); regs.set_hl(regs.hl().wrapping_add(1)); }
        0x2A => { regs.a = bus.read(regs.hl()); regs.set_hl(regs.hl().wrapping_add(1)); }
        0x32 => { bus.write(regs.hl(), regs.a); regs.set_hl(regs.hl().wrapping_sub(1)); }
        0x3A => { regs.a = bus.read(regs.hl()); regs.set_hl(regs.hl().wrapping_sub(1)); }

        // ── DAA ────────────────────────────────────────────────────────────
        0x27 => {
            let mut a = regs.a; let mut adj: u8 = 0;
            let n = regs.flag_n(); let h = regs.flag_h(); let c = regs.flag_c();
            if !n {
                if h || (a & 0x0F) > 9  { adj |= 0x06; }
                if c || a > 0x99         { adj |= 0x60; }
                a = a.wrapping_add(adj);
            } else {
                if h { adj |= 0x06; }
                if c { adj |= 0x60; }
                a = a.wrapping_sub(adj);
            }
            regs.set_flag_z(a == 0); regs.set_flag_h(false);
            if adj & 0x60 != 0 { regs.set_flag_c(true); }
            regs.a = a;
        }

        // ── CPL / SCF / CCF ────────────────────────────────────────────────
        0x2F => { regs.a = !regs.a; regs.set_flag_n(true); regs.set_flag_h(true); }
        0x37 => { regs.set_flag_n(false); regs.set_flag_h(false); regs.set_flag_c(true); }
        0x3F => { let c=regs.flag_c(); regs.set_flag_n(false); regs.set_flag_h(false); regs.set_flag_c(!c); }

        // ── HALT ───────────────────────────────────────────────────────────
        0x76 => {} // handled by caller

        // ── LD r8, r8 (full 8x8 grid 0x40-0x7F minus 0x76) ───────────────
        0x40 => {} // LD B,B  (nop)
        0x41 => regs.b = regs.c,
        0x42 => regs.b = regs.d,
        0x43 => regs.b = regs.e,
        0x44 => regs.b = regs.h,
        0x45 => regs.b = regs.l,
        0x46 => regs.b = bus.read(regs.hl()),
        0x47 => regs.b = regs.a,
        0x48 => regs.c = regs.b,
        0x49 => {} // LD C,C
        0x4A => regs.c = regs.d,
        0x4B => regs.c = regs.e,
        0x4C => regs.c = regs.h,
        0x4D => regs.c = regs.l,
        0x4E => regs.c = bus.read(regs.hl()),
        0x4F => regs.c = regs.a,
        0x50 => regs.d = regs.b,
        0x51 => regs.d = regs.c,
        0x52 => {} // LD D,D
        0x53 => regs.d = regs.e,
        0x54 => regs.d = regs.h,
        0x55 => regs.d = regs.l,
        0x56 => regs.d = bus.read(regs.hl()),
        0x57 => regs.d = regs.a,
        0x58 => regs.e = regs.b,
        0x59 => regs.e = regs.c,
        0x5A => regs.e = regs.d,
        0x5B => {} // LD E,E
        0x5C => regs.e = regs.h,
        0x5D => regs.e = regs.l,
        0x5E => regs.e = bus.read(regs.hl()),
        0x5F => regs.e = regs.a,
        0x60 => regs.h = regs.b,
        0x61 => regs.h = regs.c,
        0x62 => regs.h = regs.d,
        0x63 => regs.h = regs.e,
        0x64 => {} // LD H,H
        0x65 => regs.h = regs.l,
        0x66 => regs.h = bus.read(regs.hl()),
        0x67 => regs.h = regs.a,
        0x68 => regs.l = regs.b,
        0x69 => regs.l = regs.c,
        0x6A => regs.l = regs.d,
        0x6B => regs.l = regs.e,
        0x6C => regs.l = regs.h,
        0x6D => {} // LD L,L
        0x6E => regs.l = bus.read(regs.hl()),
        0x6F => regs.l = regs.a,
        0x70 => bus.write(regs.hl(), regs.b),
        0x71 => bus.write(regs.hl(), regs.c),
        0x72 => bus.write(regs.hl(), regs.d),
        0x73 => bus.write(regs.hl(), regs.e),
        0x74 => bus.write(regs.hl(), regs.h),
        0x75 => bus.write(regs.hl(), regs.l),
        0x77 => bus.write(regs.hl(), regs.a),
        0x78 => regs.a = regs.b,
        0x79 => regs.a = regs.c,
        0x7A => regs.a = regs.d,
        0x7B => regs.a = regs.e,
        0x7C => regs.a = regs.h,
        0x7D => regs.a = regs.l,
        0x7E => regs.a = bus.read(regs.hl()),
        0x7F => {} // LD A,A

        // ── ALU A, r8 ──────────────────────────────────────────────────────
        0x80=>{let v=regs.b; add_a(regs,v);} 0x81=>{let v=regs.c; add_a(regs,v);}
        0x82=>{let v=regs.d; add_a(regs,v);} 0x83=>{let v=regs.e; add_a(regs,v);}
        0x84=>{let v=regs.h; add_a(regs,v);} 0x85=>{let v=regs.l; add_a(regs,v);}
        0x86=>{let v=bus.read(regs.hl()); add_a(regs,v);}
        0x87=>{let v=regs.a; add_a(regs,v);}
        0x88=>{let v=regs.b; adc_a(regs,v);} 0x89=>{let v=regs.c; adc_a(regs,v);}
        0x8A=>{let v=regs.d; adc_a(regs,v);} 0x8B=>{let v=regs.e; adc_a(regs,v);}
        0x8C=>{let v=regs.h; adc_a(regs,v);} 0x8D=>{let v=regs.l; adc_a(regs,v);}
        0x8E=>{let v=bus.read(regs.hl()); adc_a(regs,v);}
        0x8F=>{let v=regs.a; adc_a(regs,v);}
        0x90=>{let v=regs.b; sub_a(regs,v);} 0x91=>{let v=regs.c; sub_a(regs,v);}
        0x92=>{let v=regs.d; sub_a(regs,v);} 0x93=>{let v=regs.e; sub_a(regs,v);}
        0x94=>{let v=regs.h; sub_a(regs,v);} 0x95=>{let v=regs.l; sub_a(regs,v);}
        0x96=>{let v=bus.read(regs.hl()); sub_a(regs,v);}
        0x97=>{let v=regs.a; sub_a(regs,v);}
        0x98=>{let v=regs.b; sbc_a(regs,v);} 0x99=>{let v=regs.c; sbc_a(regs,v);}
        0x9A=>{let v=regs.d; sbc_a(regs,v);} 0x9B=>{let v=regs.e; sbc_a(regs,v);}
        0x9C=>{let v=regs.h; sbc_a(regs,v);} 0x9D=>{let v=regs.l; sbc_a(regs,v);}
        0x9E=>{let v=bus.read(regs.hl()); sbc_a(regs,v);}
        0x9F=>{let v=regs.a; sbc_a(regs,v);}
        0xA0=>{let v=regs.b; and_a(regs,v);} 0xA1=>{let v=regs.c; and_a(regs,v);}
        0xA2=>{let v=regs.d; and_a(regs,v);} 0xA3=>{let v=regs.e; and_a(regs,v);}
        0xA4=>{let v=regs.h; and_a(regs,v);} 0xA5=>{let v=regs.l; and_a(regs,v);}
        0xA6=>{let v=bus.read(regs.hl()); and_a(regs,v);}
        0xA7=>{let v=regs.a; and_a(regs,v);}
        0xA8=>{let v=regs.b; xor_a(regs,v);} 0xA9=>{let v=regs.c; xor_a(regs,v);}
        0xAA=>{let v=regs.d; xor_a(regs,v);} 0xAB=>{let v=regs.e; xor_a(regs,v);}
        0xAC=>{let v=regs.h; xor_a(regs,v);} 0xAD=>{let v=regs.l; xor_a(regs,v);}
        0xAE=>{let v=bus.read(regs.hl()); xor_a(regs,v);}
        0xAF=>{let v=regs.a; xor_a(regs,v);}
        0xB0=>{let v=regs.b; or_a(regs,v);} 0xB1=>{let v=regs.c; or_a(regs,v);}
        0xB2=>{let v=regs.d; or_a(regs,v);} 0xB3=>{let v=regs.e; or_a(regs,v);}
        0xB4=>{let v=regs.h; or_a(regs,v);} 0xB5=>{let v=regs.l; or_a(regs,v);}
        0xB6=>{let v=bus.read(regs.hl()); or_a(regs,v);}
        0xB7=>{let v=regs.a; or_a(regs,v);}
        0xB8=>{let v=regs.b; cp_a(regs,v);} 0xB9=>{let v=regs.c; cp_a(regs,v);}
        0xBA=>{let v=regs.d; cp_a(regs,v);} 0xBB=>{let v=regs.e; cp_a(regs,v);}
        0xBC=>{let v=regs.h; cp_a(regs,v);} 0xBD=>{let v=regs.l; cp_a(regs,v);}
        0xBE=>{let v=bus.read(regs.hl()); cp_a(regs,v);}
        0xBF=>{let v=regs.a; cp_a(regs,v);}

        // ── ALU A, d8 ──────────────────────────────────────────────────────
        0xC6 => add_a(regs, imm8_val),
        0xCE => adc_a(regs, imm8_val),
        0xD6 => sub_a(regs, imm8_val),
        0xDE => sbc_a(regs, imm8_val),
        0xE6 => and_a(regs, imm8_val),
        0xEE => xor_a(regs, imm8_val),
        0xF6 => or_a(regs, imm8_val),
        0xFE => cp_a(regs, imm8_val),

        // ── RET cc ─────────────────────────────────────────────────────────
        0xC0 => { if !regs.flag_z() { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.pc=(hi<<8)|lo; return 20; } }
        0xC8 => { if  regs.flag_z() { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.pc=(hi<<8)|lo; return 20; } }
        0xD0 => { if !regs.flag_c() { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.pc=(hi<<8)|lo; return 20; } }
        0xD8 => { if  regs.flag_c() { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.pc=(hi<<8)|lo; return 20; } }

        // ── POP r16 ────────────────────────────────────────────────────────
        0xC1 => { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.set_bc((hi<<8)|lo); }
        0xD1 => { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.set_de((hi<<8)|lo); }
        0xE1 => { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.set_hl((hi<<8)|lo); }
        0xF1 => { let lo=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); let hi=bus.read(regs.sp) as u16; regs.sp=regs.sp.wrapping_add(1); regs.set_af((hi<<8)|(lo&0xF0)); }

        // ── JP cc, a16 ─────────────────────────────────────────────────────
        0xC2 => { if !regs.flag_z() { regs.pc=imm16_val; return 16; } }
        0xCA => { if  regs.flag_z() { regs.pc=imm16_val; return 16; } }
        0xD2 => { if !regs.flag_c() { regs.pc=imm16_val; return 16; } }
        0xDA => { if  regs.flag_c() { regs.pc=imm16_val; return 16; } }

        // ── JP a16 / JP HL already handled in caller ───────────────────────
        0xC3 | 0xE9 => {} // handled by step()
        0xCD | 0xC9 | 0xD9 => {} // CALL/RET handled by step()

        // ── PUSH r16 ───────────────────────────────────────────────────────
        0xC5 => { let v=regs.bc(); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,(v>>8)as u8); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,v as u8); }
        0xD5 => { let v=regs.de(); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,(v>>8)as u8); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,v as u8); }
        0xE5 => { let v=regs.hl(); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,(v>>8)as u8); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,v as u8); }
        0xF5 => { let v=regs.af(); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,(v>>8)as u8); regs.sp=regs.sp.wrapping_sub(1); bus.write(regs.sp,(v&0xF0) as u8); }

        // ── CALL cc, a16 ───────────────────────────────────────────────────
        0xC4 => { if !regs.flag_z() { push_call(regs,bus,imm16_val); return 24; } }
        0xCC => { if  regs.flag_z() { push_call(regs,bus,imm16_val); return 24; } }
        0xD4 => { if !regs.flag_c() { push_call(regs,bus,imm16_val); return 24; } }
        0xDC => { if  regs.flag_c() { push_call(regs,bus,imm16_val); return 24; } }

        // ── RST nn ─────────────────────────────────────────────────────────
        0xC7 => push_call(regs,bus,0x0000), 0xCF => push_call(regs,bus,0x0008),
        0xD7 => push_call(regs,bus,0x0010), 0xDF => push_call(regs,bus,0x0018),
        0xE7 => push_call(regs,bus,0x0020), 0xEF => push_call(regs,bus,0x0028),
        0xF7 => push_call(regs,bus,0x0030), 0xFF => push_call(regs,bus,0x0038),

        // ── LDH (a8), A / LDH A, (a8) ─────────────────────────────────────
        0xE0 => bus.write(0xFF00 | imm8_val as u16, regs.a),
        0xF0 => regs.a = bus.read(0xFF00 | imm8_val as u16),

        // ── LD (C), A / LD A, (C) ──────────────────────────────────────────
        0xE2 => bus.write(0xFF00 | regs.c as u16, regs.a),
        0xF2 => regs.a = bus.read(0xFF00 | regs.c as u16),

        // ── ADD SP, e8 ─────────────────────────────────────────────────────
        0xE8 => {
            let e = imm8_val as i8 as i32; let sp = regs.sp as i32;
            let r = sp.wrapping_add(e) as u16;
            regs.set_flag_z(false); regs.set_flag_n(false);
            regs.set_flag_h(((sp ^ e ^ r as i32) & 0x10) != 0);
            regs.set_flag_c(((sp ^ e ^ r as i32) & 0x100) != 0);
            regs.sp = r;
        }

        // ── LD (a16), A / LD A, (a16) ──────────────────────────────────────
        0xEA => bus.write(imm16_val, regs.a),
        0xFA => regs.a = bus.read(imm16_val),

        // ── LD HL, SP+e8 ───────────────────────────────────────────────────
        0xF8 => {
            let e = imm8_val as i8 as i32; let sp = regs.sp as i32;
            let r = sp.wrapping_add(e) as u16;
            regs.set_flag_z(false); regs.set_flag_n(false);
            regs.set_flag_h(((sp ^ e ^ r as i32) & 0x10) != 0);
            regs.set_flag_c(((sp ^ e ^ r as i32) & 0x100) != 0);
            regs.set_hl(r);
        }

        // ── LD SP, HL ──────────────────────────────────────────────────────
        0xF9 => regs.sp = regs.hl(),

        // ── DI / EI handled in caller ──────────────────────────────────────
        0xF3 | 0xFB => {}

        // ── CB prefix handled in caller ────────────────────────────────────
        0xCB => {}

        // ── Illegal / unused ───────────────────────────────────────────────
        _ => {}
    }
    cyc
}

// Push PC (already past instruction) and jump — used by CALL and conditional CALL
#[inline(always)]
fn push_call(regs: &mut Registers, bus: &mut Bus, target: u16) {
    regs.sp = regs.sp.wrapping_sub(1); bus.write(regs.sp, (regs.pc >> 8) as u8);
    regs.sp = regs.sp.wrapping_sub(1); bus.write(regs.sp, regs.pc as u8);
    regs.pc = target;
}


// ── GbCore ────────────────────────────────────────────────────────────────────
pub struct GbCore {
    pub regs: Registers, pub bus: Bus, pub clock: Clock,
    pub halted: bool, pub ime: bool, pub ime_pending: bool,
}
impl GbCore {
    pub fn new(cart: Cartridge) -> Self {
        let mut regs = Registers::default();
        regs.set_af(0x01B0); regs.set_bc(0x0013); regs.set_de(0x00D8); regs.set_hl(0x014D);
        regs.sp = 0xFFFE; regs.pc = 0x0100;
        GbCore { regs, bus: Bus::new(cart), clock: Clock::default(), halted: false, ime: false, ime_pending: false }
    }
    pub fn step(&mut self) -> Result<u8, CoreError> {
        if self.halted {
            self.bus.step_subsystems(4); self.clock.tick(4);
            if self.bus.if_reg & self.bus.ie & 0x1F != 0 { self.halted = false; }
            return Ok(4);
        }
        if self.ime_pending { self.ime = true; self.ime_pending = false; }
        if self.ime {
            let pending = self.bus.if_reg & self.bus.ie & 0x1F;
            if pending != 0 {
                self.ime = false;
                let bit = pending.trailing_zeros() as u8;
                self.bus.if_reg &= !(1<<bit);
                let vec: u16 = 0x0040 + (bit as u16) * 8;
                self.regs.sp = self.regs.sp.wrapping_sub(1);
                self.bus.write(self.regs.sp, (self.regs.pc>>8) as u8);
                self.regs.sp = self.regs.sp.wrapping_sub(1);
                self.bus.write(self.regs.sp, self.regs.pc as u8);
                self.regs.pc = vec;
                self.bus.step_subsystems(20); self.clock.tick(20);
                return Ok(20);
            }
        }
        let op = self.bus.read(self.regs.pc);
        // Phase 5: full SM83 instruction set via exec_op
        let cycles = if op == 0xCB {
            exec_cb(&mut self.regs, &mut self.bus)
        } else {
            // Decode: get cycle count + PC delta, advance PC
            let (cyc, delta) = decode(op, &self.bus, self.regs.pc);
            self.regs.pc = self.regs.pc.wrapping_add(delta as u16);
            // Execute instruction (exec_op reads immediates relative to advanced PC)
            let actual_cyc = exec_op(op, &mut self.regs, &mut self.bus, cyc as u8);
            // Handle ops that exec_op defers back to step()
            match op {
                0x76 => { self.regs.pc = self.regs.pc.wrapping_sub(delta as u16); self.halted = true; }
                0x10 => {
                    // STOP: execute CGB double-speed switch if armed
                    if self.bus.speed_switch_armed {
                        self.bus.double_speed = !self.bus.double_speed;
                        self.bus.speed_switch_armed = false;
                    }
                    // Consume STOP's second byte (always 0x00)
                }
                0xF3 => { self.ime = false; }
                0xFB => { self.ime_pending = true; }
                _ => {}
            }
            actual_cyc
        };
        self.bus.step_subsystems(cycles);
        self.clock.tick(cycles);
        Ok(cycles)
    }
    pub fn run_frame(&mut self) -> Result<(), CoreError> {
        let target = self.clock.t_cycles + CYCLES_PER_FRAME;
        while self.clock.t_cycles < target { self.step()?; }
        Ok(())
    }
    pub fn frame_to_ascii(&self) -> String {
        let palette = ['.', '+', '#', '@'];
        let fb = &self.bus.ppu.framebuffer;
        let mut out = String::with_capacity((LCD_WIDTH+1) * (LCD_HEIGHT/2));
        for y in (0..LCD_HEIGHT).step_by(2) {
            for x in 0..LCD_WIDTH { out.push(palette[fb[y*LCD_WIDTH+x].min(3) as usize]); }
            out.push('\n');
        }
        out
    }

    /// Serialize current emulator state to .mrom.sav JSON bytes
    pub fn save_state(&self) -> Vec<u8> {
        let cpu = format!(
            "{{"pc":{},"sp":{},"a":{},"f":{},"b":{},"c":{},"d":{},"e":{},"h":{},"l":{},"halted":{},"ime":{}}}",
            self.regs.pc, self.regs.sp, self.regs.a, self.regs.f,
            self.regs.b, self.regs.c, self.regs.d, self.regs.e, self.regs.h, self.regs.l,
            self.halted, self.ime
        );
        let t = self.clock.t_cycles;
        // Compact hex dump helpers
        let wram_hex: String = self.bus.wram.iter().flat_map(|bank| bank.iter()).map(|b| format!("{:02x}",b)).collect();
        let hram_hex: String = self.bus.hram.iter().map(|b| format!("{:02x}",b)).collect();
        let oam_hex:  String = self.bus.oam.iter().map(|b| format!("{:02x}",b)).collect();
        let v0_hex:   String = self.bus.vram[0].iter().map(|b| format!("{:02x}",b)).collect();
        let v1_hex:   String = self.bus.vram[1].iter().map(|b| format!("{:02x}",b)).collect();
        let json = format!(
            concat!(
                "{{"version":"mrom.sav.v1",",
                ""t_cycles":{t},",
                ""cpu":{cpu},",
                ""rom_bank":{rom_bank},"ram_bank":{ram_bank},",
                ""vram_bank":{vb},"double_speed":{ds},",
                ""wram":"{wram}","hram":"{hram}","oam":"{oam}",",
                ""vram0":"{v0}","vram1":"{v1}"}}"
            ),
            t=t, cpu=cpu,
            rom_bank=self.bus.mbc.rom_bank, ram_bank=self.bus.mbc.ram_bank,
            vb=self.bus.vram_bank, ds=self.bus.double_speed,
            wram=wram_hex, hram=hram_hex, oam=oam_hex, v0=v0_hex, v1=v1_hex
        );
        json.into_bytes()
    }


    /// Load emulator state from mrom.sav.v1 JSON bytes (from save_state())
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        let s = std::str::from_utf8(data)
            .map_err(|e| CoreError::InvalidRom(format!("load_state: utf8 error: {e}")))?;

        fn parse_u64(s: &str, key: &str) -> Option<u64> {
            let k = format!("\"{}\": ", key); // "key": 
            let k2 = format!("\"{}\",", key); // tolerate both
            let pos = s.find(&format!("\"{}\":", key))?;
            let rest = &s[pos + key.len() + 3..];
            let end = rest.find(|c: char| !c.is_ascii_digit())?;
            rest[..end].parse().ok()
        }
        fn parse_bool(s: &str, key: &str) -> Option<bool> {
            let pos = s.find(&format!("\"{}\":", key))?;
            let rest = &s[pos + key.len() + 3..].trim_start_matches(' ');
            Some(rest.starts_with("true"))
        }
        fn parse_hex(s: &str, key: &str) -> Option<Vec<u8>> {
            let pos = s.find(&format!("\"{}\":\"", key))?;
            let rest = &s[pos + key.len() + 4..];
            let end = rest.find('"')?;
            let hex = &rest[..end];
            let bytes: Option<Vec<u8>> = (0..hex.len()/2)
                .map(|i| u8::from_str_radix(&hex[i*2..i*2+2], 16).ok())
                .collect();
            bytes
        }

        // CPU registers from "cpu" sub-object
        let cpu_start = s.find("\"cpu\":{").map(|i| i + 7).unwrap_or(0);
        let cpu_str = if cpu_start > 0 {
            let end = s[cpu_start..].find('}').map(|i| cpu_start + i + 1).unwrap_or(s.len());
            &s[cpu_start..end]
        } else { s };

        macro_rules! pu8 {
            ($k:expr) => { parse_u64(cpu_str, $k).unwrap_or(0) as u8 }
        }
        macro_rules! pu16 {
            ($k:expr) => { parse_u64(cpu_str, $k).unwrap_or(0) as u16 }
        }

        self.regs.a  = pu8!("a");
        self.regs.f  = pu8!("f");
        self.regs.b  = pu8!("b");
        self.regs.c  = pu8!("c");
        self.regs.d  = pu8!("d");
        self.regs.e  = pu8!("e");
        self.regs.h  = pu8!("h");
        self.regs.l  = pu8!("l");
        self.regs.sp = pu16!("sp");
        self.regs.pc = pu16!("pc");
        self.halted  = parse_bool(cpu_str, "halted").unwrap_or(false);
        self.ime     = parse_bool(cpu_str, "ime").unwrap_or(false);

        // Top-level fields
        if let Some(t) = parse_u64(s, "t_cycles") { self.clock.t_cycles = t; }
        if let Some(rb) = parse_u64(s, "rom_bank") { self.bus.mbc.rom_bank = rb as u16; }
        if let Some(rb) = parse_u64(s, "ram_bank") { self.bus.mbc.ram_bank = rb as u8; }
        if let Some(vb) = parse_u64(s, "vram_bank") { self.bus.vram_bank = vb as u8; }
        if let Some(ds) = parse_bool(s, "double_speed") { self.bus.double_speed = ds; }

        // Memory banks
        if let Some(wram_bytes) = parse_hex(s, "wram") {
            for (i, b) in wram_bytes.iter().enumerate() {
                let bank = i / 0x1000;
                let off  = i % 0x1000;
                if bank < 8 { self.bus.wram[bank][off] = *b; }
            }
        }
        if let Some(hram_bytes) = parse_hex(s, "hram") {
            for (i, b) in hram_bytes.iter().enumerate() {
                if i < self.bus.hram.len() { self.bus.hram[i] = *b; }
            }
        }
        if let Some(oam_bytes) = parse_hex(s, "oam") {
            for (i, b) in oam_bytes.iter().enumerate() {
                if i < self.bus.oam.len() { self.bus.oam[i] = *b; }
            }
        }
        if let Some(v0) = parse_hex(s, "vram0") {
            for (i, b) in v0.iter().enumerate() {
                if i < 0x2000 { self.bus.vram[0][i] = *b; }
            }
        }
        if let Some(v1) = parse_hex(s, "vram1") {
            for (i, b) in v1.iter().enumerate() {
                if i < 0x2000 { self.bus.vram[1][i] = *b; }
            }
        }

        Ok(())
    }

    /// Load save state from file
    pub fn load_state_from_file(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let data = std::fs::read(path)?;
        self.load_state(&data).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Write save state to file at `path`
    pub fn save_state_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.save_state())
    }

    /// Get framebuffer as RGB888 bytes [r,g,b, r,g,b, ...] — 160×144×3 = 69,120 bytes
    /// For DMG (non-CGB): maps 2-bit palette values to greyscale
    /// For CGB: uses bg_cpal with direct palette index from tile attributes
    /// (Phase 7 approximation: maps 2-bit value through BG palette 0)
    pub fn framebuffer_rgb(&self) -> Vec<u8> {
        let fb = &self.bus.ppu.framebuffer;
        let is_cgb = self.bus.bg_cpal != [0xFFu8; 64];
        let mut out = Vec::with_capacity(LCD_WIDTH * LCD_HEIGHT * 3);

        for &px in fb.iter() {
            let (r, g, b) = if is_cgb {
                // Use CGB BG palette 0, color index = pixel value
                Bus::cgb_color(&self.bus.bg_cpal, 0, px.min(3))
            } else {
                // DMG greyscale
                let shade = match px.min(3) { 0 => 255, 1 => 170, 2 => 85, _ => 0 };
                (shade, shade, shade)
            };
            out.push(r); out.push(g); out.push(b);
        }
        out
    }

    /// Encode framebuffer as compact base64-like hex string for JSON embedding
    pub fn framebuffer_hex(&self) -> String {
        self.framebuffer_rgb().iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Full JSON snapshot for network broadcast / live replay
    /// Format: mrom.snap.v1 — lightweight, designed for WebSocket streaming
    pub fn state_json(&self) -> String {
        let fb_hex = self.framebuffer_hex();
        let cpu = format!(
            "{{"pc":{},"sp":{},"a":{},"f":{}}}",
            self.regs.pc, self.regs.sp, self.regs.a, self.regs.f
        );
        let bg_pal: String = self.bus.bg_cpal.iter().map(|b| format!("{:02x}",b)).collect();
        format!(
            concat!(
                "{{"v":"mrom.snap.v1",",
                ""f":{frame},"ly":{ly},"mode":{mode},",
                ""cpu":{cpu},",
                ""ds":{ds},"wb":{wb},"vb":{vb},",
                ""bg_pal":"{bg_pal}",",
                ""fb":"{fb}"}}"
            ),
            frame = self.clock.frame_count(),
            ly = self.bus.ppu.ly,
            mode = self.bus.ppu.mode as u8,
            cpu = cpu,
            ds = self.bus.double_speed,
            wb = self.bus.wram_bank,
            vb = self.bus.vram_bank,
            bg_pal = bg_pal,
            fb = fb_hex,
        )
    }

    pub fn state_summary(&self) -> String {
        format!(
            "PC={:#06x} SP={:#06x} A={:#04x} BC={:#06x} DE={:#06x} HL={:#06x} | Frame={} LY={} Mode={:?} | T={}",
            self.regs.pc, self.regs.sp, self.regs.a,
            self.regs.bc(), self.regs.de(), self.regs.hl(),
            self.clock.frame_count(), self.bus.ppu.ly, self.bus.ppu.mode,
            self.clock.t_cycles,
        )
    }
}