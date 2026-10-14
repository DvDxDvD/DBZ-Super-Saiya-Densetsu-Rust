//! Comparison of call outcomes, divergence text and saved divergence reports.

mod common;

use ssd_oracle::{
    Difference, Divergence, Machine, MemorySaveStore, Outcome, Registers, StatusRegister, Symbols,
    compare, report,
};

fn registers(p: u8) -> Registers {
    Registers {
        a: 0x1234,
        x: 0x0010,
        y: 0x0020,
        s: 0x1FF0,
        d: 0x0000,
        pbr: 0x00,
        pc: 0x800A,
        dbr: 0x7E,
        p: StatusRegister::from(p),
        emulation_mode: false,
    }
}

/// Memories of one outcome, sized like the hardware's.
struct Memories {
    wram: Vec<u8>,
    sram: Vec<u8>,
    vram: Vec<u16>,
    cgram: Vec<u16>,
    oam_low: Vec<u8>,
    oam_high: Vec<u8>,
    writes: Vec<(u32, u8)>,
}

impl Memories {
    fn new() -> Self {
        Self {
            wram: vec![0x55; 0x2_0000],
            sram: vec![0xAA; 0x800],
            vram: vec![0x1111; 0x8000],
            cgram: vec![0x2222; 0x100],
            oam_low: vec![0x33; 0x200],
            oam_high: vec![0x44; 0x20],
            writes: vec![(0x00_2100, 0x0F), (0x7E_0010, 0x01), (0x00_2132, 0xE0)],
        }
    }

    fn outcome(&self, regs: &Registers) -> Outcome<'_> {
        Outcome {
            regs: regs.clone(),
            wram: &self.wram,
            sram: Some(&self.sram),
            vram: &self.vram,
            cgram: &self.cgram,
            oam_low: &self.oam_low,
            oam_high: &self.oam_high,
            cpu_writes: &self.writes,
        }
    }
}

#[test]
fn equal_outcomes_do_not_differ() {
    let (original, rust) = (Memories::new(), Memories::new());
    let differences =
        compare(&original.outcome(&registers(0x30)), &rust.outcome(&registers(0x30)), 0xFF, false);
    assert!(differences.is_empty(), "{differences:?}");
}

#[test]
fn a_register_a_wram_byte_and_the_write_order_are_compared() {
    let original = Memories::new();
    let mut rust = Memories::new();
    rust.wram[0x0040] = 0x35;
    rust.writes.swap(0, 2);
    let mut regs = registers(0x30);
    regs.x = 0x0011;

    let differences =
        compare(&original.outcome(&registers(0x30)), &rust.outcome(&regs), 0xFF, false);
    assert_eq!(
        differences,
        [
            Difference::Register { name: "X", original: 0x0010, rust: 0x0011 },
            Difference::Memory { space: "wram", address: 0x0040, original: 0x55, rust: 0x35 },
            // The WRAM write is not an observable register: the two others are compared.
            Difference::Write {
                index: 0,
                original: Some((0x00_2100, 0x0F)),
                rust: Some((0x00_2132, 0xE0)),
            },
            Difference::Write {
                index: 1,
                original: Some((0x00_2132, 0xE0)),
                rust: Some((0x00_2100, 0x0F)),
            },
        ]
    );
}

#[test]
fn flags_outside_the_mask_are_ignored() {
    let (original, rust) = (Memories::new(), Memories::new());
    let (with_z, without_z) = (registers(0x32), registers(0x30));
    let differences = compare(&original.outcome(&with_z), &rust.outcome(&without_z), !0x02, false);
    assert!(differences.is_empty(), "{differences:?}");
    assert_eq!(
        compare(&original.outcome(&with_z), &rust.outcome(&without_z), 0xFF, false),
        [Difference::Register { name: "P", original: 0x32, rust: 0x30 }]
    );
}

#[test]
fn ppu_memories_are_compared_by_byte_address() {
    let original = Memories::new();
    let mut rust = Memories::new();
    rust.vram[0x1000] = 0x1112; // low byte of word $1000: VRAM byte $2000
    rust.cgram[0x01] = 0x3222; // high byte of color 1: CGRAM byte $03
    rust.oam_high[0x1F] = 0x45; // the last byte of OAM ($21F)
    rust.sram[0x7FF] = 0xAB;

    let regs = registers(0x30);
    assert_eq!(
        compare(&original.outcome(&regs), &rust.outcome(&regs), 0xFF, false),
        [
            Difference::Memory { space: "sram", address: 0x7FF, original: 0xAA, rust: 0xAB },
            Difference::Memory { space: "vram", address: 0x2000, original: 0x11, rust: 0x12 },
            Difference::Memory { space: "cgram", address: 0x03, original: 0x22, rust: 0x32 },
            Difference::Memory { space: "oam", address: 0x21F, original: 0x44, rust: 0x45 },
        ]
    );
}

