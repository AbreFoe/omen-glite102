//! Low-level keyboard hook that catches F13..F24 sent by the mouse firmware,
//! plus SendInput helpers used by actions.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const VK_F13: u32 = 0x7c;
/// Marks input we inject so the hook ignores it.
const MAGIC: usize = 0x474c_6974;

/// Bit n set => button n is bound to an app action and its F13+n must be swallowed.
pub static ACTIVE_SLOTS: AtomicU32 = AtomicU32::new(0);
static TX: OnceLock<Sender<(usize, bool)>> = OnceLock::new();

unsafe extern "system" fn hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == HC_ACTION as i32 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        if k.dwExtraInfo != MAGIC && (VK_F13..VK_F13 + 12).contains(&k.vkCode) {
            let slot = (k.vkCode - VK_F13) as usize;
            if ACTIVE_SLOTS.load(Ordering::Relaxed) & (1 << slot) != 0 {
                let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                if let Some(tx) = TX.get() {
                    let _ = tx.send((slot, down));
                }
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

pub fn start_hook(tx: Sender<(usize, bool)>) {
    let _ = TX.set(tx);
    std::thread::spawn(|| unsafe {
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {}
    });
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

fn is_extended(vk: u16) -> bool {
    matches!(vk, 0x21..=0x2e | 0x5b | 0x5c | 0x5d | 0x6f | 0x90 | 0xa3 | 0xa5 | 0xa6..=0xb7)
}

fn mouse_input(flags: u32, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: MAGIC },
        },
    }
}

fn key_input(vk: u16, up: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

/// Press or release a virtual key; VK 1/2/4/5/6 are mouse buttons.
pub fn vk(vk: u16, up: bool) {
    let i = match vk {
        0x01 => mouse_input(if up { MOUSEEVENTF_LEFTUP } else { MOUSEEVENTF_LEFTDOWN }, 0),
        0x02 => mouse_input(if up { MOUSEEVENTF_RIGHTUP } else { MOUSEEVENTF_RIGHTDOWN }, 0),
        0x04 => mouse_input(if up { MOUSEEVENTF_MIDDLEUP } else { MOUSEEVENTF_MIDDLEDOWN }, 0),
        0x05 | 0x06 => mouse_input(if up { MOUSEEVENTF_XUP } else { MOUSEEVENTF_XDOWN }, (vk - 4) as u32),
        _ => key_input(vk, up),
    };
    send(&[i]);
}

pub fn combo(keys: &[u16], up: bool) {
    if up {
        keys.iter().rev().for_each(|k| vk(*k, true));
    } else {
        keys.iter().for_each(|k| vk(*k, false));
    }
}

pub fn tap_combo(keys: &[u16]) {
    combo(keys, false);
    std::thread::sleep(std::time::Duration::from_millis(30));
    combo(keys, true);
}

pub fn type_text(text: &str) {
    for u in text.encode_utf16() {
        let mk = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: u,
                    dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: MAGIC,
                },
            },
        };
        send(&[mk(false), mk(true)]);
    }
}
