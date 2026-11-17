//! Owned, foreground-gated port of PBZPTID Dll/Macro.cpp and MacroUtils.cpp.
//! Only explicit enabled configuration can produce input; tests use the pure model.
use anyhow::{Context, Result};
use std::{
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_DELAY_MS: u32 = 9_999;
const HOLD_MS: u64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Scope3Q,
    ThreeQ,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub enabled: bool,
    pub mode: Mode,
    pub delay_ms: u32,
    pub target_pid: Option<u32>,
    pub menu_open: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::Scope3Q,
            delay_ms: 25,
            target_pid: None,
            menu_open: false,
        }
    }
}

impl Config {
    fn bounded(mut self) -> Self {
        self.delay_ms = self.delay_ms.min(MAX_DELAY_MS);
        self
    }

    fn same(self, other: Self) -> bool {
        self.enabled == other.enabled
            && self.mode == other.mode
            && self.delay_ms == other.delay_ms
            && self.target_pid == other.target_pid
            && self.menu_open == other.menu_open
    }
}

struct Shared {
    config: Config,
    generation: u64,
    stop: bool,
}

/// The worker owns its thread and every synthetic input it presses.
/// Repeated unchanged render updates neither wake the worker nor allocate.
pub struct Worker {
    shared: Arc<Mutex<Shared>>,
    handle: Option<JoinHandle<Result<()>>>,
}

impl Worker {
    pub fn start() -> Result<Self> {
        #[cfg(windows)]
        {
            let shared = Arc::new(Mutex::new(Shared {
                config: Config::default(),
                generation: 0,
                stop: false,
            }));
            let thread_shared = Arc::clone(&shared);
            let handle = thread::Builder::new()
                .name("pointblank-input-macros".into())
                .spawn(move || native::run(thread_shared))
                .context("Start owned input-macro worker")?;
            Ok(Self {
                shared,
                handle: Some(handle),
            })
        }
        #[cfg(not(windows))]
        {
            anyhow::bail!("Input-macro worker requires Windows")
        }
    }

    pub fn update(&self, config: Config) {
        let config = config.bounded();
        let changed = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if shared.stop || shared.config.same(config) {
                false
            } else {
                shared.config = config;
                // Preserve even A -> B -> A changes between worker polls.
                shared.generation = shared.generation.wrapping_add(1);
                true
            }
        };
        if changed && let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
    }

    pub fn shutdown(&mut self) -> Result<()> {
        {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.stop = true;
        }
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Input-macro worker panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Left,
    Three,
    Q,
}

impl Control {
    #[cfg(windows)]
    fn bit(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Three => 2,
            Self::Q => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Down(Control),
    Up(Control),
}

const SCOPE_SEQUENCE: [Event; 6] = [
    Event::Down(Control::Left),
    Event::Up(Control::Left),
    Event::Down(Control::Three),
    Event::Up(Control::Three),
    Event::Down(Control::Q),
    Event::Up(Control::Q),
];
const THREE_SEQUENCE: [Event; 4] = [
    Event::Down(Control::Three),
    Event::Up(Control::Three),
    Event::Down(Control::Q),
    Event::Up(Control::Q),
];

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    target_foreground: bool,
    left: bool,
    right: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Effects {
    event: Option<Event>,
    cancel: bool,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    index: usize,
    due_ms: u64,
}

/// Millisecond-clock model. It performs zero OS operations and owns zero threads.
/// Deadlines start at actual output time, so late polls preserve 20-ms key holds.
#[derive(Debug, Default)]
struct Machine {
    generation: Option<u64>,
    pending: Option<Pending>,
    held: Option<Control>,
    armed: bool,
    previous_trigger: bool,
}

impl Machine {
    fn cancel(&mut self) -> Effects {
        self.pending = None;
        self.armed = false;
        self.previous_trigger = false;
        Effects {
            event: self.held.take().map(Event::Up),
            cancel: true,
        }
    }

