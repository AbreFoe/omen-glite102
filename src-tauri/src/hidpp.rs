//! Minimal HID++ 2.0 client for wired Logitech G mice (G102 / G203 family).

use hidapi::{HidApi, HidDevice};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const VID: u16 = 0x046d;
// G102 LIGHTSYNC, G102/G203 Prodigy, G203 LIGHTSYNC
const PIDS: &[(u16, &str)] = &[
    (0xc092, "G102 LIGHTSYNC"),
    (0xc084, "G102 Prodigy"),
    (0xc09d, "G203 LIGHTSYNC"),
];
const SW_ID: u8 = 0x0a;

pub const F_ADJUSTABLE_DPI: u16 = 0x2201;
pub const F_REPORT_RATE: u16 = 0x8060;
pub const F_RGB_EFFECTS: u16 = 0x8071;
pub const F_ONBOARD_PROFILES: u16 = 0x8100;

type R<T> = Result<T, String>;

pub struct Device {
    dev: HidDevice,
    feats: HashMap<u16, u8>,
    pub name: String,
    /// effect id -> effect index on RGB zone 0
    pub effects: Vec<u16>,
    pub sector_size: u16,
}

impl Device {
    pub fn open() -> R<Device> {
        let api = HidApi::new().map_err(|e| e.to_string())?;
        for info in api.device_list() {
            if info.vendor_id() != VID || info.usage_page() != 0xff00 || info.usage() != 0x0002 {
                continue;
            }
            let Some(&(_, name)) = PIDS.iter().find(|(p, _)| *p == info.product_id()) else { continue };
            let dev = info.open_device(&api).map_err(|e| e.to_string())?;
            let mut d = Device { dev, feats: HashMap::new(), name: name.into(), effects: vec![], sector_size: 255 };
            d.init()?;
            return Ok(d);
        }
        Err("Мышь не найдена".into())
    }

    fn init(&mut self) -> R<()> {
        for f in [F_ADJUSTABLE_DPI, F_REPORT_RATE, F_RGB_EFFECTS, F_ONBOARD_PROFILES] {
            let r = self.request(0, 0, &[(f >> 8) as u8, f as u8])?;
            if r[0] != 0 {
                self.feats.insert(f, r[0]);
            }
        }
        if let Some(&fi) = self.feats.get(&F_RGB_EFFECTS) {
            let zone = self.request(fi, 0, &[0, 0xff, 0])?;
            for e in 0..zone[4] {
                let r = self.request(fi, 0, &[0, e, 0])?;
                self.effects.push(u16::from_be_bytes([r[2], r[3]]));
            }
        }
        if let Some(&fi) = self.feats.get(&F_ONBOARD_PROFILES) {
            let r = self.request(fi, 0, &[])?;
            self.sector_size = u16::from_be_bytes([r[7], r[8]]);
        }
        Ok(())
    }

    fn feat(&self, id: u16) -> R<u8> {
        self.feats.get(&id).copied().ok_or_else(|| format!("Функция 0x{id:04x} не поддерживается"))
    }