#[test]
fn calls_crossed_by_hdma_skip_ppu_memories_and_writes() {
    let original = Memories::new();
    let mut rust = Memories::new();
    rust.vram[0] = 0;
    rust.cgram[0] = 0;
    rust.oam_low[0] = 0;
    rust.oam_high[0] = 0;
    rust.writes.clear();
    rust.wram[0x10] = 0;
    rust.sram[0x10] = 0;

    let regs = registers(0x30);
    assert_eq!(
        compare(&original.outcome(&regs), &rust.outcome(&regs), 0xFF, true),
        [
            Difference::Memory { space: "wram", address: 0x10, original: 0x55, rust: 0 },
            Difference::Memory { space: "sram", address: 0x10, original: 0xAA, rust: 0 },
        ]
    );
}

#[test]
fn writes_are_compared_on_observable_registers_through_any_mirror() {
    let mut original = Memories::new();
    original.writes = vec![
        (0x00_2118, 0x01), // VMDATAL
        (0x00_4202, 0x07), // WRMPYA: CPU-internal math, not observable
        (0x00_4300, 0x01), // DMA channel 0
        (0x00_2141, 0x02), // APU port 1
        (0x7E_2100, 0x03), // WRAM, not a register
    ];
    let mut rust = Memories::new();
    rust.writes = vec![
        (0x80_2118, 0x01), // the same register in the bank $80 mirror
        (0xBF_4300, 0x01),
        (0x00_2145, 0x02), // APU port 1 through its mirror at $2144-$2147
        (0x00_4203, 0x05), // WRMPYB: not observable
    ];
    let regs = registers(0x30);
    assert_eq!(compare(&original.outcome(&regs), &rust.outcome(&regs), 0xFF, false), []);

    rust.writes.push((0x00_420B, 0x01)); // MDMAEN, one write more
    assert_eq!(
        compare(&original.outcome(&regs), &rust.outcome(&regs), 0xFF, false),
        [Difference::Write { index: 3, original: None, rust: Some((0x00_420B, 0x01)) }]
    );
}

fn symbols_file(source: &str) -> Symbols {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("symbols.toml");
    std::fs::write(&path, source).unwrap();
    Symbols::load(&path).unwrap()
}

#[test]
fn display_names_variables_and_formats_every_kind_of_difference() {
    let symbols = symbols_file("[[variable]]\nname = \"seed\"\naddress = 0x7E0040\nsize = 2\n");
    let differences = vec![
        Difference::Register { name: "A", original: 0x1234, rust: 0x1235 },
        Difference::Register { name: "P", original: 0x32, rust: 0x30 },
        Difference::Memory { space: "wram", address: 0x0040, original: 0x34, rust: 0x35 },
        Difference::Memory { space: "wram", address: 0x0041, original: 0x01, rust: 0x02 },
        Difference::Memory { space: "wram", address: 0x1_0042, original: 0x01, rust: 0x02 },
        Difference::Memory { space: "vram", address: 0x2000, original: 0x11, rust: 0x12 },
        Difference::Write { index: 3, original: Some((0x2118, 0x01)), rust: None },
    ];
    let divergence = Divergence::new("rng_next_a".into(), 12, 345, differences, Some(&symbols));
    assert_eq!(divergence.truncated, 0);
    assert_eq!(
        divergence.to_string(),
        "rng_next_a: call 12 at frame 345, 7 differences\n\
         rng_next_a — A: $1234 != $1235\n\
         rng_next_a — P: $32 (nvMXdiZc) != $30 (nvMXdizc)\n\
         rng_next_a — wram[$7E:0040] seed: $34 != $35\n\
         rng_next_a — wram[$7E:0041] seed+1: $01 != $02\n\
         rng_next_a — wram[$7F:0042]: $01 != $02\n\
         rng_next_a — vram[$2000]: $11 != $12\n\
         rng_next_a — write 3: $00:2118 = $01 != none\n"
    );
}

