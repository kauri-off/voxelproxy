use std::ptr::null_mut;
use std::sync::mpsc::Sender;
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
use windows::Win32::UI::Input::KeyboardAndMouse::MOD_ALT;
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, VK_F10};
use windows::Win32::UI::WindowsAndMessaging::GetMessageW;
use windows::Win32::UI::WindowsAndMessaging::MSG;
use windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
use windows::Win32::UI::WindowsAndMessaging::WM_HOTKEY;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

#[derive(PartialEq, Eq, Debug)]
enum State {
    Opened,
    Closed,
}

impl State {
    fn swap(&mut self) {
        *self = match *self {
            State::Opened => State::Closed,
            State::Closed => State::Opened,
        }
    }

    fn cmd(&self) -> SHOW_WINDOW_CMD {
        match self {
            State::Opened => SW_HIDE,
            State::Closed => SW_SHOW,
        }
    }
}

pub unsafe fn setup_keybind(tx: Sender<String>) {
    if let Err(e) = RegisterHotKey(None, 1, MOD_ALT | MOD_NOREPEAT, VK_F10.0 as u32) {
        tx.send(format!("Ошибка при установке горячей клавиши: {}", e))
            .unwrap();
        return;
    }

    tx.send(format!(
        "Установлена горячая клавиша ALT+F10 для скрытия/показа окна консоли"
    ))
    .unwrap();
    let mut state = State::Opened;
    let hwnd = GetConsoleWindow();

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg as *mut MSG, None, 0, 0).as_bool() {
        if msg.message == WM_HOTKEY {
            if hwnd.0 != null_mut() {
                let _ = ShowWindow(hwnd, state.cmd());
                state.swap();
            }
        }
    }
}
