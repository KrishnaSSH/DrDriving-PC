//! Keyboard access for the game. Real key state comes from macroquad; a script can additionally
//! "press" keys at given times, which lets the whole menu/race flow be driven and tested
//! without a human (`--script`), through exactly the same code path as the real keyboard.

use macroquad::prelude::*;

#[derive(Default)]
pub struct Keys {
    script: Vec<(f32, KeyCode)>,
    next: usize,
    now: f32,
    fired: Vec<KeyCode>,
    injected: Vec<KeyCode>,
}

pub fn key_from_name(n: &str) -> Option<KeyCode> {
    Some(match n.to_ascii_lowercase().as_str() {
        "space" => KeyCode::Space,
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "f1" => KeyCode::F1,
        "f2" => KeyCode::F2,
        "f3" => KeyCode::F3,
        "f4" => KeyCode::F4,
        "1" => KeyCode::Key1,
        "2" => KeyCode::Key2,
        "3" => KeyCode::Key3,
        "4" => KeyCode::Key4,
        s if s.len() == 1 => match s.as_bytes()[0] {
            b'a' => KeyCode::A,
            b'b' => KeyCode::B,
            b'c' => KeyCode::C,
            b'd' => KeyCode::D,
            b'e' => KeyCode::E,
            b'h' => KeyCode::H,
            b'n' => KeyCode::N,
            b'p' => KeyCode::P,
            b'q' => KeyCode::Q,
            b'r' => KeyCode::R,
            b's' => KeyCode::S,
            b'w' => KeyCode::W,
            b'x' => KeyCode::X,
            b'y' => KeyCode::Y,
            b'z' => KeyCode::Z,
            _ => return None,
        },
        _ => return None,
    })
}

impl Keys {
    /// `spec` = comma-separated `seconds:Key` presses, e.g. `1.0:Space,2.5:Enter`.
    pub fn with_script(spec: &str) -> Keys {
        let mut script: Vec<(f32, KeyCode)> = spec
            .split(',')
            .filter_map(|item| {
                let (t, k) = item.split_once(':')?;
                Some((t.trim().parse().ok()?, key_from_name(k.trim())?))
            })
            .collect();
        script.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        Keys { script, ..Default::default() }
    }

    /// Advance script time; keys scheduled up to now count as pressed for this frame only.
    pub fn tick(&mut self, dt: f32) {
        self.now += dt;
        self.fired.clear();
        self.fired.append(&mut self.injected);
        while self.next < self.script.len() && self.script[self.next].0 <= self.now {
            self.fired.push(self.script[self.next].1);
            self.next += 1;
        }
    }

    /// Press `k` on the next frame (used by the autopilot demo to click through menus).
    pub fn inject(&mut self, k: KeyCode) {
        self.injected.push(k);
    }

    pub fn pressed(&self, k: KeyCode) -> bool {
        is_key_pressed(k) || self.fired.contains(&k)
    }

    pub fn down(&self, k: KeyCode) -> bool {
        is_key_down(k) || self.fired.contains(&k)
    }
}
