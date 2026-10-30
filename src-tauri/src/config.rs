use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    #[default]
    None,
    /// Hold a key combination while the button is held.
    Keys { keys: Vec<u16> },
    Text { text: String },
    Macro {
        steps: Vec<Step>,
        /// "once" | "while_held" | "toggle"
        #[serde(default)]
        repeat: String,
    },
    Launch { target: String },
    DiscordMute,
    DiscordDeafen,
    DiscordPtt,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Step {
    /// "down" | "up" | "tap" | "delay"
    pub kind: String,
    #[serde(default)]
    pub vk: u16,
    #[serde(default)]
    pub ms: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Binding {
    Mouse { bit: u16 },
    Special { code: u8 },
    Disabled,
    App { action: Action },
    Raw { bytes: [u8; 4] },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Led {
    /// "off" | "static" | "cycle" | "breathing"
    pub effect: String,
    pub color: String,
    pub period: u16,
    pub brightness: u8,
}

impl Default for Led {
    fn default() -> Self {
        Led { effect: "static".into(), color: "#7c4dff".into(), period: 3000, brightness: 100 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Discord {
    /// "rpc" | "hotkey"
    pub mode: String,
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: String,
    pub mute_keys: Vec<u16>,
    pub deafen_keys: Vec<u16>,
}

impl Default for Discord {
    fn default() -> Self {
        Discord {
            mode: "hotkey".into(),
            client_id: String::new(),
            client_secret: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            mute_keys: vec![],
            deafen_keys: vec![],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub dpi_stages: Vec<u16>,
    pub default_stage: usize,
    pub shift_stage: usize,
    pub report_rate: u8,
    pub led: Led,
    pub buttons: Vec<Binding>,
    pub discord: Discord,
    /// "auto" (follow LED colour) or "#rrggbb"
    pub theme_seed: String,
    /// "system" | "light" | "dark"
    pub theme_mode: String,
    pub start_minimized: bool,
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dpi_stages: vec![400, 800, 1600, 3200],
            default_stage: 1,
            shift_stage: 0,
            report_rate: 1,
            led: Led::default(),
            buttons: vec![
                Binding::Mouse { bit: 0x01 },
                Binding::Mouse { bit: 0x02 },
                Binding::Mouse { bit: 0x04 },
                Binding::Mouse { bit: 0x08 },
                Binding::Mouse { bit: 0x10 },
                Binding::Special { code: 0x05 },
            ],
            discord: Discord::default(),
            theme_seed: "auto".into(),
            theme_mode: "system".into(),
            start_minimized: false,
            autostart: false,
        }
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let d = PathBuf::from(base).join("GLite");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load() -> Config {
    std::fs::read_to_string(dir().join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(c: &Config) -> Result<(), String> {
    let s = serde_json::to_string_pretty(c).map_err(|e| e.to_string())?;
    std::fs::write(dir().join("config.json"), s).map_err(|e| e.to_string())
}

/// Bitmask of buttons that send F13+n for the app to handle.
pub fn app_slots(c: &Config) -> u32 {
    c.buttons.iter().enumerate().filter(|(_, b)| matches!(b, Binding::App { .. })).fold(0, |m, (i, _)| m | 1 << i)
}
