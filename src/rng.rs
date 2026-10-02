//! Bit-exact port of `java.util.Random` (48-bit LCG), so seeded routes match the Java game.

const MULT: u64 = 0x5DEECE66D;
const ADD: u64 = 0xB;
const MASK: u64 = (1 << 48) - 1;

#[derive(Clone, Debug)]
pub struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        Self { seed: ((seed as u64) ^ MULT) & MASK }
    }

    /// Seeds from the system clock (native only; on wasm `SystemTime` is unavailable, so callers
    /// should pass their own seed via [`JavaRandom::new`]).
    pub fn from_entropy() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        #[cfg(target_arch = "wasm32")]
        let n = 0x1234_5678_9abc_def0_i64;
        Self::new(n ^ 0x2545F4914F6CDD1D)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(MULT).wrapping_add(ADD)) & MASK;
        (self.seed >> (48 - bits)) as i64 as i32
    }

    /// `nextFloat()`: uniform in [0, 1) with 24 bits of precision.
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1u32 << 24) as f32
    }

    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `nextInt(bound)`; panics if `bound <= 0` like Java throws.
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if bound & (-bound) == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
}
