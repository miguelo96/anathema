use uefi::boot;
use uefi::proto::console::text::{Key as Raw, ScanCode};
use uefi::system::with_stdin;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Esc,
    Char(char),
}

pub fn next() -> Key {
    loop {
        let event = with_stdin(|input| input.wait_for_key_event());
        if let Some(event) = event {
            let mut events = [event];
            let _ = boot::wait_for_event(&mut events);
        } else {
            boot::stall(20_000);
        }
        let raw = with_stdin(|input| input.read_key());
        match raw {
            Ok(Some(Raw::Special(code))) => {
                let key = if code == ScanCode::UP {
                    Key::Up
                } else if code == ScanCode::DOWN {
                    Key::Down
                } else if code == ScanCode::PAGE_UP {
                    Key::PageUp
                } else if code == ScanCode::PAGE_DOWN {
                    Key::PageDown
                } else if code == ScanCode::HOME {
                    Key::Home
                } else if code == ScanCode::END {
                    Key::End
                } else if code == ScanCode::ESCAPE {
                    Key::Esc
                } else {
                    continue;
                };
                return key;
            }
            Ok(Some(Raw::Printable(c))) => {
                let c = char::from(c);
                return if c == '\r' || c == '\n' { Key::Enter } else { Key::Char(c) };
            }
            _ => {}
        }
    }
}
