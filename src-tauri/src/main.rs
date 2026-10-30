#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod config;
mod discord;
mod hidpp;
mod input;
mod profile;

use config::Config;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

pub struct Shared {
    pub dev: Mutex<Option<hidpp::Device>>,
    pub cfg: Mutex<Config>,
    pub discord: discord::Handle,
}

type R<T> = Result<T, String>;

#[derive(Serialize)]
struct DeviceInfo {
    connected: bool,
    name: String,
    dpi_min: u16,
    dpi_max: u16,
    dpi_step: u16,
    dpi: u16,
    rates: Vec<u8>,
}

fn with_dev<T>(s: &Shared, f: impl FnOnce(&hidpp::Device) -> R<T>) -> R<T> {
    let g = s.dev.lock().unwrap();
    f(g.as_ref().ok_or("Мышь не подключена")?)
}

#[tauri::command]
fn get_device(s: State<Shared>) -> DeviceInfo {
    with_dev(&s, |d| {
        let (dpi_min, dpi_max, dpi_step) = d.dpi_range()?;
        Ok(DeviceInfo {
            connected: true,
            name: d.name.clone(),
            dpi_min,
            dpi_max,
            dpi_step,
            dpi: d.get_dpi().unwrap_or(0),
            rates: d.report_rates().unwrap_or_default(),
        })
    })
    .unwrap_or(DeviceInfo {
        connected: false,
        name: String::new(),
        dpi_min: 50,
        dpi_max: 8000,
        dpi_step: 50,
        dpi: 0,
        rates: vec![1, 2, 4, 8],
    })
}

#[tauri::command]
fn get_config(s: State<Shared>) -> Config {
    s.cfg.lock().unwrap().clone()
}

fn store_config(s: &Shared, cfg: Config) -> R<()> {
    input::ACTIVE_SLOTS.store(config::app_slots(&cfg), Ordering::Relaxed);
    config::save(&cfg)?;
    *s.cfg.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
fn save_config(s: State<Shared>, cfg: Config) -> R<()> {
    store_config(&s, cfg)
}

#[tauri::command]
fn live_dpi(s: State<Shared>, dpi: u16) -> R<()> {
    with_dev(&s, |d| d.set_dpi(dpi))
}

#[tauri::command]
fn live_rate(s: State<Shared>, ms: u8) -> R<()> {
    with_dev(&s, |d| d.set_report_rate(ms))
}

#[tauri::command]
fn live_led(s: State<Shared>, led: config::Led) -> R<()> {
    let (id, p) = profile::led_params(&led);
    with_dev(&s, |d| d.set_led(id, &p))
}

/// Make sure there is a user profile sector; returns its number.
fn ensure_profile(d: &hidpp::Device) -> R<u16> {
    if let Some(s) = d.profile_sector()? {
        return Ok(s);
    }
    let mut p = d.read_sector(0x0101)?; // factory ROM profile
    hidpp::seal(&mut p);
    d.write_sector(0x0001, &p)?;
    let mut dir = vec![0xffu8; d.sector_size as usize];
    dir[..4].copy_from_slice(&[0x00, 0x01, 0x01, 0x00]);
    hidpp::seal(&mut dir);
    d.write_sector(0x0000, &dir)?;
    Ok(0x0001)
}

#[tauri::command]
fn write_mouse(s: State<Shared>, cfg: Config) -> R<()> {
    with_dev(&s, |d| {
        let sector = ensure_profile(d)?;
        let mut data = d.read_sector(sector)?;
        let backup = config::dir().join("profile-backup.bin");
        if !backup.exists() {
            std::fs::write(&backup, &data).map_err(|e| e.to_string())?;
        }
        profile::apply(&mut data, &cfg);
        d.write_sector(sector, &data)?;
        d.reload_profile(sector)
    })?;
    store_config(&s, cfg)
}

#[tauri::command]
fn read_mouse(s: State<Shared>) -> R<Config> {
    let current = s.cfg.lock().unwrap().clone();
    with_dev(&s, |d| {
        let sector = d.profile_sector()?.unwrap_or(0x0101);
        Ok(profile::read_into(&d.read_sector(sector)?, &current))
    })
}

#[tauri::command]
fn restore_backup(s: State<Shared>) -> R<()> {
    let data = std::fs::read(config::dir().join("profile-backup.bin")).map_err(|_| "Резервной копии нет")?;
    with_dev(&s, |d| {
        let sector = d.profile_sector()?.ok_or("Нет профиля в мыши")?;
        if data.len() != d.sector_size as usize {
            return Err("Копия не подходит к этой мыши".into());
        }
        d.write_sector(sector, &data)?;
        d.reload_profile(sector)
    })
}

#[tauri::command]
fn discord_connect(s: State<Shared>) {
    s.discord.send(discord::Cmd::Connect);
}

#[tauri::command]
fn discord_action(s: State<Shared>, what: String) {
    s.discord.send(if what == "deafen" { discord::Cmd::ToggleDeaf } else { discord::Cmd::ToggleMute });
}

#[tauri::command]
fn test_action(app: AppHandle, action: config::Action) {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        actions::run(&action, true, &app, &flag);
        std::thread::sleep(Duration::from_millis(80));
        actions::run(&action, false, &app, &flag);
    });
}

