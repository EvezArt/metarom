//! gb-core smoke tests

#[cfg(test)]
mod tests {
    use gb_core::{Cartridge, GbCore, CPU_HZ, CYCLES_PER_FRAME};

    fn minimal_rom() -> Vec<u8> {
        let mut rom = vec![0x00u8; 32 * 1024];
        rom[0x100] = 0x00;
        rom[0x101] = 0xC3; rom[0x102] = 0x50; rom[0x103] = 0x01;
        for (i, b) in b"GBCORE_TEST".iter().enumerate() { rom[0x134 + i] = *b; }
        rom[0x147] = 0x00; rom[0x148] = 0x00; rom[0x149] = 0x00;
        rom
    }

    #[test]
    fn cartridge_parse() {
        let cart = Cartridge::from_bytes(minimal_rom()).unwrap();
        assert_eq!(cart.title, "GBCORE_TEST");
        assert_eq!(cart.rom_size_kb, 32);
    }

    #[test]
    fn clock_frame_model() {
        assert_eq!(CPU_HZ, 4_194_304);
        assert_eq!(CYCLES_PER_FRAME, 70224);
    }

    #[test]
    fn core_step_advances_pc() {
        let mut core = GbCore::new(Cartridge::from_bytes(minimal_rom()).unwrap());
        let cycles = core.step().unwrap();
        assert_eq!(cycles, 4);
        assert_eq!(core.regs.pc, 0x0101);
    }

    #[test]
    fn run_frame_completes() {
        let mut core = GbCore::new(Cartridge::from_bytes(minimal_rom()).unwrap());
        core.run_frame().unwrap();
        assert!(core.clock.t_cycles >= CYCLES_PER_FRAME);
    }
}
