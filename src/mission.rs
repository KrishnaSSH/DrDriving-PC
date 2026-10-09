//! Mission types and route generation (`drdriving.l.MISSION_*`, `tool.k`).

use crate::rng::JavaRandom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Mission {
    Speed = 0,
    Highway = 1,
    Drift = 2,
    Fuel = 3,
    Vip = 4,
    Parking = 5,
    BrokenBrake = 6,
    Truck = 7,
    Lane = 8,
}

impl Mission {
    pub const ALL: [Mission; 9] = [
        Mission::Speed,
        Mission::Highway,
        Mission::Drift,
        Mission::Fuel,
        Mission::Vip,
        Mission::Parking,
        Mission::BrokenBrake,
        Mission::Truck,
        Mission::Lane,
    ];
    pub fn from_index(i: i32) -> Option<Self> {
        Self::ALL.get(usize::try_from(i).ok()?).copied()
    }
    pub fn index(self) -> usize {
        self as usize
    }
    /// Number of blocks before the finish zone opens (`g_iNoBlockForMission`).
    pub fn blocks_for_mission(self) -> i32 {
        match self {
            Mission::Truck => 5,
            Mission::Speed => 6,
            Mission::Highway => 9,
            Mission::Lane => 8,
            Mission::Parking => 2,
            Mission::BrokenBrake => 6,
            _ => 5,
        }
    }
    /// Starting fuel (`g_fGasLeft`).
    pub fn start_fuel(self) -> f32 {
        if self == Mission::Fuel {
            100.0
        } else {
            500.0
        }
    }
    pub fn name(self) -> &'static str {
        ["SPEED", "HIGHWAY", "DRIFT", "FUEL", "VIP ESCORT", "PARKING", "BROKEN BRAKE", "TRUCK", "LANE"][self.index()]
    }
}

/// Relative turn codes used in `directions`.
pub const REL_STRAIGHT: i8 = 0;
pub const REL_RIGHT: i8 = 1;
pub const REL_UTURN: i8 = 2;
pub const REL_LEFT: i8 = 3;

/// `tool.k` — one generated course: 20 relative directions + 40 AI-traffic picks.
/// This is also exactly the payload of the multiplayer `I` message.
#[derive(Clone, Debug)]
pub struct MissionInfo {
    pub mission: Mission,
    pub directions: [i8; 20],
    pub aicar: [u8; 40],
    last: i32,
    last_last: i32,
    last_last_last: i32,
}

impl MissionInfo {
    pub fn new(mission: Mission) -> Self {
        Self { mission, directions: [0; 20], aicar: [0; 40], last: -1, last_last: -1, last_last_last: -1 }
    }

    /// `generate(i3)`: pick a *different* random mission (not Fuel/VIP, not any of the last
    /// three) and build its route. Panics-free but loops until a legal mission is found;
    /// with 9 missions and 5 exclusions there is always one.
    pub fn generate(&mut self, current: Mission, rng: &mut JavaRandom) {
        let cur = current.index() as i32;
        let mut n = cur;
        loop {
            n = ((n + 1) + rng.next_int_bound(8)) % 9;
            if n != cur
                && n != self.last
                && n != self.last_last
                && n != self.last_last_last
                && n != 3
                && n != 4
            {
                self.last_last_last = self.last_last;
                self.last_last = self.last;
                self.last = self.mission.index() as i32;
                self.mission = Mission::from_index(n).unwrap();
                self.generate_directions(rng);
                return;
            }
        }
    }

    /// `generateMissionDirections()` — ported statement-for-statement.
    pub fn generate_directions(&mut self, rng: &mut JavaRandom) {
        let m = self.mission.index() as i32;
        let mut i4 = if m == 0 { 6 } else { 5 };
        if m == 1 {
            i4 = 9;
        }
        if m == 5 {
            i4 = 2;
        }
        let i5 = if m != 6 { i4 } else { 6 };

        let mut idx = 0usize;
        let mut z2 = false; // a U-turn has been emitted
        let mut count = 0; // i7
        let mut last = 0i32; // i8
        let mut last_last = 2i32; // i9
        while idx < 20 {
            let n = loop {
                let n = rng.next_int_bound(4);
                if m == 1 || m == 8 {
                    break 0;
                }
                if m == 5 {
                    if n != 0
                        && n != 2
                        && n != last
                        && (n != 2 || (!z2 && last_last != 2 && m != 5 && m != 2))
                    {
                        break n;
                    }
                } else if m != 4 || count < i5 || (n != 0 && n != 2) {
                    if n != last {
                        break n;
                    }
                }
            };
            self.directions[idx] = n as i8;
            if n == 2 {
                z2 = true;
            }
            count += 1;
            idx += 1;
            last_last = last;
            last = n;
        }

        let mut n = rng.next_int_bound(3);
        for i in 0..40 {
            self.aicar[i] = n as u8;
            n = ((n + 1) + rng.next_int_bound(2)) % 3;
        }
    }
}
