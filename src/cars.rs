//! Car catalogue (`tool.e` / `tool.d`). The table itself is generated from the decompiled
//! source in `cars_table.rs`.

pub use crate::cars_table::{CAR_COUNT, CAR_TABLE};

/// Ability indices into `CarInfo::ability` / `SaveGame::upgrade[car]`.
pub const ABILITY_ENGINE: usize = 0;
pub const ABILITY_BRAKE: usize = 1;
pub const ABILITY_TIRE: usize = 2;
pub const ABILITY_FUEL: usize = 3;
pub const ABILITY_COMFORT: usize = 4;
pub const ABILITY_DURABILITY: usize = 5;

/// Cars `0..PURCHASABLE_CARS` can be bought; `PURCHASABLE_CARS..PLAYER_CARS` are rental-only.
pub const PURCHASABLE_CARS: usize = 16;
/// Player-selectable cars (28 = bus is never selectable, 29 = truck is forced in Truck mission).
pub const PLAYER_CARS: usize = 28;
pub const TRUCK_CAR: usize = 29;

#[derive(Clone, Copy, Debug)]
pub struct CarInfo {
    pub no: usize,
    pub name: &'static str,
    /// engine, brake, tire, fuel, comfort, durability
    pub ability: [i32; 6],
    pub half_length: f32,
    pub half_width: f32,
    pub eye: f32,
    pub eye_height: f32,
    pub wheel_factor: f32,
    pub pitch: f32,
    pub repair_cost: i32,
    /// `< 0`: price in coins. `> 0`: price in gold. `0`: free.
    pub price: i32,
    pub wheel_height: f32,
    pub wheel_front: f32,
    pub wheel_back: f32,
    pub wheel_width_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Price {
    Free,
    Coins(i32),
    Gold(i32),
}

impl CarInfo {
    pub fn price_kind(&self) -> Price {
        match self.price {
            0 => Price::Free,
            p if p < 0 => Price::Coins(-p),
            p => Price::Gold(p),
        }
    }
}

pub fn car(i: usize) -> &'static CarInfo {
    &CAR_TABLE[i]
}