    // The pure model assumes instantaneous output. The native adapter confirms
    // completion so API latency and millisecond rounding never shorten a hold.
    fn output_completed(&mut self, at_ms: u64, config: Config, event: Event) {
        if let Some(pending) = &mut self.pending {
            let wait = match event {
                Event::Down(_) => HOLD_MS,
                Event::Up(_) => u64::from(config.delay_ms.min(MAX_DELAY_MS)),
            };
            pending.due_ms = at_ms.saturating_add(wait);
        }
    }

    fn tick(&mut self, now_ms: u64, generation: u64, config: Config, sample: Sample) -> Effects {
        let available = config.enabled
            && !config.menu_open
            && config.target_pid.is_some_and(|pid| pid != 0)
            && sample.target_foreground;
        if self.generation != Some(generation) || !available {
            self.generation = Some(generation);
            return self.cancel();
        }

        // Following startup, cancellation or focus/menu return, observe both
        // buttons released before accepting a fresh physical rising edge.
        if !sample.left && !sample.right {
            self.armed = true;
        }
        let trigger = match config.mode {
            Mode::Scope3Q => sample.right,
            Mode::ThreeQ => sample.left,
        };
        let rising = trigger && !self.previous_trigger;
        self.previous_trigger = trigger;
        if self.pending.is_none() && self.armed && rising {
            self.armed = false;
            self.pending = Some(Pending {
                index: 0,
                due_ms: now_ms.saturating_add(u64::from(config.delay_ms.min(MAX_DELAY_MS))),
            });
        }

        let Some(pending) = self.pending else {
            return Effects::default();
        };
        if now_ms < pending.due_ms {
            return Effects::default();
        }
        let sequence: &[Event] = match config.mode {
            Mode::Scope3Q => &SCOPE_SEQUENCE,
            Mode::ThreeQ => &THREE_SEQUENCE,
        };
        let event = sequence[pending.index];
        let wait = match event {
            Event::Down(control) => {
                self.held = Some(control);
                HOLD_MS
            }
            Event::Up(_) => {
                self.held = None;
                u64::from(config.delay_ms.min(MAX_DELAY_MS))
            }
        };
        self.pending = if pending.index + 1 == sequence.len() {
            None
        } else {
            Some(Pending {
                index: pending.index + 1,
                due_ms: now_ms.saturating_add(wait),
            })
        };
        Effects {
            event: Some(event),
            cancel: false,
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    #[cfg(not(test))]
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    };
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };

