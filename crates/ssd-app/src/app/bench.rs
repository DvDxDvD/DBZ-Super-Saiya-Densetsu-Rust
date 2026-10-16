//! `--bench-app`: the window's frame loop without the window. Each frame of the loop is the window's:
//! the emulated frames of the speed (the machine, the HD menu's driver, the Enhanced 16:9 picture),
//! the pictures, and the interface painted over them (the debug panel open, as the speed buttons
//! are in it) into an offscreen texture on the GPU, which is waited for as a swap chain with one
//! frame of latency would. It runs flat out, with no clock, and reports the emulated frames per
//! second at each speed, where the time of a window frame went, and how many cores were busy.

use super::{
    Enhanced, Env, Pacer, Stages, WindowState, enhanced_session, fallback_picture, frame_layer,
    game_views, hd_layer, header_status, new_machine, note_wram, window_frames,
};
use crate::egui_input::InputAdapter;
use crate::hd::Hd;
use crate::hook_setup::{HookSetup, HookSwitch, HooksArg};
use crate::hotkeys::Speed;
use crate::panel::{Controls, HooksSwitch};
use crate::replay_io::Session;
use crate::snapshot::{Offscreen, PresentTimes};
use crate::ui::{self, View};
use crate::{audio, hd_art};
use anyhow::Context;
use ssd_game::Ruleset;
use ssd_hd::HdOptions;
use ssd_oracle::Joypad;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The size of the window the bench draws: the one the window opens with.
const SIZE: (u32, u32) = (1440, 900);

/// What `--bench-app` is asked for.
#[derive(Debug, Clone)]
pub struct Request {
    /// The replays played, each from power-on.
    pub replays: Vec<PathBuf>,
    /// The speeds run (1, 2, 4 or 8).
    pub speeds: Vec<u32>,
    /// The emulated frames timed at each speed (fewer when the replay ends first).
    pub frames: u64,
    /// The scale and look of the Enhanced picture (`--hd-scale`).
    pub enhanced: HdOptions,
    /// Whether the debug panel is open.
    pub panel: bool,
    /// The frames run before the timing starts: the art is loaded and the buffers are sized.
    pub warm_up: u64,
}

/// The sums of one run's window frames.
#[derive(Debug, Default, Clone, Copy)]
struct Sums {
    window_frames: u64,
    emulated: u64,
    wall: Duration,
    stages: Stages,
    status: Duration,
    views: Duration,
    present: PresentTimes,
}

