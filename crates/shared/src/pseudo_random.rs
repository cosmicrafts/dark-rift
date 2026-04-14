#[derive(Clone, Copy, Debug)]
pub struct PseudoRandom {
    state: u64,
}

impl PseudoRandom {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_int(&mut self, min: i32, max: i32) -> i32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let result = (self.state >> 32) as u32;
        let range = (max - min).max(1) as u32;
        min + (result % range) as i32
    }
}
