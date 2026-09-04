//! Translation from ScarletUI input events to the GameStream input protocol.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use moonlight_sys::{
    ConnectionControl, InputAction, InputError, KeyboardModifiers as RemoteModifiers,
    MouseButton as RemoteMouseButton,
};
use scarlet_ui::event::KeyModifiers;
use scarlet_ui::{KeyCode, KeyEvent, MouseButton};

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_INSERT: u16 = 0x2D;
const VK_DELETE: u16 = 0x2E;
const VK_F1: u16 = 0x70;
const VK_OEM_1: u16 = 0xBA;
const VK_OEM_PLUS: u16 = 0xBB;
const VK_OEM_COMMA: u16 = 0xBC;
const VK_OEM_MINUS: u16 = 0xBD;
const VK_OEM_PERIOD: u16 = 0xBE;
const VK_OEM_2: u16 = 0xBF;
const VK_OEM_3: u16 = 0xC0;
const VK_OEM_4: u16 = 0xDB;
const VK_OEM_5: u16 = 0xDC;
const VK_OEM_6: u16 = 0xDD;
const VK_OEM_7: u16 = 0xDE;
const SCARLET_UI_WHEEL_UNITS: i64 = 32;
const WINDOWS_WHEEL_UNITS: i64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamShortcut {
    ToggleCapture,
    Disconnect,
    ToggleFullscreen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShortcutDispatch {
    NotShortcut,
    Consumed,
    Command(StreamShortcut),
}

#[derive(Default)]
struct PressedInput {
    keys: BTreeSet<u16>,
    mouse_buttons: u8,
    suppressed_mouse_buttons: u8,
    suppressed_shortcut_keys: BTreeSet<u16>,
}

/// Session-local tracking for input that has been sent to the remote host.
#[derive(Clone, Default)]
pub(crate) struct RemoteInput {
    state: Arc<Mutex<PressedInput>>,
}

impl RemoteInput {
    pub(crate) fn reset(&self) {
        *lock(&self.state) = PressedInput::default();
    }

    pub(crate) fn classify_shortcut(&self, event: KeyEvent) -> ShortcutDispatch {
        let (keycode, modifiers, pressed) = match event {
            KeyEvent::Pressed { keycode, modifiers } => (keycode, modifiers, true),
            KeyEvent::Released { keycode, modifiers } => (keycode, modifiers, false),
            KeyEvent::Char { .. } => return ShortcutDispatch::NotShortcut,
        };
        let Some(vk_code) = keycode_to_vk(keycode) else {
            return ShortcutDispatch::NotShortcut;
        };

        let mut state = lock(&self.state);
        if !pressed && state.suppressed_shortcut_keys.remove(&vk_code) {
            return ShortcutDispatch::Consumed;
        }
        if !pressed || !moonlight_shortcut_modifiers(modifiers) {
            return ShortcutDispatch::NotShortcut;
        }
        let command = match vk_code {
            0x5A => StreamShortcut::ToggleCapture,
            0x51 => StreamShortcut::Disconnect,
            0x58 => StreamShortcut::ToggleFullscreen,
            _ => return ShortcutDispatch::NotShortcut,
        };
        if !state.suppressed_shortcut_keys.insert(vk_code) {
            return ShortcutDispatch::Consumed;
        }
        ShortcutDispatch::Command(command)
    }

    pub(crate) fn send_key(
        &self,
        control: &ConnectionControl,
        event: KeyEvent,
    ) -> Result<bool, InputError> {
        let (keycode, modifiers, action) = match event {
            KeyEvent::Pressed { keycode, modifiers } => (keycode, modifiers, InputAction::Press),
            KeyEvent::Released { keycode, modifiers } => (keycode, modifiers, InputAction::Release),
            KeyEvent::Char { .. } => return Ok(false),
        };
        let Some(vk_code) = keycode_to_vk(keycode) else {
            return Ok(false);
        };
        control.send_keyboard(vk_code, action, remote_modifiers(modifiers))?;

        let mut state = lock(&self.state);
        match action {
            InputAction::Press => {
                state.keys.insert(vk_code);
            }
            InputAction::Release => {
                state.keys.remove(&vk_code);
            }
        }
        Ok(true)
    }

    pub(crate) fn suppress_capture_click(&self, button: MouseButton) {
        lock(&self.state).suppressed_mouse_buttons |= mouse_button_bit(button);
    }

    pub(crate) fn consume_suppressed_mouse_release(&self, button: MouseButton) -> bool {
        let bit = mouse_button_bit(button);
        let mut state = lock(&self.state);
        if state.suppressed_mouse_buttons & bit == 0 {
            return false;
        }
        state.suppressed_mouse_buttons &= !bit;
        true
    }

    pub(crate) fn send_mouse_button(
        &self,
        control: &ConnectionControl,
        button: MouseButton,
        pressed: bool,
    ) -> Result<(), InputError> {
        let action = if pressed {
            InputAction::Press
        } else {
            InputAction::Release
        };
        control.send_mouse_button(remote_mouse_button(button), action)?;

        let bit = mouse_button_bit(button);
        let mut state = lock(&self.state);
        if pressed {
            state.mouse_buttons |= bit;
        } else {
            state.mouse_buttons &= !bit;
        }
        Ok(())
    }

    pub(crate) fn send_mouse_delta(
        &self,
        control: &ConnectionControl,
        delta_x: i32,
        delta_y: i32,
    ) -> Result<(), InputError> {
        control.send_mouse_move(clamp_i16(delta_x), clamp_i16(delta_y))
    }

    pub(crate) fn send_wheel(
        &self,
        control: &ConnectionControl,
        delta_x: i32,
        delta_y: i32,
    ) -> Result<(), InputError> {
        let horizontal = wheel_units(delta_x);
        let vertical = wheel_units(delta_y).saturating_neg();
        if horizontal != 0 {
            control.send_horizontal_scroll(horizontal)?;
        }
        if vertical != 0 {
            control.send_vertical_scroll(vertical)?;
        }
        Ok(())
    }

    pub(crate) fn release_all(
        &self,
        control: Option<&ConnectionControl>,
    ) -> Result<(), InputError> {
        let (keys, mouse_buttons) = {
            let mut state = lock(&self.state);
            let keys = std::mem::take(&mut state.keys);
            let mouse_buttons = std::mem::take(&mut state.mouse_buttons);
            state.suppressed_mouse_buttons = 0;
            (keys, mouse_buttons)
        };
        let Some(control) = control else {
            return Ok(());
        };

        let mut first_error = None;
        for key_code in keys {
            if let Err(error) =
                control.send_keyboard(key_code, InputAction::Release, RemoteModifiers::default())
            {
                first_error.get_or_insert(error);
            }
        }
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            if mouse_buttons & mouse_button_bit(button) != 0
                && let Err(error) =
                    control.send_mouse_button(remote_mouse_button(button), InputAction::Release)
            {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn keycode_to_vk(keycode: KeyCode) -> Option<u16> {
    match keycode {
        KeyCode::Unknown => None,
        KeyCode::Escape => Some(VK_ESCAPE),
        KeyCode::Enter => Some(VK_RETURN),
        KeyCode::Tab => Some(VK_TAB),
        KeyCode::Backspace => Some(VK_BACK),
        KeyCode::Space => Some(VK_SPACE),
        KeyCode::Left => Some(VK_LEFT),
        KeyCode::Right => Some(VK_RIGHT),
        KeyCode::Up => Some(VK_UP),
        KeyCode::Down => Some(VK_DOWN),
        KeyCode::Home => Some(VK_HOME),
        KeyCode::End => Some(VK_END),
        KeyCode::PageUp => Some(VK_PRIOR),
        KeyCode::PageDown => Some(VK_NEXT),
        KeyCode::Insert => Some(VK_INSERT),
        KeyCode::Delete => Some(VK_DELETE),
        KeyCode::F(number @ 1..=24) => Some(VK_F1 + u16::from(number - 1)),
        KeyCode::F(_) => None,
        KeyCode::Char(character) => char_to_vk(character),
    }
}

fn char_to_vk(character: char) -> Option<u16> {
    match character {
        'a'..='z' => Some(character.to_ascii_uppercase() as u16),
        'A'..='Z' | '0'..='9' => Some(character as u16),
        '\u{1}'..='\u{1a}' => Some(u16::from(character as u8 - 1) + u16::from(b'A')),
        '\u{1b}' => Some(VK_OEM_4),
        '\u{1c}' => Some(VK_OEM_5),
        '\u{1d}' => Some(VK_OEM_6),
        '\u{1f}' => Some(VK_OEM_MINUS),
        '!' => Some(u16::from(b'1')),
        '@' => Some(u16::from(b'2')),
        '#' => Some(u16::from(b'3')),
        '$' => Some(u16::from(b'4')),
        '%' => Some(u16::from(b'5')),
        '^' => Some(u16::from(b'6')),
        '&' => Some(u16::from(b'7')),
        '*' => Some(u16::from(b'8')),
        '(' => Some(u16::from(b'9')),
        ')' => Some(u16::from(b'0')),
        ';' | ':' => Some(VK_OEM_1),
        '=' | '+' => Some(VK_OEM_PLUS),
        ',' | '<' => Some(VK_OEM_COMMA),
        '-' | '_' => Some(VK_OEM_MINUS),
        '.' | '>' => Some(VK_OEM_PERIOD),
        '/' | '?' => Some(VK_OEM_2),
        '`' | '~' => Some(VK_OEM_3),
        '[' | '{' => Some(VK_OEM_4),
        '\\' | '|' => Some(VK_OEM_5),
        ']' | '}' => Some(VK_OEM_6),
        '\'' | '"' => Some(VK_OEM_7),
        _ => None,
    }
}

fn remote_modifiers(modifiers: KeyModifiers) -> RemoteModifiers {
    RemoteModifiers::new(
        modifiers.shift,
        modifiers.control,
        modifiers.alt,
        modifiers.super_key,
    )
}

fn moonlight_shortcut_modifiers(modifiers: KeyModifiers) -> bool {
    modifiers.shift && modifiers.control && modifiers.alt
}

fn remote_mouse_button(button: MouseButton) -> RemoteMouseButton {
    match button {
        MouseButton::Left => RemoteMouseButton::Left,
        MouseButton::Middle => RemoteMouseButton::Middle,
        MouseButton::Right => RemoteMouseButton::Right,
    }
}

fn mouse_button_bit(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0x01,
        MouseButton::Middle => 0x02,
        MouseButton::Right => 0x04,
    }
}

fn clamp_i16(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn wheel_units(value: i32) -> i16 {
    let scaled = i64::from(value).saturating_mul(WINDOWS_WHEEL_UNITS);
    let rounded = if scaled >= 0 {
        scaled.saturating_add(SCARLET_UI_WHEEL_UNITS / 2) / SCARLET_UI_WHEEL_UNITS
    } else {
        scaled.saturating_sub(SCARLET_UI_WHEEL_UNITS / 2) / SCARLET_UI_WHEEL_UNITS
    };
    rounded.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut_modifiers() -> KeyModifiers {
        KeyModifiers {
            shift: true,
            control: true,
            alt: true,
            super_key: false,
        }
    }

    #[test]
    fn maps_scarlet_keys_to_win32_virtual_keys() {
        assert_eq!(keycode_to_vk(KeyCode::Char('w')), Some(0x57));
        assert_eq!(keycode_to_vk(KeyCode::Char('W')), Some(0x57));
        assert_eq!(keycode_to_vk(KeyCode::Char('@')), Some(0x32));
        assert_eq!(keycode_to_vk(KeyCode::Char('\u{1}')), Some(0x41));
        assert_eq!(keycode_to_vk(KeyCode::Left), Some(VK_LEFT));
        assert_eq!(keycode_to_vk(KeyCode::F(12)), Some(0x7B));
        assert_eq!(keycode_to_vk(KeyCode::F(25)), None);
    }

    #[test]
    fn scales_scarlet_wheel_units_to_windows_units() {
        assert_eq!(wheel_units(32), 120);
        assert_eq!(wheel_units(-32), -120);
        assert_eq!(wheel_units(1), 4);
    }

    #[test]
    fn shortcut_press_repeats_and_release_are_consumed_once() {
        let input = RemoteInput::default();
        let press = KeyEvent::Pressed {
            keycode: KeyCode::Char('z'),
            modifiers: shortcut_modifiers(),
        };
        let release_without_modifiers = KeyEvent::Released {
            keycode: KeyCode::Char('z'),
            modifiers: KeyModifiers::empty(),
        };

        assert_eq!(
            input.classify_shortcut(press),
            ShortcutDispatch::Command(StreamShortcut::ToggleCapture)
        );
        assert_eq!(input.classify_shortcut(press), ShortcutDispatch::Consumed);
        assert_eq!(
            input.classify_shortcut(release_without_modifiers),
            ShortcutDispatch::Consumed
        );
        assert_eq!(
            input.classify_shortcut(release_without_modifiers),
            ShortcutDispatch::NotShortcut
        );
    }
}
