//! Maps egui input events to ironrdp-input `Operation`s.

use egui::{Key, PointerButton};
use ironrdp_input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};

/// Convert an egui pointer button to an ironrdp mouse button.
pub fn mouse_button(button: PointerButton) -> Option<MouseButton> {
    match button {
        PointerButton::Primary => Some(MouseButton::Left),
        PointerButton::Secondary => Some(MouseButton::Right),
        PointerButton::Middle => Some(MouseButton::Middle),
        PointerButton::Extra1 => Some(MouseButton::X1),
        PointerButton::Extra2 => Some(MouseButton::X2),
    }
}

pub fn mouse_move(x: u16, y: u16) -> Operation {
    Operation::MouseMove(MousePosition { x, y })
}

pub fn wheel(delta_x: f32, delta_y: f32) -> Vec<Operation> {
    // RDP expects rotation units; one notch is ~120 units.
    let mut ops = Vec::new();
    if delta_y != 0.0 {
        ops.push(Operation::WheelRotations(WheelRotations {
            is_vertical: true,
            rotation_units: (delta_y * 120.0) as i16,
        }));
    }
    if delta_x != 0.0 {
        ops.push(Operation::WheelRotations(WheelRotations {
            is_vertical: false,
            rotation_units: (delta_x * 120.0) as i16,
        }));
    }
    ops
}

/// Map an egui `Key` to a PC/AT set-1 scancode.
/// Returns the raw 16-bit scancode (0xE0xx for extended keys).
pub fn key_scancode(key: Key) -> Option<u16> {
    let code = match key {
        Key::Escape => 0x01,
        Key::Num1 => 0x02,
        Key::Num2 => 0x03,
        Key::Num3 => 0x04,
        Key::Num4 => 0x05,
        Key::Num5 => 0x06,
        Key::Num6 => 0x07,
        Key::Num7 => 0x08,
        Key::Num8 => 0x09,
        Key::Num9 => 0x0A,
        Key::Num0 => 0x0B,
        Key::Minus => 0x0C,
        Key::Equals => 0x0D,
        Key::Backspace => 0x0E,
        Key::Tab => 0x0F,
        Key::Q => 0x10,
        Key::W => 0x11,
        Key::E => 0x12,
        Key::R => 0x13,
        Key::T => 0x14,
        Key::Y => 0x15,
        Key::U => 0x16,
        Key::I => 0x17,
        Key::O => 0x18,
        Key::P => 0x19,
        Key::OpenBracket => 0x1A,
        Key::CloseBracket => 0x1B,
        Key::Enter => 0x1C,
        Key::A => 0x1E,
        Key::S => 0x1F,
        Key::D => 0x20,
        Key::F => 0x21,
        Key::G => 0x22,
        Key::H => 0x23,
        Key::J => 0x24,
        Key::K => 0x25,
        Key::L => 0x26,
        Key::Semicolon => 0x27,
        Key::Backtick => 0x29,
        Key::Backslash => 0x2B,
        Key::Z => 0x2C,
        Key::X => 0x2D,
        Key::C => 0x2E,
        Key::V => 0x2F,
        Key::B => 0x30,
        Key::N => 0x31,
        Key::M => 0x32,
        Key::Comma => 0x33,
        Key::Period => 0x34,
        Key::Slash => 0x35,
        Key::Space => 0x39,
        Key::F1 => 0x3B,
        Key::F2 => 0x3C,
        Key::F3 => 0x3D,
        Key::F4 => 0x3E,
        Key::F5 => 0x3F,
        Key::F6 => 0x40,
        Key::F7 => 0x41,
        Key::F8 => 0x42,
        Key::F9 => 0x43,
        Key::F10 => 0x44,
        Key::F11 => 0x57,
        Key::F12 => 0x58,
        // Extended keys (0xE0 prefix).
        Key::Insert => 0xE052,
        Key::Delete => 0xE053,
        Key::Home => 0xE047,
        Key::End => 0xE04F,
        Key::PageUp => 0xE049,
        Key::PageDown => 0xE051,
        Key::ArrowUp => 0xE048,
        Key::ArrowDown => 0xE050,
        Key::ArrowLeft => 0xE04B,
        Key::ArrowRight => 0xE04D,
        _ => return None,
    };
    Some(code)
}

/// Map an egui modifier to its scancode. egui does not expose modifier
/// key events individually, so we synthesize them from `Modifiers`.
pub fn modifier_scancodes(mods: &egui::Modifiers) -> Vec<u16> {
    let mut codes = Vec::new();
    if mods.shift {
        codes.push(0x2A); // Left Shift
    }
    if mods.ctrl || mods.command {
        codes.push(0x1D); // Left Ctrl
    }
    if mods.alt {
        codes.push(0x38); // Left Alt
    }
    codes
}

pub fn key_pressed(scancode: u16) -> Operation {
    Operation::KeyPressed(Scancode::from_u16(scancode))
}

pub fn key_released(scancode: u16) -> Operation {
    Operation::KeyReleased(Scancode::from_u16(scancode))
}
