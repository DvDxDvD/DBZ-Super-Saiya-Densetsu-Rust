mod common;

use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio};

/// Master cycles in one NTSC frame (262 lines of 1364); no frame is longer.
const NTSC_FRAME_MCLK: u64 = 262 * 1364;

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

#[test]
fn cgram_shows_the_backdrop_color() {
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 3);
    assert_eq!(m.cgram()[0], 0x001F);
}

#[test]
fn bus_write_and_read_reach_wram() {
    let mut m = machine_with(common::red_backdrop_rom());
    m.bus_write(0x7E_0100, 0xAB);
    assert_eq!(m.wram()[0x100], 0xAB);
    assert_eq!(m.bus_read(0x7E_0100), 0xAB);
}

#[test]
fn cpu_write_log_records_program_order() {
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_cpu_write_log(true);
    run(&mut m, 1);
    assert_eq!(
        m.take_cpu_writes(),
        vec![(0x2100, 0x80), (0x2121, 0x00), (0x2122, 0x1F), (0x2122, 0x00), (0x2100, 0x0F)]
    );
    run(&mut m, 1);
    assert!(m.take_cpu_writes().is_empty(), "the idle loop writes nothing");
}

#[test]
fn cpu_write_log_is_off_by_default_and_records_bus_writes() {
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 1);
    m.bus_write(0x7E_0010, 1);
    assert_eq!(m.take_cpu_writes(), []);

    m.set_cpu_write_log(true);
    m.bus_write(0x7E_0010, 2);
    assert_eq!(m.take_cpu_writes(), vec![(0x7E_0010, 2)]);
    assert!(m.take_cpu_writes().is_empty(), "take drains the log");
}

#[test]
fn dma_writes_are_not_cpu_writes() {
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 1);
    m.set_cpu_write_log(true);
    let setup: &[(u32, u8)] = &[
        (0x2181, 0x00), // WMADD = $00:0200
        (0x2182, 0x02),
        (0x2183, 0x00),
        (0x4300, 0x00), // channel 0: A to B, increment, one register
        (0x4301, 0x80), // B = WMDATA ($2180)
        (0x4302, 0x00), // A = $00:8000 (ROM)
        (0x4303, 0x80),
        (0x4304, 0x00),
        (0x4305, 0x10), // 16 bytes
        (0x4306, 0x00),
        (0x420B, 0x01), // start channel 0
    ];
    for &(address, value) in setup {
        m.bus_write(address, value);
    }
    assert!(m.gpdma_requested());

    let frames = m.frame_count();
    let rendered = m.advance_idle(Joypad::default(), &mut NullAudio, 2000).unwrap();
    assert!(!rendered);
    assert_eq!(m.frame_count(), frames);
    assert!(!m.gpdma_requested(), "the transfer finished while the CPU was held");
    assert_eq!(&m.wram()[0x200..0x210], &common::red_backdrop_rom()[..0x10]);
    assert_eq!(m.take_cpu_writes(), setup, "the DMA's 16 writes to $2180 are not logged");
}

#[test]
fn setting_the_registers_back_is_a_no_op() {
    let mut a = machine_with(common::sram_counter_rom());
    let mut b = machine_with(common::sram_counter_rom());
    run(&mut a, 2);
    run(&mut b, 2);
    a.set_cpu_registers(a.cpu_registers());
    run(&mut a, 3);
    run(&mut b, 3);
    assert_eq!(a.save_state(), b.save_state());
}

#[test]
fn advance_idle_holds_the_cpu_through_a_frame() {
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 1);
    let before = m.cpu_registers();
    let fetches = m.opcode_fetches();
    let cycles = m.total_master_cycles();

    let rendered = m.advance_idle(Joypad::default(), &mut NullAudio, NTSC_FRAME_MCLK).unwrap();

    assert!(rendered);
    assert_eq!(m.frame_count(), 2);
    let after = m.cpu_registers();
    assert_eq!((after.pbr, after.pc), (before.pbr, before.pc));
    assert_eq!(m.opcode_fetches(), fetches, "a held CPU fetches nothing");
    assert!(m.total_master_cycles() >= cycles + NTSC_FRAME_MCLK);
}

#[test]
fn advance_idle_counts_every_frame_it_renders() {
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 1);
    assert!(m.advance_idle(Joypad::default(), &mut NullAudio, 3 * NTSC_FRAME_MCLK).unwrap());
    assert_eq!(m.frame_count(), 4);
}

