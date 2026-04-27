use rand_core::Rng;
pub use bevy_prng::WyRand;
pub use rand_core::SeedableRng;

pub trait NextIntExt {
    /// Generates a random integer between min (inclusive) and max (exclusive).
    fn next_int(&mut self, min: i32, max: i32) -> i32;

    /// Returns true with probability 1/odds.
    fn chance(&mut self, odds: u32) -> bool;
}

impl<R: Rng> NextIntExt for R {
    #[inline]
    fn next_int(&mut self, min: i32, max: i32) -> i32 {
        let range = (max - min).max(1) as u32;
        min + (self.next_u32() % range) as i32
    }

    #[inline]
    fn chance(&mut self, odds: u32) -> bool {
        if odds == 0 {
            false
        } else {
            (self.next_u32() % odds) == 0
        }
    }
}