#[test]
fn display_counts_the_differences_not_kept() {
    let differences: Vec<Difference> = (0..300)
        .map(|address| Difference::Memory { space: "sram", address, original: 0, rust: 1 })
        .collect();
    let divergence = Divergence::new("leaf".into(), 0, 1, differences, None);
    assert_eq!((divergence.differences.len(), divergence.truncated), (256, 44));
    let text = divergence.to_string();
    assert!(text.starts_with("leaf: call 0 at frame 1, 300 differences\n"), "{text}");
    assert!(text.ends_with("leaf — sram[$00FF]: $00 != $01\nleaf — … 44 more differences\n"));
}

#[test]
fn save_writes_the_entry_state_and_the_text_under_a_repro_header() {
    let dir = tempfile::tempdir().unwrap();
    let reports = dir.path().join("divergences"); // created on demand
    let mut pristine =
        Machine::new(common::jsr_leaf_rom(), Box::new(MemorySaveStore::default())).unwrap();
    // The state is taken at the hooked opcode fetch: through the bank $80 mirror here, the
    // header holds the physical key.
    let mut at_the_call = pristine.cpu_registers();
    (at_the_call.pbr, at_the_call.pc) = (0x80, 0x8040);
    pristine.set_cpu_registers(at_the_call);
    let difference = Difference::Memory { space: "wram", address: 0x10, original: 1, rust: 2 };
    let divergence = Divergence::new("leaf".into(), 3, 7, vec![difference], None);

    let path = report::save(&reports, &divergence, &pristine).unwrap();
    assert_eq!(path, reports.join("leaf-7-3.ssdstate"));
    assert_eq!(std::fs::read(&path).unwrap(), pristine.save_state());
    let text = std::fs::read_to_string(reports.join("leaf-7-3.txt")).unwrap();
    assert_eq!(text, format!("ssd-repro: key=$008040 routine=leaf\n{divergence}"));
    assert_eq!(report::text_path(&path), reports.join("leaf-7-3.txt"));
}

#[test]
fn the_header_names_the_routine_as_it_is_not_as_the_file_is() {
    let dir = tempfile::tempdir().unwrap();
    let pristine =
        Machine::new(common::jsr_leaf_rom(), Box::new(MemorySaveStore::default())).unwrap();
    let divergence = Divergence::new("odd name/1".into(), 0, 1, Vec::new(), None);

    let path = report::save(dir.path(), &divergence, &pristine).unwrap();
    assert_eq!(path, dir.path().join("odd_name_1-1-0.ssdstate"));
    let text = std::fs::read_to_string(report::text_path(&path)).unwrap();
    let header = report::ReproHeader::parse(&text).expect("a header");
    assert_eq!(header.routine, "odd name/1");
}

#[test]
fn a_header_is_the_first_line_of_a_report_and_parses_back() {
    let header = report::ReproHeader { routine: "rng_next_a".into(), key: 0x00_82A3 };
    assert_eq!(header.to_string(), "ssd-repro: key=$0082A3 routine=rng_next_a");
    let text = format!("{header}\nrng_next_a: call 12 at frame 345, 1 difference\n");
    assert_eq!(report::ReproHeader::parse(&text), Some(header));

    // A name keeps everything up to the end of the line.
    let spaced = report::ReproHeader { routine: "a b=c".into(), key: 0x7E_0000 };
    assert_eq!(report::ReproHeader::parse(&format!("{spaced}\n")), Some(spaced));
}

#[test]
fn text_without_a_well_formed_first_line_header_has_none() {
    for text in [
        "",
        "leaf: call 3 at frame 7, 1 difference\n",
        "leaf: call 3 at frame 7, 1 difference\nssd-repro: key=$008040 routine=leaf\n",
        "ssd-repro: key=$008040 routine=\n",
        "ssd-repro: key=$008040\n",
        "ssd-repro: key=008040 routine=leaf\n",
        "ssd-repro: key=$ routine=leaf\n",
        "ssd-repro: key=$12345678 routine=leaf\n",
        "ssd-repro: key=$00804G routine=leaf\n",
        // `from_str_radix` takes a sign: six characters, one of them a sign, are not a key.
        "ssd-repro: key=$+08040 routine=leaf\n",
        "ssd-repro: key=$-08040 routine=leaf\n",
        "ssd-repro: routine=leaf key=$008040\n",
    ] {
        assert_eq!(report::ReproHeader::parse(text), None, "{text:?}");
    }
}
