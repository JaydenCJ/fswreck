//! Deterministic primitives: a SplitMix64 PRNG and FNV-1a 64-bit hashing.
//!
//! fswreck's core promise is that the same seed always produces a
//! byte-identical tree, so we implement both primitives ourselves instead of
//! pulling in `rand` (whose stream stability is not guaranteed across major
//! versions) or a hashing crate.

/// SplitMix64: tiny, fast, and its output stream is a published constant —
/// it will never change underneath a recorded manifest.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Fill `buf` with deterministic bytes (little-endian u64 chunks).
    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

/// Streaming FNV-1a 64-bit hash. Used to fingerprint file contents in the
/// manifest and to derive stable per-path sub-seeds.
#[derive(Debug, Clone)]
pub struct Fnv64(u64);

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl Fnv64 {
    pub fn new() -> Self {
        Self(FNV_OFFSET)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv64 {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot convenience for hashing a byte slice.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = Fnv64::new();
    h.write(bytes);
    h.finish()
}

/// Adapter so content generators can stream straight into the hash.
impl std::io::Write for Fnv64 {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Fnv64::write(self, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_deterministic_and_matches_the_reference_vector() {
        // First outputs of splitmix64(0) from Vigna's reference implementation.
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(r.next_u64(), 0x06C4_5D18_8009_454F);
        // Same seed, same stream; different seed, different stream.
        let mut r1 = SplitMix64::new(42);
        let mut r2 = SplitMix64::new(42);
        for _ in 0..64 {
            assert_eq!(r1.next_u64(), r2.next_u64());
        }
        assert_ne!(SplitMix64::new(1).next_u64(), SplitMix64::new(2).next_u64());
    }

    #[test]
    fn fill_covers_non_multiple_of_eight_lengths() {
        let mut r1 = SplitMix64::new(7);
        let mut r2 = SplitMix64::new(7);
        let mut a = [0u8; 13];
        let mut b = [0u8; 13];
        r1.fill(&mut a);
        r2.fill(&mut b);
        assert_eq!(a, b);
        assert_ne!(a, [0u8; 13], "fill left the buffer untouched");
    }

    #[test]
    fn fnv1a64_matches_published_vectors_and_streams_identically() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        let mut h = Fnv64::new();
        h.write(b"foo");
        h.write(b"bar");
        assert_eq!(h.finish(), fnv1a64(b"foobar"));
    }
}