#[tauri::command]
fn set_autostart(on: bool) -> R<()> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new("reg");
    if on {
        cmd.args(["add", key, "/v", "GLite", "/t", "REG_SZ", "/f", "/d"])
            .arg(format!("\"{}\" --minimized", exe.display()));
    } else {
        cmd.args(["delete", key, "/v", "GLite", "/f"]);
    }
    let ok = cmd.creation_flags(0x0800_0000).status().map_err(|e| e.to_string())?.success();
    if ok || !on { Ok(()) } else { Err("Не удалось изменить автозапуск".into()) }
}

/// The window (and its WebView2 processes) only exists while it is open;
/// in the tray GLite is just the Rust process.
fn show_main(app: &AppHandle) {
    let w = app.get_webview_window("main").or_else(|| {
        tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0]).ok()?.build().ok()
    });
    if let Some(w) = w {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Keeps the device handle alive across unplug/replug and tells the UI.
fn spawn_device_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut was = None;
        loop {
            let shared = app.state::<Shared>();
            let connected = {
                let mut g = shared.dev.lock().unwrap();
                if g.as_ref().is_some_and(|d| !d.ping()) {
                    *g = None;
                }
                if g.is_none() {
                    *g = hidpp::Device::open().ok();
                }
                g.is_some()
            };
            if was != Some(connected) {
                was = Some(connected);
                let _ = app.emit("device", connected);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn main() {
    let cfg = config::load();
    let minimized = std::env::args().any(|a| a == "--minimized") || cfg.start_minimized;
    input::ACTIVE_SLOTS.store(config::app_slots(&cfg), Ordering::Relaxed);

    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();
            app.manage(Shared {
                dev: Mutex::new(hidpp::Device::open().ok()),
                cfg: Mutex::new(cfg),
                discord: discord::spawn(handle.clone()),
            });

            let (tx, rx) = std::sync::mpsc::channel();
            input::start_hook(tx);
            actions::spawn_executor(handle.clone(), rx);
            spawn_device_monitor(handle.clone());

            let show = MenuItem::with_id(app, "show", "Открыть GLite", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("GLite")
                .menu(&Menu::with_items(app, &[&show, &quit])?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            if !minimized {
                show_main(&handle);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_device,
            get_config,
            save_config,
            live_dpi,
            live_rate,
            live_led,
            write_mouse,
            read_mouse,
            restore_backup,
            discord_connect,
            discord_action,
            test_action,
            set_autostart
        ])
        .build(tauri::generate_context!())
        .expect("error while running GLite")
        .run(|_, e| {
            // closing the window keeps GLite in the tray; only "Выход" quits
            if let RunEvent::ExitRequested { api, code: None, .. } = e {
                api.prevent_exit();
            }
        });
}
