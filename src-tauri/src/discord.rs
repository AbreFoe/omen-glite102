//! Discord control: either via the local RPC pipe (needs your own Discord app
//! client id/secret) or by pressing the keybinds you set up in Discord.

use crate::{config, input, Shared};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Sender};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug)]
pub enum Cmd {
    Connect,
    ToggleMute,
    ToggleDeaf,
    SetMute(bool),
}

pub struct Handle(Sender<Cmd>);

impl Handle {
    pub fn send(&self, c: Cmd) {
        let _ = self.0.send(c);
    }
}

struct Ipc {
    pipe: File,
    nonce: u64,
}

impl Ipc {
    fn open(client_id: &str) -> Result<Ipc, String> {
        let pipe = (0..10)
            .find_map(|i| OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")).ok())
            .ok_or("Discord не запущен")?;
        let mut ipc = Ipc { pipe, nonce: 0 };
        ipc.write(0, &json!({ "v": 1, "client_id": client_id }))?;
        let (op, v) = ipc.read()?;
        if op != 1 || v["evt"] != "READY" {
            return Err(format!("Discord отклонил подключение: {}", v["message"].as_str().unwrap_or("?")));
        }
        Ok(ipc)
    }

    fn write(&mut self, op: u32, v: &Value) -> Result<(), String> {
        let body = v.to_string();
        let mut buf = Vec::with_capacity(body.len() + 8);
        buf.extend_from_slice(&op.to_le_bytes());
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(body.as_bytes());
        self.pipe.write_all(&buf).map_err(|e| e.to_string())
    }

    fn read(&mut self) -> Result<(u32, Value), String> {
        let mut h = [0u8; 8];
        self.pipe.read_exact(&mut h).map_err(|e| e.to_string())?;
        let op = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        let mut body = vec![0u8; len];
        self.pipe.read_exact(&mut body).map_err(|e| e.to_string())?;
        Ok((op, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    }

    fn cmd(&mut self, cmd: &str, args: Value) -> Result<Value, String> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        self.write(1, &json!({ "cmd": cmd, "args": args, "nonce": nonce }))?;
        loop {
            let (op, v) = self.read()?;
            if op == 2 {
                return Err(v["message"].as_str().unwrap_or("Discord закрыл соединение").into());
            }
            if v["nonce"] == nonce.as_str() {
                if v["evt"] == "ERROR" {
                    return Err(v["data"]["message"].as_str().unwrap_or("ошибка").into());
                }
                return Ok(v["data"].clone());
            }
        }
    }
}

fn token_request(form: &[(&str, &str)]) -> Result<(String, String), String> {
    let r = ureq::post("https://discord.com/api/oauth2/token").send_form(form);
    let v: Value = match r {
        Ok(resp) => resp.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(_, resp)) => {
            let v: Value = resp.into_json().unwrap_or(Value::Null);
            return Err(format!("OAuth: {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("ошибка")));
        }
        Err(e) => return Err(e.to_string()),
    };
    Ok((
        v["access_token"].as_str().unwrap_or_default().to_string(),
        v["refresh_token"].as_str().unwrap_or_default().to_string(),
    ))
}

struct Worker {
    app: AppHandle,
    ipc: Option<Ipc>,
    mute: bool,
    deaf: bool,
}

impl Worker {
    fn cfg(&self) -> config::Discord {
        self.app.state::<Shared>().cfg.lock().unwrap().discord.clone()
    }

    fn store_tokens(&self, access: &str, refresh: &str) {
        let shared = self.app.state::<Shared>();
        let mut c = shared.cfg.lock().unwrap();
        c.discord.access_token = access.into();
        c.discord.refresh_token = refresh.into();
        let _ = config::save(&c);
    }

    fn emit(&self, error: Option<String>) {
        let _ = self.app.emit(
            "discord",
            json!({ "connected": self.ipc.is_some(), "mute": self.mute, "deaf": self.deaf, "error": error }),
        );
    }

    fn connect(&mut self) -> Result<(), String> {
        if self.ipc.is_some() {
            return Ok(());
        }
        let d = self.cfg();
        if d.client_id.is_empty() || d.client_secret.is_empty() {
            return Err("Укажи Client ID и Client Secret".into());
        }
        let mut ipc = Ipc::open(&d.client_id)?;
        let mut ok = !d.access_token.is_empty() && ipc.cmd("AUTHENTICATE", json!({ "access_token": d.access_token })).is_ok();
        if !ok && !d.refresh_token.is_empty() {
            ipc = Ipc::open(&d.client_id)?;
            if let Ok((a, r)) = token_request(&[
                ("client_id", &d.client_id),
                ("client_secret", &d.client_secret),
                ("grant_type", "refresh_token"),
                ("refresh_token", &d.refresh_token),
            ]) {
                ok = ipc.cmd("AUTHENTICATE", json!({ "access_token": a })).is_ok();
                if ok {
                    self.store_tokens(&a, &r);
                }
            }
        }
        if !ok {
            ipc = Ipc::open(&d.client_id)?;
            let data = ipc.cmd(
                "AUTHORIZE",
                json!({ "client_id": d.client_id, "scopes": ["rpc", "rpc.voice.read", "rpc.voice.write"] }),
            )?;
            let code = data["code"].as_str().ok_or("Discord не вернул код")?.to_string();
            let base = [
                ("client_id", d.client_id.as_str()),
                ("client_secret", d.client_secret.as_str()),
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
            ];
            let mut with_redirect = base.to_vec();
            with_redirect.push(("redirect_uri", "http://localhost"));
            let (a, r) = token_request(&with_redirect).or_else(|_| token_request(&base))?;
            ipc.cmd("AUTHENTICATE", json!({ "access_token": a }))?;
            self.store_tokens(&a, &r);
        }
        let v = ipc.cmd("GET_VOICE_SETTINGS", json!({}))?;
        self.mute = v["mute"].as_bool().unwrap_or(false);
        self.deaf = v["deaf"].as_bool().unwrap_or(false);
        self.ipc = Some(ipc);
        Ok(())
    }

    fn set(&mut self, args: Value) -> Result<(), String> {
        for attempt in 0..2 {
            self.connect()?;
            match self.ipc.as_mut().unwrap().cmd("SET_VOICE_SETTINGS", args.clone()) {
                Ok(v) => {
                    self.mute = v["mute"].as_bool().unwrap_or(self.mute);
                    self.deaf = v["deaf"].as_bool().unwrap_or(self.deaf);
                    return Ok(());
                }
                Err(e) if attempt == 1 => return Err(e),
                Err(_) => self.ipc = None, // stale pipe (Discord restarted) — reconnect once
            }
        }
        Ok(())
    }

    fn handle(&mut self, cmd: Cmd) -> Result<(), String> {
        let d = self.cfg();
        if d.mode != "rpc" {
            let keys = match cmd {
                Cmd::ToggleMute | Cmd::SetMute(_) => &d.mute_keys,
                Cmd::ToggleDeaf => &d.deafen_keys,
                Cmd::Connect => return Ok(()),
            };
            if keys.is_empty() {
                return Err("Не задано сочетание клавиш для Discord".into());
            }
            input::tap_combo(keys);
            return Ok(());
        }
        match cmd {
            Cmd::Connect => {
                self.ipc = None;
                self.connect()
            }
            Cmd::ToggleMute => {
                let m = !self.mute;
                self.set(json!({ "mute": m }))
            }
            Cmd::ToggleDeaf => {
                let m = !self.deaf;
                self.set(json!({ "deaf": m }))
            }
            Cmd::SetMute(m) => self.set(json!({ "mute": m })),
        }
    }
}

pub fn spawn(app: AppHandle) -> Handle {
    let (tx, rx) = channel::<Cmd>();
    std::thread::spawn(move || {
        let mut w = Worker { app, ipc: None, mute: false, deaf: false };
        for cmd in rx {
            let r = w.handle(cmd);
            w.emit(r.err());
        }
    });
    Handle(tx)
}
