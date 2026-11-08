//! Works out what to tell Discord ("Rich Presence") from the current game state.

use crate::game::Game;
use crate::rpc::Presence;
use drdriving_core::cars::car;
use drdriving_core::economy::Outcome;
use drdriving_core::mission::Mission;
use drdriving_core::session::Phase;

/// "VIP ESCORT" -> "VIP Escort", "BROKEN BRAKE" -> "Broken Brake".
fn nice(m: Mission) -> String {
    m.name()
        .split(' ')
        .map(|w| if w == "VIP" { w.to_string() } else { w[..1].to_string() + &w[1..].to_lowercase() })
        .collect::<Vec<_>>()
        .join(" ")
}

fn thousands(n: i32) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

impl Game {
    /// Refreshes the Discord presence about once a second (the sender throttles and dedups).
    pub fn update_presence(&mut self) {
        if self.clock - self.presence_at < 1.0 {
            return;
        }
        self.presence_at = self.clock;
        let p = self.build_presence();
        self.rpc.update(p);
    }

    fn build_presence(&self) -> Presence {
        let s = &self.save;
        let level = format!("Level {}", s.driver_level);
        let mut p = Presence {
            large_text: format!("Dr. Driving · {level} · {} coins", thousands(s.coin)),
            small_image: String::new(),
            ..Default::default()
        };
        let car_name = |id: usize| car(id.min(drdriving_core::cars::PLAYER_CARS.saturating_sub(1))).name;
        let rated = format!("Rating {}", s.rating_value());

        if let (Some(sess), 2..=7) = (&self.session, self.mode) {
            let w = &sess.world;
            let mission = nice(w.mission);
            p.scene = self.race_id * 2 + 1;
            let progress = format!("Block {}/{}", w.blocks_passed.clamp(0, w.blocks_for_mission), w.blocks_for_mission);
            let fuel = if w.gas_initial > 0.0 { format!("Fuel {}%", ((w.gas_left / w.gas_initial) * 100.0).clamp(0.0, 100.0).round() as i32) } else { String::new() };
            let car_s = format!("Car {}", car_name(w.car_mode));
            if let Some(on) = sess.online {
                let kind = if on.friendly { "Friendly match" } else { "Ranked match" };
                p.details = match sess.phase {
                    Phase::Success => format!("{kind}: won vs {}", w.opponent.name),
                    Phase::Ending(Outcome::OppWin) => format!("{kind}: lost vs {}", w.opponent.name),
                    Phase::Ending(_) => format!("{kind}: wrecked vs {}", w.opponent.name),
                    _ => format!("{kind} vs {} ({})", w.opponent.name, on.opp_rating),
                };
                p.state = format!("{mission} · {progress} · {car_s}");
                p.party = Some((2, 2));
                p.small_text = rated;
                return p;
            }
            p.details = match sess.phase {
                Phase::Starting => format!("{mission} mission: at the start line"),
                Phase::Playing => match (w.drifting, w.mission) {
                    (true, _) => format!("{mission} mission: drifting!"),
                    _ => format!("{mission} mission: driving"),
                },
                Phase::Paused => format!("{mission} mission: paused"),
                Phase::Success => format!("{mission} mission: completed"),
                Phase::Ending(Outcome::NoFuel) => format!("{mission} mission: out of fuel"),
                Phase::Ending(_) => format!("{mission} mission: crashed"),
            };
            p.state = match sess.phase {
                Phase::Playing | Phase::Paused => [car_s, progress, fuel].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
                _ => format!("{car_s} · {level}"),
            };
            return p;
        }

        // not racing: scene 0 shares one timer across all menu screens
        p.scene = 0;
        if let Some(search) = &self.lobby.search {
            p.details = if search.quick { "Searching for an online match".into() } else { format!("Waiting for a friend (channel {})", search.channel) };
            p.state = format!("{rated} · searching {}s", search.time as i32);
            return p;
        }
        let owned = (0..drdriving_core::cars::PLAYER_CARS).filter(|&c| s.has_car.get(c).copied().unwrap_or(false)).count();
        let (details, state) = match self.mode {
            0 => ("Starting up".to_string(), "Loading".to_string()),
            1 => (format!("Choosing a mission: {}", nice(self.world.mission)), format!("Car {} · {level}", car_name(self.world.car_mode))),
            8 => ("In the main menu".to_string(), format!("{level} · {} coins · {} missions cleared", thousands(s.coin), s.missions_clear_count)),
            9 => ("Browsing the garage".to_string(), format!("{owned} cars owned · {} coins", thousands(s.coin))),
            10 => ("Shopping in the store".to_string(), format!("{} gold · {} coins", thousands(s.gold), thousands(s.coin))),
            11 => ("Tweaking the options".to_string(), level.clone()),
            14 | 15 => ("Setting up an online match".to_string(), rated.clone()),
            13 => ("Leaving the garage".to_string(), level.clone()),
            _ => ("In the menus".to_string(), level.clone()),
        };
        p.details = details;
        p.state = state;
        p
    }
}