#[test]
fn counters_split_cpu_time_from_memory_refresh() {
    // Frames 2 and 3 of the red backdrop ROM: the CPU spins on `BRA $801A` (opcode + operand from
    // SlowROM and one idle cycle = 22 master cycles), there is no DMA, and every scanline spends 40
    // master cycles on DRAM refresh.
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 1);
    let (total, busy, fetches) = (m.total_master_cycles(), m.cpu_busy_cycles(), m.opcode_fetches());
    run(&mut m, 2);
    let total = m.total_master_cycles() - total;
    let busy = m.cpu_busy_cycles() - busy;
    let fetches = m.opcode_fetches() - fetches;

    let refresh = total - busy;
    assert_eq!(refresh % 40, 0, "only refresh cycles are not CPU cycles");
    assert!((2 * 261..=2 * 263).contains(&(refresh / 40)), "one refresh per scanline");
    assert!(busy.abs_diff(fetches * 22) <= 22, "one fetch per 22-cycle loop iteration");
}

#[test]
fn cpu_write_log_setting_survives_load_state() {
    let mut m = machine_with(common::sram_counter_rom());
    m.set_cpu_write_log(true);
    run(&mut m, 1);
    let state = m.save_state();
    m.load_state(&state).unwrap();
    m.take_cpu_writes();

    run(&mut m, 1);
    let writes = m.take_cpu_writes();
    assert!(writes.iter().any(|&(address, _)| address == 0x70_0000), "{writes:x?}");
}

#[test]
fn counters_survive_load_state_and_recorded_writes_are_dropped() {
    let mut m = machine_with(common::sram_counter_rom());
    m.set_cpu_write_log(true);
    run(&mut m, 1);
    let state = m.save_state();
    run(&mut m, 1);
    let fetches = m.opcode_fetches();
    let busy = m.cpu_busy_cycles();

    m.load_state(&state).unwrap();
    assert_eq!(m.opcode_fetches(), fetches, "counters are not part of the state");
    assert_eq!(m.cpu_busy_cycles(), busy);
    assert!(m.take_cpu_writes().is_empty(), "writes recorded before the load are dropped");
}

#[test]
fn snapshot_keeps_the_log_flag_and_starts_with_an_empty_log() {
    let mut m = machine_with(common::sram_counter_rom());
    m.set_cpu_write_log(true);
    run(&mut m, 1);
    let mut snap = m.snapshot();
    assert_eq!(snap.take_cpu_writes(), []);
    assert_eq!(snap.opcode_fetches(), m.opcode_fetches());
    run(&mut snap, 1);
    assert!(snap.take_cpu_writes().iter().any(|&(address, _)| address == 0x70_0000));
}

#[test]
fn memory_views_have_hardware_sizes() {
    let m = machine_with(common::sram_counter_rom());
    assert_eq!(m.wram().len(), 128 * 1024);
    assert_eq!(m.vram().len(), 32 * 1024);
    assert_eq!(m.cgram().len(), 256);
    let (low, high) = m.oam();
    assert_eq!((low.len(), high.len()), (512, 32));
    assert_eq!(m.sram().map(<[u8]>::len), Some(2048));
}

#[test]
fn the_interrupt_entry_is_visible_between_steps() {
    // The main loop of the WAI/NMI fixture waits for the NMI; its handler starts at NMI_HANDLER.
    let mut m = machine_with(common::wai_nmi_rom());
    run(&mut m, 1);
    let limit = m.frame_count() + 2;
    while !m.is_handling_interrupt() {
        assert!(m.frame_count() < limit, "no interrupt entry");
        m.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert!(m.is_mid_instruction(), "the entry sequence takes several CPU cycles");
    assert!(!m.at_fetch_boundary());
    while m.is_handling_interrupt() {
        m.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert!(!m.is_mid_instruction());
    let registers = m.cpu_registers();
    assert_eq!(registers.pc, common::NMI_HANDLER, "the handler's first opcode comes next");
    assert_eq!(registers.s, common::MAIN_STACK - 4, "PBR, PC and P were pushed");
}

#[test]
fn hdma_copy_units_count_one_per_channel_and_visible_scanline() {
    let mut m = machine_with(common::hdma_leaf_rom());
    run(&mut m, 2);
    let units = m.hdma_copy_units();
    assert!(units > 0, "HDMA ran in the second frame");
    run(&mut m, 1);
    assert_eq!(m.hdma_copy_units() - units, common::HDMA_UNITS_PER_FRAME);

    // A frame without any HDMA channel: the reload check at the start of every frame copies
    // nothing.
    let mut idle = machine_with(common::red_backdrop_rom());
    run(&mut idle, 3);
    assert_eq!(idle.hdma_copy_units(), 0);
}

#[test]
fn hdma_copy_units_survive_load_state_and_snapshots() {
    let mut m = machine_with(common::hdma_leaf_rom());
    let power_on = m.save_state();
    run(&mut m, 3);
    let units = m.hdma_copy_units();
    assert!(units > 0);

    m.load_state(&power_on).unwrap();
    assert_eq!(m.hdma_copy_units(), units, "the counter is not part of the state");
    assert_eq!(m.snapshot().hdma_copy_units(), units);
}