/// The CPU time this process has used, user and kernel, over all its threads.
#[cfg(windows)]
fn process_cpu() -> Duration {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(
            process: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let ticks = |time: FileTime| (u64::from(time.high) << 32) | u64::from(time.low);
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo-handle of the current process is always valid, and the four times are
    // plain structs the call fills.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if ok == 0 {
        return Duration::ZERO;
    }
    // 100 ns units
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

#[cfg(not(windows))]
fn process_cpu() -> Duration {
    Duration::ZERO
}

fn millis(total: Duration, count: u64) -> f64 {
    total.as_secs_f64() * 1000.0 / count.max(1) as f64
}

/// Plays `replay` at `speed` through the window's frame loop and sums what it costs.
fn run_one(
    rom: &[u8],
    hooks: Option<&HookSetup>,
    switch: Option<&HookSwitch>,
    replay: &Path,
    speed: u32,
    request: &Request,
) -> anyhow::Result<(Sums, Duration)> {
    let ruleset = Ruleset::enhanced();
    let mut session = Session::new(Some(replay), None, rom)?;
    enhanced_session(&mut session, ruleset)?;
    let mut machine = new_machine(rom.to_vec(), hooks, &mut session)?;
    let mut state =
        WindowState::new(Controls::new(HooksArg::Original, HooksSwitch::Allowed, false, false));
    state.controls.speed = Speed::ALL
        .into_iter()
        .find(|s| s.multiplier() == speed)
        .with_context(|| format!("{speed} is not a speed: 1, 2, 4 or 8"))?;
    state.hd = hd_layer(rom, ruleset)?;
    if let Some(hd) = state.hd.as_mut() {
        hd.set_replaying();
    }
    state.panel.open = request.panel;
    state.scaled = Some(request.enhanced);
    machine.set_line_log(true);
    let mut enhanced = Enhanced::new(request.enhanced, rom);
    let env = Env { rom, switch, verify: None };
    let mut offscreen = Offscreen::new(SIZE.0, SIZE.1)?;
    let mut input = InputAdapter::default();
    let mut audio = audio::Audio::muted();
    let clock = Instant::now();
    // Flat out: the clock stays at the start, so the window is always behind and runs as many frames
    // as it can hold
    let mut pacer = Pacer::new(Duration::from_secs_f64(1.0 / machine.target_fps()));
    let mut sums = Sums::default();
    let mut cpu_start = None;
    loop {
        if session.playback_over(&machine) {
            break;
        }
        let frame_started = Instant::now();
        state.stages = Stages::default();
        let frames = pacer.frames(speed, Instant::now());
        let ran = window_frames(
            &mut state,
            &mut machine,
            &mut session,
            &env,
            Joypad::default(),
            &mut audio,
            frames,
        )?;
        pacer.note_cost(ran, state.stages.frames());
        let status_started = Instant::now();
        note_wram(&mut state, &machine);
        let status = header_status(&machine, &state.history, 60.0 * speed as f32);
        let views_started = Instant::now();
        let status_time = views_started - status_started;
        let picture = fallback_picture(&state, &mut machine, Some(&mut enhanced), false);
        let registers = machine.cpu_registers();
        let controls = state.controls;
        let views = game_views(
            state.hd_view.as_ref(),
            state.view.as_ref(),
            &machine,
            picture.as_ref(),
            None,
        );
        let scene = state.hd.as_ref().and_then(|hd| hd.scene(machine.wram()));
        let overlay = state.hd.as_ref().and_then(|hd| hd.overlay(machine.wram(), machine.sram()));
        let symbols = env.symbols();
        let history = &state.history;
        let panel = &mut state.panel;
        let mut hd_art = state.hd.as_mut().map(Hd::art);
        let mut actions = Vec::new();
        let shown: &ssd_oracle::Machine = &machine;
        let views_time = views_started.elapsed();
        let present =
            offscreen.present(&mut input, clock.elapsed().as_secs_f64(), &views, |ui, games| {
                let view = View {
                    status,
                    controls: &controls,
                    registers: &registers,
                    recording: false,
                    verify_labels: None,
                    machine: shown,
                    symbols,
                    history,
                    touch: None,
                };
                let layer = frame_layer(scene.as_ref(), overlay.as_ref(), hd_art.as_deref_mut());
                ui::draw(ui, &view, games, panel, &mut actions, layer);
            })?;
        let wall = frame_started.elapsed();
        log::debug!("window frame: {ran} frames run, at frame {}", machine.frame_count());
        if machine.frame_count() >= request.warm_up {
            cpu_start.get_or_insert_with(process_cpu);
            sums.window_frames += 1;
            sums.emulated += u64::from(ran);
            sums.wall += wall;
            sums.stages.driver += state.stages.driver;
            sums.stages.emulate += state.stages.emulate;
            sums.stages.history += state.stages.history;
            sums.stages.hd += state.stages.hd;
            sums.status += status_time;
            sums.views += views_time;
            sums.present.ui += present.ui;
            sums.present.paint += present.paint;
            sums.present.wait += present.wait;
            if sums.emulated >= request.frames {
                break;
            }
        }
    }
    let cpu = cpu_start.map_or(Duration::ZERO, |start| process_cpu().saturating_sub(start));
    Ok((sums, cpu))
}

/// Runs the bench and prints its table.
///
/// # Errors
///
/// A replay cannot be played, or there is no GPU.
pub fn run(
    rom: &[u8],
    hooks: Option<&HookSetup>,
    switch: Option<&HookSwitch>,
    request: &Request,
) -> anyhow::Result<()> {
    println!(
        "bench-app: the window's frame loop, Enhanced {}x 16:9, art {}, panel {}, {} frames per run",
        request.enhanced.scale,
        if hd_art::pack(rom).is_some() { "on" } else { "off" },
        if request.panel { "open" } else { "closed" },
        request.frames,
    );
    println!(
        "replay      speed  emu fps  holds   win ms  frames  driver emulate    hist      hd  status   views      ui   paint    wait  cores"
    );
    for replay in &request.replays {
        let name =
            replay.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        for &speed in &request.speeds {
            let (sums, cpu) = run_one(rom, hooks, switch, replay, speed, request)?;
            let seconds = sums.wall.as_secs_f64().max(1e-9);
            let fps = sums.emulated as f64 / seconds;
            let target = 60.0 * f64::from(speed);
            let n = sums.window_frames;
            println!(
                "{name:<11} {speed:>4}x {fps:>8.0} {:>6} {:>8.2} {:>7.1} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>6.2}",
                if fps >= target { "yes" } else { "NO" },
                millis(sums.wall, n),
                sums.emulated as f64 / n.max(1) as f64,
                millis(sums.stages.driver, n),
                millis(sums.stages.emulate, n),
                millis(sums.stages.history, n),
                millis(sums.stages.hd, n),
                millis(sums.status, n),
                millis(sums.views, n),
                millis(sums.present.ui, n),
                millis(sums.present.paint, n),
                millis(sums.present.wait, n),
                cpu.as_secs_f64() / seconds,
            );
        }
    }
    println!(
        "(ms per window frame; holds: the flat-out rate reaches 60 x speed; cores: CPU time over wall time)"
    );
    Ok(())
}