    pub fn request(&self, feat: u8, func: u8, params: &[u8]) -> R<[u8; 16]> {
        let mut buf = [0u8; 20];
        buf[0] = 0x11;
        buf[1] = 0xff;
        buf[2] = feat;
        buf[3] = (func << 4) | SW_ID;
        buf[4..4 + params.len()].copy_from_slice(params);
        self.dev.write(&buf).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_millis(1000);
        let mut r = [0u8; 20];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("Мышь не отвечает".into());
            }
            let n = self.dev.read_timeout(&mut r, left.as_millis() as i32).map_err(|e| e.to_string())?;
            if n < 6 {
                continue;
            }
            if r[2] == feat && r[3] == buf[3] {
                let mut out = [0u8; 16];
                out.copy_from_slice(&r[4..20]);
                return Ok(out);
            }
            if r[2] == 0xff && r[3] == feat && r[4] == buf[3] {
                return Err(format!("Ошибка HID++ {}", r[5]));
            }
        }
    }

    pub fn ping(&self) -> bool {
        self.request(0, 0, &[0, 0]).is_ok()
    }

    // ---- DPI ----

    /// (min, max, step)
    pub fn dpi_range(&self) -> R<(u16, u16, u16)> {
        let f = self.feat(F_ADJUSTABLE_DPI)?;
        let r = self.request(f, 1, &[0])?;
        let vals: Vec<u16> = r[1..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).take_while(|v| *v != 0).collect();
        let mut step = 50;
        let mut list = vec![];
        for v in vals {
            if v >> 13 == 0b111 {
                step = v & 0x1fff;
            } else {
                list.push(v);
            }
        }
        let min = *list.iter().min().unwrap_or(&100);
        let max = *list.iter().max().unwrap_or(&8000);
        Ok((min, max, step))
    }

    pub fn get_dpi(&self) -> R<u16> {
        let f = self.feat(F_ADJUSTABLE_DPI)?;
        let r = self.request(f, 2, &[0])?;
        Ok(u16::from_be_bytes([r[1], r[2]]))
    }

    pub fn set_dpi(&self, dpi: u16) -> R<()> {
        let f = self.feat(F_ADJUSTABLE_DPI)?;
        self.request(f, 3, &[0, (dpi >> 8) as u8, dpi as u8]).map(|_| ())
    }

    // ---- report rate ----

    /// Supported intervals in ms.
    pub fn report_rates(&self) -> R<Vec<u8>> {
        let f = self.feat(F_REPORT_RATE)?;
        let r = self.request(f, 0, &[])?;
        Ok((0..8).filter(|i| r[0] & (1 << i) != 0).map(|i| i + 1).collect())
    }

    pub fn set_report_rate(&self, ms: u8) -> R<()> {
        let f = self.feat(F_REPORT_RATE)?;
        self.request(f, 2, &[ms]).map(|_| ())
    }

    // ---- RGB ----

    /// Apply an effect immediately (not persisted; the onboard profile holds the saved one).
    pub fn set_led(&self, effect_id: u16, params: &[u8; 10]) -> R<()> {
        let f = self.feat(F_RGB_EFFECTS)?;
        let idx = self.effects.iter().position(|e| *e == effect_id).ok_or("Эффект не поддерживается")?;
        let mut p = vec![0u8, idx as u8];
        p.extend_from_slice(params);
        p.push(0);
        self.request(f, 1, &p).map(|_| ())
    }

    // ---- onboard profiles ----

    pub fn read_sector(&self, sector: u16) -> R<Vec<u8>> {
        let f = self.feat(F_ONBOARD_PROFILES)?;
        let size = self.sector_size as usize;
        let mut out = vec![0u8; size];
        let mut off = 0;
        while off < size {
            let o = off.min(size - 16);
            let r = self.request(f, 5, &[(sector >> 8) as u8, sector as u8, (o >> 8) as u8, o as u8])?;
            out[o..o + 16].copy_from_slice(&r);
            off = o + 16;
        }
        Ok(out)
    }

    pub fn write_sector(&self, sector: u16, data: &[u8]) -> R<()> {
        let f = self.feat(F_ONBOARD_PROFILES)?;
        let len = data.len();
        self.request(f, 6, &[(sector >> 8) as u8, sector as u8, 0, 0, (len >> 8) as u8, len as u8])?;
        for chunk in data.chunks(16) {
            let mut c = [0u8; 16];
            c[..chunk.len()].copy_from_slice(chunk);
            self.request(f, 7, &c)?;
        }
        self.request(f, 8, &[])?;
        if self.read_sector(sector)? != data {
            return Err("Проверка записи не прошла".into());
        }
        Ok(())
    }

    /// Sector of the active user profile, if the directory lists one.
    pub fn profile_sector(&self) -> R<Option<u16>> {
        let dir = self.read_sector(0)?;
        for e in dir.chunks(4) {
            let s = u16::from_be_bytes([e[0], e[1]]);
            if s == 0xffff {
                break;
            }
            if s & 0xff00 == 0 && e[2] != 0 {
                return Ok(Some(s));
            }
        }
        Ok(None)
    }

    pub fn reload_profile(&self, sector: u16) -> R<()> {
        let f = self.feat(F_ONBOARD_PROFILES)?;
        self.request(f, 1, &[1])?; // onboard mode
        self.request(f, 3, &[0, sector as u8]).map(|_| ())
    }
}

pub fn crc(data: &[u8]) -> u16 {
    let mut c: u16 = 0xffff;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

pub fn seal(sector: &mut [u8]) {
    let n = sector.len();
    let c = crc(&sector[..n - 2]);
    sector[n - 2..].copy_from_slice(&c.to_be_bytes());
}
