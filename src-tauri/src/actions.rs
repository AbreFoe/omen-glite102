//! Runs the action bound to a mouse button when its F13+n key arrives.

use crate::config::{Action, Binding, Step};
use crate::discord::Cmd;
use crate::{input, Shared};
use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn run_steps(steps: &[Step], stop: Option<&AtomicBool>) {
    for s in steps {
        if stop.is_some_and(|f| !f.load(Ordering::Relaxed)) {
            return;
        }
        match s.kind.as_str() {
            "down" => input::vk(s.vk, false),
            "up" => input::vk(s.vk, true),
            "tap" => {
                input::vk(s.vk, false);
                std::thread::sleep(Duration::from_millis(15));
                input::vk(s.vk, true);
            }
            "delay" => std::thread::sleep(Duration::from_millis(s.ms.min(60_000) as u64)),
            _ => {}
        }
    }
}

/// Release every key a macro might have left pressed when it was interrupted.
fn release_all(steps: &[Step]) {
    for s in steps.iter().filter(|s| s.kind == "down") {
        input::vk(s.vk, true);
    }
}

pub fn run(action: &Action, down: bool, app: &AppHandle, running: &Arc<AtomicBool>) {
    let shared = app.state::<Shared>();
    match action {
        Action::None => {}
        Action::Keys { keys } => input::combo(keys, !down),
        Action::Text { text } if down => input::type_text(text),
        Action::Launch { target } if down => {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", target])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
        Action::DiscordMute if down => shared.discord.send(Cmd::ToggleMute),
        Action::DiscordDeafen if down => shared.discord.send(Cmd::ToggleDeaf),
        Action::DiscordPtt => shared.discord.send(Cmd::SetMute(!down)),
        Action::Macro { steps, repeat } => {
            let looping = match repeat.as_str() {
                "while_held" => {
                    running.store(down, Ordering::Relaxed);
                    down
                }
                "toggle" if down => !running.fetch_xor(true, Ordering::Relaxed),
                _ => {
                    if down {
                        let steps = steps.clone();
                        std::thread::spawn(move || run_steps(&steps, None));
                    }
                    false
                }
            };
            if looping {
                let steps = steps.clone();
                let flag = running.clone();
                std::thread::spawn(move || {
                    while flag.load(Ordering::Relaxed) {
                        run_steps(&steps, Some(&flag));
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    release_all(&steps);
                });
            }
        }
        _ => {}
    }
}

pub fn spawn_executor(app: AppHandle, rx: Receiver<(usize, bool)>) {
    std::thread::spawn(move || {
        let running: Vec<Arc<AtomicBool>> = (0..12).map(|_| Arc::new(AtomicBool::new(false))).collect();
        let mut held = [false; 12];
        for (slot, down) in rx {
            // Windows auto-repeats held keys; only edges matter.
            if held[slot] == down {
                continue;
            }
            held[slot] = down;
            let action = {
                let shared = app.state::<Shared>();
                let cfg = shared.cfg.lock().unwrap();
                match cfg.buttons.get(slot) {
                    Some(Binding::App { action }) => action.clone(),
                    _ => continue,
                }
            };
            run(&action, down, &app, &running[slot]);
        }
    });
}