    fn sample(config: Config) -> Sample {
        if !config.enabled || config.menu_open {
            return Sample::default();
        }
        let Some(target_pid) = config.target_pid.filter(|pid| *pid != 0) else {
            return Sample::default();
        };
        // Resolve the current HWND's PID every poll: process exit and foreground
        // changes invalidate the context without retaining a stale HWND.
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return Sample::default();
        }
        let mut pid = 0;
        let thread_id = unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
        if thread_id == 0 || pid != target_pid {
            return Sample::default();
        }
        Sample {
            target_foreground: true,
            left: unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) } < 0,
            right: unsafe { GetAsyncKeyState(i32::from(VK_RBUTTON.0)) } < 0,
        }
    }

    #[derive(Default)]
    struct Inputs {
        held: u8,
    }

    impl Inputs {
        fn emit(&mut self, event: Event) -> Result<()> {
            #[cfg(test)]
            {
                let _ = event;
                anyhow::bail!("Native macro input is excluded from test builds")
            }
            #[cfg(not(test))]
            {
                let (control, down) = match event {
                    Event::Down(control) => (control, true),
                    Event::Up(control) => (control, false),
                };
                let input = match control {
                    Control::Left => INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dwFlags: if down {
                                    MOUSEEVENTF_LEFTDOWN
                                } else {
                                    MOUSEEVENTF_LEFTUP
                                },
                                ..Default::default()
                            },
                        },
                    },
                    Control::Three | Control::Q => INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: VIRTUAL_KEY(if control == Control::Three {
                                    0x33
                                } else {
                                    0x51
                                }),
                                dwFlags: if down {
                                    Default::default()
                                } else {
                                    KEYEVENTF_KEYUP
                                },
                                ..Default::default()
                            },
                        },
                    },
                };
                let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
                anyhow::ensure!(sent == 1, "SendInput failed for {event:?}");
                if down {
                    self.held |= control.bit();
                } else {
                    self.held &= !control.bit();
                }
                Ok(())
            }
        }

        fn release_all(&mut self) -> Result<()> {
            let mut failure = None;
            for control in [Control::Left, Control::Three, Control::Q] {
                if self.held & control.bit() != 0
                    && let Err(error) = self.emit(Event::Up(control))
                {
                    failure = Some(error);
                }
            }
            if let Some(error) = failure {
                return Err(error.context("Release owned synthetic macro input"));
            }
            Ok(())
        }
    }

    impl Drop for Inputs {
        fn drop(&mut self) {
            // Unwinding also releases successful downs. Failed ups remain tracked
            // and get additional bounded attempts during teardown.
            for _ in 0..3 {
                if self.held == 0 {
                    break;
                }
                let _ = self.release_all();
            }
        }
    }

    pub(super) fn run(shared: Arc<Mutex<Shared>>) -> Result<()> {
        let clock = Instant::now();
        let mut machine = Machine::default();
        let mut inputs = Inputs::default();
        let result = (|| -> Result<()> {
            loop {
                // Serialize a short sample/output operation with update. Once an
                // update returns, stale configuration can produce zero new downs.
                let state = shared.lock().unwrap_or_else(|error| error.into_inner());
                if state.stop {
                    machine.cancel();
                    break;
                }
                let config = state.config;
                let active = config.enabled
                    && !config.menu_open
                    && config.target_pid.is_some_and(|pid| pid != 0);
                // At most six outputs can be due (zero custom delay); each down
                // still establishes a new real 20-ms hold deadline.
                for _ in 0..SCOPE_SEQUENCE.len() {
                    let now_ms = clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                    // Revalidate foreground PID immediately before every output,
                    // including zero-delay transitions within a single poll.
                    let effects = machine.tick(now_ms, state.generation, config, sample(config));
                    if effects.cancel {
                        inputs.release_all()?;
                        break;
                    }
                    match effects.event {
                        Some(event) => {
                            inputs.emit(event)?;
                            // Ceiling to the next clock millisecond guarantees
                            // at least 20 real milliseconds after each down.
                            let completed_ms = clock
                                .elapsed()
                                .as_millis()
                                .saturating_add(1)
                                .min(u128::from(u64::MAX))
                                as u64;
                            machine.output_completed(completed_ms, config, event);
                        }
                        None => break,
                    }
                }
                drop(state);
                thread::park_timeout(Duration::from_millis(if active { 1 } else { 8 }));
            }
            Ok(())
        })();
        let released = inputs.release_all();
        result.and(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: Mode, delay_ms: u32) -> Config {
        Config {
            enabled: true,
            mode,
            delay_ms,
            target_pid: Some(42),
            menu_open: false,
        }
    }

    fn sample(mode: Mode, pressed: bool) -> Sample {
        Sample {
            target_foreground: true,
            left: mode == Mode::ThreeQ && pressed,
            right: mode == Mode::Scope3Q && pressed,
        }
    }

    fn arm(machine: &mut Machine, config: Config) {
        assert!(
            machine
                .tick(0, 0, config, sample(config.mode, false))
                .cancel
        );
        assert_eq!(
            machine.tick(1, 0, config, sample(config.mode, false)),
            Effects::default()
        );
    }

    fn events(mode: Mode, delay_ms: u32) -> Vec<(u64, Event)> {
        let config = config(mode, delay_ms);
        let mut machine = Machine::default();
        arm(&mut machine, config);
        let mut events = Vec::new();
        let end = 2 + u64::from(delay_ms.min(MAX_DELAY_MS)) * 3 + HOLD_MS * 3;
        for now in 2..=end + 10 {
            for _ in 0..6 {
                let effects = machine.tick(now, 0, config, sample(mode, true));
                assert!(!effects.cancel);
                if let Some(event) = effects.event {
                    events.push((now, event));
                } else {
                    break;
                }
            }
        }
        assert_eq!(machine.held, None);
        assert!(machine.pending.is_none());
        events
    }

    #[test]
    fn disabled_default_and_bounded_custom_delay() {
        let default = Config::default();
        assert!(!default.enabled);
        assert_eq!(default.target_pid, None);
        assert_eq!(config(Mode::Scope3Q, u32::MAX).bounded().delay_ms, 9_999);
        assert_eq!(config(Mode::Scope3Q, 0).bounded().delay_ms, 0);
    }

    #[test]
    fn exact_scope_sequence_at_every_source_preset_and_custom() {
        for delay in [0, 25, 50, 75, 100, 120, 9_999, u32::MAX] {
            let d = u64::from(delay.min(MAX_DELAY_MS));
            assert_eq!(
                events(Mode::Scope3Q, delay),
                vec![
                    (2 + d, Event::Down(Control::Left)),
                    (22 + d, Event::Up(Control::Left)),
                    (22 + 2 * d, Event::Down(Control::Three)),
                    (42 + 2 * d, Event::Up(Control::Three)),
                    (42 + 3 * d, Event::Down(Control::Q)),
                    (62 + 3 * d, Event::Up(Control::Q)),
                ]
            );
        }
    }

    #[test]
    fn exact_three_q_sequence_at_every_source_preset_and_custom() {
        for delay in [0, 25, 50, 75, 100, 120, 9_999, u32::MAX] {
            let d = u64::from(delay.min(MAX_DELAY_MS));
            assert_eq!(
                events(Mode::ThreeQ, delay),
                vec![
                    (2 + d, Event::Down(Control::Three)),
                    (22 + d, Event::Up(Control::Three)),
                    (22 + 2 * d, Event::Down(Control::Q)),
                    (42 + 2 * d, Event::Up(Control::Q)),
                ]
            );
        }
    }

    #[test]
    fn one_trigger_per_hold_then_fresh_release_edge() {
        for mode in [Mode::Scope3Q, Mode::ThreeQ] {
            let config = config(mode, 0);
            let mut machine = Machine::default();
            arm(&mut machine, config);
            assert!(matches!(
                machine.tick(2, 0, config, sample(mode, true)).event,
                Some(Event::Down(_))
            ));
            for now in 3..200 {
                machine.tick(now, 0, config, sample(mode, true));
            }
            assert!(machine.pending.is_none());
            assert_eq!(machine.tick(200, 0, config, sample(mode, true)).event, None);
            machine.tick(201, 0, config, sample(mode, false));
            assert!(matches!(
                machine.tick(202, 0, config, sample(mode, true)).event,
                Some(Event::Down(_))
            ));
        }
    }

    #[test]
    fn startup_held_trigger_requires_full_release() {
        for mode in [Mode::Scope3Q, Mode::ThreeQ] {
            let config = config(mode, 0);
            let mut machine = Machine::default();
            for now in 0..100 {
                assert_eq!(machine.tick(now, 0, config, sample(mode, true)).event, None);
            }
            // Releasing only the selected trigger while the other is held also
            // keeps the post-cancellation gate closed.
            let other_held = Sample {
                target_foreground: true,
                left: mode == Mode::Scope3Q,
                right: mode == Mode::ThreeQ,
            };
            machine.tick(100, 0, config, other_held);
            assert_eq!(machine.tick(101, 0, config, sample(mode, true)).event, None);
            machine.tick(102, 0, config, sample(mode, false));
            assert!(matches!(
                machine.tick(103, 0, config, sample(mode, true)).event,
                Some(Event::Down(_))
            ));
        }
    }

    #[test]
    fn cancellation_releases_each_possible_synthetic_hold() {
        for (mode, at, held) in [
            (Mode::Scope3Q, 2, Control::Left),
            (Mode::Scope3Q, 23, Control::Three),
            (Mode::Scope3Q, 44, Control::Q),
            (Mode::ThreeQ, 2, Control::Three),
            (Mode::ThreeQ, 23, Control::Q),
        ] {
            let config = config(mode, 0);
            let mut machine = Machine::default();
            arm(&mut machine, config);
            for now in 2..=at {
                machine.tick(now, 0, config, sample(mode, true));
            }
            assert_eq!(machine.held, Some(held));
            assert_eq!(
                machine.cancel(),
                Effects {
                    event: Some(Event::Up(held)),
                    cancel: true
                }
            );
            assert_eq!(machine.held, None);
            assert!(machine.pending.is_none());
            assert_eq!(machine.cancel().event, None);
        }
    }

    #[test]
    fn menu_focus_disable_and_missing_target_cancel_without_retrigger() {
        for cause in 0..5 {
            let active = config(Mode::Scope3Q, 0);
            let mut machine = Machine::default();
            arm(&mut machine, active);
            assert_eq!(
                machine.tick(2, 0, active, sample(active.mode, true)).event,
                Some(Event::Down(Control::Left))
            );
            let mut blocked = active;
            let mut input = sample(active.mode, true);
            match cause {
                0 => blocked.enabled = false,
                1 => blocked.menu_open = true,
                2 => blocked.target_pid = None,
                3 => blocked.target_pid = Some(0),
                _ => input.target_foreground = false,
            }
            let canceled = machine.tick(3, 1, blocked, input);
            assert!(canceled.cancel);
            assert_eq!(canceled.event, Some(Event::Up(Control::Left)));
            assert!(machine.tick(4, 2, active, sample(active.mode, true)).cancel);
            assert_eq!(
                machine
                    .tick(100, 2, active, sample(active.mode, true))
                    .event,
                None
            );
            machine.tick(101, 2, active, sample(active.mode, false));
            assert_eq!(
                machine
                    .tick(102, 2, active, sample(active.mode, true))
                    .event,
                Some(Event::Down(Control::Left))
            );
        }
    }

    #[test]
    fn same_configuration_focus_loss_also_requires_release_on_return() {
        let config = config(Mode::ThreeQ, 0);
        let mut machine = Machine::default();
        arm(&mut machine, config);
        machine.tick(2, 0, config, sample(config.mode, true));
        let lost = Sample {
            target_foreground: false,
            ..sample(config.mode, true)
        };
        assert_eq!(
            machine.tick(3, 0, config, lost).event,
            Some(Event::Up(Control::Three))
        );
        assert_eq!(
            machine
                .tick(100, 0, config, sample(config.mode, true))
                .event,
            None
        );
        machine.tick(101, 0, config, sample(config.mode, false));
        assert_eq!(
            machine
                .tick(102, 0, config, sample(config.mode, true))
                .event,
            Some(Event::Down(Control::Three))
        );
    }

    #[test]
    fn mode_target_delay_and_intermediate_config_changes_cancel() {
        for change in 0..4 {
            let original = config(Mode::Scope3Q, 0);
            let mut machine = Machine::default();
            arm(&mut machine, original);
            machine.tick(2, 0, original, sample(original.mode, true));
            let mut changed = original;
            match change {
                0 => changed.mode = Mode::ThreeQ,
                1 => changed.target_pid = Some(43),
                2 => changed.delay_ms = 50,
                _ => {} // A -> B -> A still increments the shared generation.
            }
            assert_eq!(
                machine.tick(3, 2, changed, sample(changed.mode, true)),
                Effects {
                    event: Some(Event::Up(Control::Left)),
                    cancel: true
                }
            );
            assert_eq!(
                machine
                    .tick(100, 2, changed, sample(changed.mode, true))
                    .event,
                None
            );
            machine.tick(101, 2, changed, sample(changed.mode, false));
            machine.tick(102, 2, changed, sample(changed.mode, true));
            let expected = if changed.mode == Mode::ThreeQ {
                Control::Three
            } else {
                Control::Left
            };
            if changed.delay_ms == 0 {
                assert_eq!(machine.held, Some(expected));
            } else {
                assert_eq!(
                    machine
                        .tick(
                            102 + u64::from(changed.delay_ms),
                            2,
                            changed,
                            sample(changed.mode, true)
                        )
                        .event,
                    Some(Event::Down(expected))
                );
            }
        }
    }

    #[test]
    fn canceled_wait_never_emits_a_late_down() {
        let active = config(Mode::Scope3Q, 100);
        let mut machine = Machine::default();
        arm(&mut machine, active);
        machine.tick(2, 0, active, sample(active.mode, true));
        assert!(machine.pending.is_some());
        let disabled = Config {
            enabled: false,
            ..active
        };
        let canceled = machine.tick(101, 1, disabled, sample(active.mode, true));
        assert!(canceled.cancel);
        assert_eq!(canceled.event, None);
        assert_eq!(
            machine
                .tick(100_000, 1, disabled, sample(active.mode, true))
                .event,
            None
        );
        assert!(machine.pending.is_none());
    }

    #[test]
    fn late_poll_preserves_actual_twenty_millisecond_hold() {
        let config = config(Mode::ThreeQ, 25);
        let mut machine = Machine::default();
        arm(&mut machine, config);
        machine.tick(2, 0, config, sample(config.mode, true));
        assert_eq!(
            machine
                .tick(1000, 0, config, sample(config.mode, true))
                .event,
            Some(Event::Down(Control::Three))
        );
        assert_eq!(
            machine
                .tick(1019, 0, config, sample(config.mode, true))
                .event,
            None
        );
        assert_eq!(
            machine
                .tick(1020, 0, config, sample(config.mode, true))
                .event,
            Some(Event::Up(Control::Three))
        );
        assert_eq!(
            machine
                .tick(1044, 0, config, sample(config.mode, true))
                .event,
            None
        );
        assert_eq!(
            machine
                .tick(1045, 0, config, sample(config.mode, true))
                .event,
            Some(Event::Down(Control::Q))
        );
    }

    #[test]
    fn native_output_latency_is_added_to_holds_and_following_delays() {
        let config = config(Mode::ThreeQ, 25);
        let mut machine = Machine::default();
        arm(&mut machine, config);
        machine.tick(2, 0, config, sample(config.mode, true));
        let down = machine
            .tick(27, 0, config, sample(config.mode, true))
            .event
            .unwrap();
        machine.output_completed(31, config, down);
        assert_eq!(
            machine.tick(50, 0, config, sample(config.mode, true)).event,
            None
        );
        let up = machine
            .tick(51, 0, config, sample(config.mode, true))
            .event
            .unwrap();
        assert_eq!(up, Event::Up(Control::Three));
        machine.output_completed(55, config, up);
        assert_eq!(
            machine.tick(79, 0, config, sample(config.mode, true)).event,
            None
        );
        assert_eq!(
            machine.tick(80, 0, config, sample(config.mode, true)).event,
            Some(Event::Down(Control::Q))
        );
    }

    #[test]
    fn trigger_during_running_sequence_is_never_queued() {
        let config = config(Mode::ThreeQ, 25);
        let mut machine = Machine::default();
        arm(&mut machine, config);
        machine.tick(2, 0, config, sample(config.mode, true));
        machine.tick(3, 0, config, sample(config.mode, false));
        machine.tick(4, 0, config, sample(config.mode, true));
        for now in 5..1000 {
            machine.tick(now, 0, config, sample(config.mode, true));
        }
        assert!(machine.pending.is_none());
        assert_eq!(
            machine
                .tick(1000, 0, config, sample(config.mode, true))
                .event,
            None
        );
    }
}
