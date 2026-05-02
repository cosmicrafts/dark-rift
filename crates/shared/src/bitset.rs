use serde::{Deserialize, Serialize};

/// A highly-optimized, flat bitset for tracking boolean state across a large ID space (like map tiles).
/// It stores bits sequentially in a flat `Vec<u64>` to eliminate pointer chasing and cache misses.
/// 
/// For network efficiency via Lightyear/Serde, `serialize` and `deserialize` convert this dense
/// array into a sparse `Vec<u32>` payload, sending only the active indices over the wire.
#[derive(Clone, Debug, PartialEq)]
pub struct DenseBitSet {
    /// Each u64 holds 64 bits. Total length = (max_capacity + 63) / 64.
    pub blocks: Vec<u64>,
}

impl Default for DenseBitSet {
    fn default() -> Self {
        Self { blocks: Vec::new() }
    }
}

impl DenseBitSet {
    /// Creates an empty BitSet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a BitSet pre-sized for the given capacity (e.g., total map tiles).
    pub fn with_capacity(capacity: usize) -> Self {
        let block_count = (capacity + 63) / 64;
        Self {
            blocks: vec![0; block_count],
        }
    }

    /// Clears all bits.
    pub fn clear(&mut self) {
        self.blocks.fill(0);
    }

    /// Sets the bit at `idx` to 1. Returns `true` if it was not already set.
    #[inline]
    pub fn insert(&mut self, idx: u32) -> bool {
        let idx = idx as usize;
        let block = idx / 64;
        let bit = idx % 64;
        
        if block >= self.blocks.len() {
            self.blocks.resize(block + 1, 0);
        }
        
        let old = self.blocks[block];
        let mask = 1 << bit;
        self.blocks[block] = old | mask;
        (old & mask) == 0
    }

    /// Clears the bit at `idx` to 0. Returns `true` if it was previously set.
    #[inline]
    pub fn remove(&mut self, idx: u32) -> bool {
        let idx = idx as usize;
        let block = idx / 64;
        let bit = idx % 64;
        
        if block < self.blocks.len() {
            let old = self.blocks[block];
            let mask = 1 << bit;
            if (old & mask) != 0 {
                self.blocks[block] = old & !mask;
                return true;
            }
        }
        false
    }

    /// Checks if the bit at `idx` is set.
    #[inline]
    pub fn contains(&self, idx: u32) -> bool {
        let idx = idx as usize;
        let block = idx / 64;
        if block < self.blocks.len() {
            let bit = idx % 64;
            (self.blocks[block] & (1 << bit)) != 0
        } else {
            false
        }
    }

    /// Returns an iterator over all set indices. 
    /// Iteration is absolutely deterministic and spatially ordered (from index 0 to max).
    pub fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.blocks.iter().enumerate().flat_map(|(b_idx, &block)| {
            let mut val = block;
            let offset = 0;
            std::iter::from_fn(move || {
                if val == 0 {
                    None
                } else {
                    let trailing = val.trailing_zeros();
                    val &= !(1 << trailing); // clear the lowest set bit
                    let global_idx = (b_idx * 64 + offset + trailing as usize) as u32;
                    Some(global_idx)
                }
            })
        })
    }

    /// Counts total set bits.
    pub fn count_ones(&self) -> usize {
        self.blocks.iter().map(|b| b.count_ones() as usize).sum()
    }
    
    /// Returns true if no bits are set.
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|&b| b == 0)
    }

    /// Extends the set from an iterator of indices.
    pub fn extend(&mut self, iter: impl IntoIterator<Item = u32>) {
        for idx in iter {
            self.insert(idx);
        }
    }
}

// Custom Serialize: send only active indices (Sparse)
impl Serialize for DenseBitSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a Vec<u32> to keep network payload small
        let indices: Vec<u32> = self.ones().collect();
        indices.serialize(serializer)
    }
}

// Custom Deserialize: reconstruct dense bitset from sparse indices
impl<'de> Deserialize<'de> for DenseBitSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let indices = Vec::<u32>::deserialize(deserializer)?;
        let mut bitset = DenseBitSet::new();
        // Presize if possible based on max index to avoid resizing
        if let Some(&max) = indices.iter().max() {
            bitset.blocks.resize((max as usize / 64) + 1, 0);
        }
        for idx in indices {
            bitset.insert(idx);
        }
        Ok(bitset)
    }
}
