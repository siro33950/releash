use avt::parser::{Function, Parser, State};
use std::fmt::Write;

// SwiftTerm 1.99.0 keeps at most 16 saved keyboard settings per screen.
const KEYBOARD_STACK_LIMIT: usize = 16;

#[derive(Default)]
pub(super) struct TerminalInputModes {
    parser: Parser,
    parameters: Vec<u32>,
    prefix: Option<char>,
    has_intermediate: bool,
    mouse_tracking: Option<u16>,
    mouse_encoding: Option<u16>,
    alternate_scroll: Option<bool>,
    focus: Option<bool>,
    paste: Option<bool>,
    alternate: bool,
    keyboards: [KeyboardMode; 2],
}

#[derive(Default)]
struct KeyboardMode {
    flags: u16,
    stack: Vec<u16>,
}

impl TerminalInputModes {
    pub(super) fn apply(&mut self, output: &str) {
        for ch in output.chars() {
            let previous = self.parser.state;
            let function = self.parser.feed(ch);
            if matches!(function, Some(Function::Ris)) {
                self.mouse_tracking = Some(0);
                self.alternate_scroll = Some(true);
                self.paste = Some(false);
                self.alternate = false;
                self.keyboards = Default::default();
            }
            match (previous, self.parser.state, ch) {
                (_, State::CsiEntry, _) => {
                    self.parameters.clear();
                    self.prefix = None;
                    self.has_intermediate = false;
                }
                (
                    State::CsiEntry | State::CsiParam | State::CsiIntermediate,
                    State::Ground,
                    '@'..='~',
                ) => {
                    self.dispatch(ch);
                    self.parameters.clear();
                }
                (State::CsiEntry, State::CsiParam, '<'..='?') => self.prefix = Some(ch),
                (_, State::CsiParam, '0'..='9') => {
                    if self.parameters.is_empty() {
                        self.parameters.push(0);
                    }
                    let value = self.parameters.last_mut().unwrap();
                    *value = value
                        .saturating_mul(10)
                        .saturating_add(ch as u32 - '0' as u32);
                }
                (_, State::CsiParam, ';') => {
                    if self.parameters.is_empty() {
                        self.parameters.push(0);
                    }
                    if self.parameters.len() < 32 {
                        self.parameters.push(0);
                    }
                }
                (_, State::CsiIntermediate, _) | (_, State::CsiParam, ':') => {
                    self.has_intermediate = true;
                }
                _ => {}
            }
        }
    }

    fn dispatch(&mut self, final_char: char) {
        if self.has_intermediate {
            return;
        }
        let Some(prefix) = self.prefix else { return };
        if prefix == '?' && matches!(final_char, 'h' | 'l') {
            let enabled = final_char == 'h';
            for &mode in &self.parameters {
                match mode {
                    9 | 1000 | 1002 | 1003 => {
                        self.mouse_tracking = Some(if enabled { mode as u16 } else { 0 });
                    }
                    1005 | 1006 | 1015 | 1016 => {
                        self.mouse_encoding = Some(if enabled { mode as u16 } else { 0 });
                    }
                    1004 => self.focus = Some(enabled),
                    1007 => self.alternate_scroll = Some(enabled),
                    2004 => self.paste = Some(enabled),
                    47 | 1047 | 1049 => self.alternate = enabled,
                    _ => {}
                }
            }
        } else if final_char == 'u' && matches!(prefix, '=' | '>' | '<') {
            let value = self.parameters.first().copied().unwrap_or(0);
            let keyboard = &mut self.keyboards[usize::from(self.alternate)];
            let flags = (value & 31) as u16;
            match prefix {
                '=' => match self.parameters.get(1).copied().unwrap_or(1) {
                    1 => keyboard.flags = flags,
                    2 => keyboard.flags |= flags,
                    3 => keyboard.flags &= !flags,
                    _ => {}
                },
                '>' => {
                    if keyboard.stack.len() == KEYBOARD_STACK_LIMIT {
                        keyboard.stack.remove(0);
                    }
                    keyboard.stack.push(keyboard.flags);
                    keyboard.flags = flags;
                }
                '<' => {
                    let count = value.max(1) as usize;
                    keyboard.flags = keyboard
                        .stack
                        .len()
                        .checked_sub(count)
                        .map_or(0, |index| keyboard.stack[index]);
                    keyboard
                        .stack
                        .truncate(keyboard.stack.len().saturating_sub(count));
                }
                _ => unreachable!(),
            }
        }
    }

    pub(super) fn append_keyboard_replay(&self, replay: &mut String) {
        for (index, keyboard) in self.keyboards.iter().enumerate() {
            if keyboard.stack.is_empty() && keyboard.flags == 0 {
                continue;
            }
            if index == 1 {
                replay.push_str("\x1b[?1047h");
            }
            let mut settings = keyboard
                .stack
                .iter()
                .chain(std::iter::once(&keyboard.flags));
            write!(replay, "\x1b[={}u", settings.next().unwrap()).unwrap();
            for flags in settings {
                write!(replay, "\x1b[>{flags}u").unwrap();
            }
            if index == 1 {
                replay.push_str("\x1b[?1047l");
            }
        }
    }

    pub(super) fn append_replay(&self, replay: &mut String) {
        for (selection, reset) in [(self.mouse_tracking, 1000), (self.mouse_encoding, 1006)] {
            if let Some(mode) = selection {
                write!(
                    replay,
                    "\x1b[?{}{}",
                    if mode == 0 { reset } else { mode },
                    if mode == 0 { 'l' } else { 'h' }
                )
                .unwrap();
            }
        }
        for (mode, enabled) in [
            (1007, self.alternate_scroll),
            (1004, self.focus),
            (2004, self.paste),
        ] {
            if let Some(enabled) = enabled {
                write!(replay, "\x1b[?{mode}{}", if enabled { 'h' } else { 'l' }).unwrap();
            }
        }
    }
}

#[cfg(test)]
#[path = "terminal_input_modes_test.rs"]
mod terminal_input_modes_tests;
