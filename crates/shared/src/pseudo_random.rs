#[derive(Clone, Copy, Debug)]
pub struct PseudoRandom {
    state: u64,
}

impl PseudoRandom {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_int(&mut self, min: i32, max: i32) -> i32 {
        let oldstate = self.state;
        self.state = oldstate.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let xorshifted = (((oldstate >> 18) ^ oldstate) >> 27) as u32;
        let rot = (oldstate >> 59) as u32;
        let result = (xorshifted >> rot) | (xorshifted << ((!rot).wrapping_add(1) & 31));
        let range = (max - min).max(1) as u32;
        min + (result % range) as i32
    }
}
