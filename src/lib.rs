//! Engine-independent core logic of Dr. Driving 1.73, ported from the decompiled
//! `com.ansangha.drdriving` classes. See `../README.md` for the Java -> Rust map and
//! the list of things that were deliberately **not** ported (rendering, ads, billing,
//! Firebase, AI traffic).
//!
//! All numeric behaviour is kept `f32`/`i32` to match the Java original.

pub mod autopilot;
pub mod cars;
pub mod economy;
pub mod math;
pub mod mission;
pub mod net;
pub mod netplay;
pub mod rng;
pub mod session;
pub mod traffic;
pub mod rtdb;
pub mod save;
pub mod world;

mod cars_table;
