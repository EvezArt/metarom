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
}

impl Mbc {
    pub fn new(kind: CartridgeKind) -> Self {
        Mbc { kind, rom_bank: 1, ram_bank: 0, ram_enable: false, mode: 0, upper_bits: 0 }
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
                    0x4000..=0x5FFF => { self.ram_bank = val & 0x07; true }
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

// ── APU stub ──────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct Apu { pub power: bool, pub master_vol: u8 }
impl Apu {
    pub fn write_reg(&mut self, r: u8, v: u8) {
        match r { 0x24=>self.master_vol=v, 0x26=>self.power=v&0x80!=0, _=>{} }
    }
}

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
    pub vram: [u8; 0x2000], pub wram: [u8; 0x2000],
    pub hram: [u8; 0x7F], pub oam: [u8; 0xA0],
    pub io: [u8; 0x80], pub ie: u8, pub if_reg: u8,
    pub mbc: Mbc, pub ppu: Ppu, pub apu: Apu, pub timer: Timer,
    pub joypad: u8,
}
impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        let mbc = Mbc::new(cart.kind.clone());
        Bus { rom: cart.rom, ram: cart.ram, vram: [0u8;0x2000], wram: [0u8;0x2000],
              hram: [0u8;0x7F], oam: [0u8;0xA0], io: [0u8;0x80], ie: 0, if_reg: 0,
              mbc, ppu: Ppu::new(), apu: Apu::default(), timer: Timer::default(), joypad: 0xFF }
    }
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => { let m=self.mbc.rom_addr(addr); self.rom.get(m).copied().unwrap_or(0xFF) }
            0x8000..=0x9FFF => self.vram[(addr-0x8000) as usize],
            0xA000..=0xBFFF => {
                if self.mbc.ram_enable {
                    let off = self.mbc.ram_bank as usize * 0x2000 + (addr-0xA000) as usize;
                    self.ram.get(off).copied().unwrap_or(0xFF)
                } else { 0xFF }
            }
            0xC000..=0xDFFF => self.wram[(addr-0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr-0xE000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr-0xFE00) as usize],
            0xFF00 => self.joypad,
            0xFF01..=0xFF03 => self.io[(addr-0xFF00) as usize],
            0xFF04..=0xFF07 => self.timer.read((addr-0xFF00) as u8),
            0xFF0F => self.if_reg,
            0xFF10..=0xFF3F => 0xFF,
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.read_reg((addr-0xFF00) as u8),
            0xFF46 => 0xFF,
            0xFF80..=0xFFFE => self.hram[(addr-0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }
    pub fn write(&mut self, addr: u16, val: u8) {
        if self.mbc.write(addr, val) { return; }
        match addr {
            0x8000..=0x9FFF => self.vram[(addr-0x8000) as usize] = val,
            0xA000..=0xBFFF => {
                if self.mbc.ram_enable {
                    let off = self.mbc.ram_bank as usize * 0x2000 + (addr-0xA000) as usize;
                    if off < self.ram.len() { self.ram[off] = val; }
                }
            }
            0xC000..=0xDFFF => self.wram[(addr-0xC000) as usize] = val,
            0xFE00..=0xFE9F => self.oam[(addr-0xFE00) as usize] = val,
            0xFF00 => self.joypad = val,
            0xFF04..=0xFF07 => self.timer.write((addr-0xFF00) as u8, val),
            0xFF0F => self.if_reg = val,
            0xFF10..=0xFF3F => self.apu.write_reg((addr-0xFF00) as u8, val),
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.write_reg((addr-0xFF00) as u8, val),
            0xFF46 => { let src=(val as u16)<<8; for i in 0..0xA0u16 { let b=self.read(src+i); self.oam[i as usize]=b; } }
            0xFF80..=0xFFFE => self.hram[(addr-0xFF80) as usize] = val,
            0xFFFF => self.ie = val,
            _ => {}
        }
    }
    pub fn step_subsystems(&mut self, cycles: u8) {
        let vram = self.vram; let oam = self.oam;
        self.ppu.step(cycles, &vram, &oam);
        if self.ppu.vblank_irq { self.if_reg |= 0x01; }
        if self.ppu.stat_irq   { self.if_reg |= 0x02; }
        self.timer.step(cycles);
        if self.timer.overflow_irq { self.if_reg |= 0x04; }
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
#[inline(always)]
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
        let cycles = if op == 0xCB {
            exec_cb(&mut self.regs, &mut self.bus)
        } else {
            let (cyc, delta) = decode(op, &self.bus, self.regs.pc);
            match op {
                0xC3 => { let lo=self.bus.read(self.regs.pc.wrapping_add(1)) as u16; let hi=self.bus.read(self.regs.pc.wrapping_add(2)) as u16; self.regs.pc=(hi<<8)|lo; }
                0xE9 => { self.regs.pc = self.regs.hl(); }
                0xCD => {
                    let lo=self.bus.read(self.regs.pc.wrapping_add(1)) as u16;
                    let hi=self.bus.read(self.regs.pc.wrapping_add(2)) as u16;
                    let ret=self.regs.pc.wrapping_add(3);
                    self.regs.sp=self.regs.sp.wrapping_sub(1); self.bus.write(self.regs.sp,(ret>>8)as u8);
                    self.regs.sp=self.regs.sp.wrapping_sub(1); self.bus.write(self.regs.sp,ret as u8);
                    self.regs.pc=(hi<<8)|lo;
                }
                0xC9|0xD9 => {
                    let lo=self.bus.read(self.regs.sp) as u16; self.regs.sp=self.regs.sp.wrapping_add(1);
                    let hi=self.bus.read(self.regs.sp) as u16; self.regs.sp=self.regs.sp.wrapping_add(1);
                    self.regs.pc=(hi<<8)|lo; if op==0xD9 { self.ime=true; }
                }
                0x76 => { self.halted=true; self.regs.pc=self.regs.pc.wrapping_add(1); }
                0xF3 => { self.ime=false; self.regs.pc=self.regs.pc.wrapping_add(delta as u16); }
                0xFB => { self.ime_pending=true; self.regs.pc=self.regs.pc.wrapping_add(delta as u16); }
                _ => { self.regs.pc=self.regs.pc.wrapping_add(delta as u16); }
            }
            cyc
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