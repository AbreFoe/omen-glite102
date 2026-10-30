//! Onboard profile (format v4) <-> Config. Only the fields we understand are touched;
//! everything else in the sector is preserved byte for byte.

use crate::config::{Action, Binding, Config, Led};
use crate::hidpp::seal;

const DPI_OFF: usize = 3;
const BTN_OFF: usize = 32;
const LED_OFF: usize = 208;
const LED_ALT_OFF: usize = 230;
const BUTTONS: usize = 6;
/// HID usage of F13; button n is sent as F13+n when bound to an app action.
pub const F13_HID: u8 = 0x68;

pub const LED_OFF_ID: u16 = 0x00;
pub const LED_STATIC_ID: u16 = 0x01;
pub const LED_CYCLE_ID: u16 = 0x03;
pub const LED_BREATHING_ID: u16 = 0x0a;

fn hex_rgb(s: &str) -> [u8; 3] {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// Effect id and its 10 parameter bytes (same layout for RGB feature and profile).
pub fn led_params(l: &Led) -> (u16, [u8; 10]) {
    let [r, g, b] = hex_rgb(&l.color);
    let [ph, pl] = l.period.clamp(1000, 20000).to_be_bytes();
    let br = l.brightness.clamp(1, 100);
    match l.effect.as_str() {
        "static" => (LED_STATIC_ID, [r, g, b, 0, 0, 0, 0, 0, 0, 0]),
        "cycle" => (LED_CYCLE_ID, [0, 0, 0, 0, 0, ph, pl, br, 0, 0]),
        "breathing" => (LED_BREATHING_ID, [r, g, b, ph, pl, 0, br, 0, 0, 0]),
        _ => (LED_OFF_ID, [0; 10]),
    }
}

fn led_from(b: &[u8]) -> Led {
    let color = format!("#{:02x}{:02x}{:02x}", b[1], b[2], b[3]);
    let bright = |v: u8| if v == 0 { 100 } else { v };
    match b[0] as u16 {
        LED_STATIC_ID => Led { effect: "static".into(), color, ..Led::default() },
        LED_CYCLE_ID => Led {
            effect: "cycle".into(),
            period: u16::from_be_bytes([b[6], b[7]]),
            brightness: bright(b[8]),
            ..Led::default()
        },
        LED_BREATHING_ID => Led {
            effect: "breathing".into(),
            color,
            period: u16::from_be_bytes([b[4], b[5]]),
            brightness: bright(b[7]),
        },
        _ => Led { effect: "off".into(), ..Led::default() },
    }
}

fn encode_button(i: usize, b: &Binding) -> [u8; 4] {
    match b {
        Binding::Mouse { bit } => [0x80, 0x01, (bit >> 8) as u8, *bit as u8],
        Binding::Special { code } => [0x90, *code, 0xff, 0xff],
        Binding::Disabled => [0xff; 4],
        Binding::App { .. } => [0x80, 0x02, 0x00, F13_HID + i as u8],
        Binding::Raw { bytes } => *bytes,
    }
}

fn decode_button(i: usize, b: &[u8], current: Option<&Binding>) -> Binding {
    match (b[0], b[1]) {
        (0x80, 0x01) => Binding::Mouse { bit: u16::from_be_bytes([b[2], b[3]]) },
        (0x90, code) => Binding::Special { code },
        (0xff, 0xff) => Binding::Disabled,
        (0x80, 0x02) if b[2] == 0 && b[3] == F13_HID + i as u8 => match current {
            Some(app @ Binding::App { .. }) => app.clone(),
            _ => Binding::App { action: Action::None },
        },
        _ => Binding::Raw { bytes: [b[0], b[1], b[2], b[3]] },
    }
}

pub fn apply(sector: &mut [u8], c: &Config) {
    sector[0] = c.report_rate;
    let n = c.dpi_stages.len().clamp(1, 5);
    sector[1] = c.default_stage.min(n - 1) as u8;
    sector[2] = c.shift_stage.min(n - 1) as u8;
    for i in 0..5 {
        let v = c.dpi_stages.get(i).copied().unwrap_or(0);
        sector[DPI_OFF + i * 2..DPI_OFF + i * 2 + 2].copy_from_slice(&v.to_le_bytes());
    }
    for (i, b) in c.buttons.iter().take(BUTTONS).enumerate() {
        sector[BTN_OFF + i * 4..BTN_OFF + i * 4 + 4].copy_from_slice(&encode_button(i, b));
    }
    let (id, p) = led_params(&c.led);
    for off in [LED_OFF, LED_ALT_OFF] {
        sector[off] = id as u8;
        sector[off + 1..off + 11].copy_from_slice(&p);
    }
    seal(sector);
}

/// Merge what the mouse stores into a copy of the current config.
pub fn read_into(sector: &[u8], current: &Config) -> Config {
    let mut c = current.clone();
    c.report_rate = sector[0];
    c.dpi_stages = (0..5)
        .map(|i| u16::from_le_bytes([sector[DPI_OFF + i * 2], sector[DPI_OFF + i * 2 + 1]]))
        .take_while(|v| *v != 0 && *v != 0xffff)
        .collect();
    if c.dpi_stages.is_empty() {
        c.dpi_stages = vec![800];
    }
    c.default_stage = (sector[1] as usize).min(c.dpi_stages.len() - 1);
    c.shift_stage = (sector[2] as usize).min(c.dpi_stages.len() - 1);
    c.buttons = (0..BUTTONS)
        .map(|i| decode_button(i, &sector[BTN_OFF + i * 4..BTN_OFF + i * 4 + 4], current.buttons.get(i)))
        .collect();
    c.led = led_from(&sector[LED_OFF..LED_OFF + 11]);
    c
}
