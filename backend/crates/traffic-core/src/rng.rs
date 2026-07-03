//! Deterministic, stateless pseudo-randomness for the sim path.
//!
//! Per project convention (PR #90 precedent), randomness in the simulation
//! must be a pure function of `(seed, tick, id)` — never a stateful RNG whose
//! draw order depends on iteration order or thread scheduling. This module
//! exposes exactly one primitive: [`u01`], a splitmix64 finalizer that hashes
//! the three inputs into a uniform `f32` in `[0, 1)`.
//!
//! The high bits of a raw hash cluster badly (see #90's FNV-1a lesson), so we
//! run the full splitmix64 finalizer and take the top 24 bits to build the
//! mantissa of a float in `[0, 1)` — uniform and free of low-bit artefacts.

/// splitmix64 finalizer (Steele, Lea & Flood 2014). Bijective avalanche of a
/// 64-bit input into a well-mixed 64-bit output.
#[inline]
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform `f32` in `[0, 1)`, deterministic in `(seed, tick, id)`.
///
/// The three inputs are folded with distinct odd multipliers before the
/// finalizer so that varying any one of them independently perturbs the whole
/// 64-bit word (no field aliases another). The top 24 bits form the mantissa
/// of the result, giving `2^-24` resolution across `[0, 1)`.
#[inline]
pub fn u01(seed: u64, tick: u64, id: u64) -> f32 {
    let mixed = seed
        .wrapping_mul(0xD1B5_4A32_D192_ED03)
        .wrapping_add(tick.wrapping_mul(0xA0761D6478BD642F))
        .wrapping_add(id.wrapping_mul(0xE703_7ED1_A0B4_28DB));
    let h = splitmix64(mixed);
    // Top 24 bits -> mantissa; divide by 2^24 for a uniform [0, 1) float.
    ((h >> 40) as f32) * (1.0 / 16_777_216.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u01_in_unit_interval() {
        for id in 0..10_000u64 {
            let x = u01(42, 7, id);
            assert!((0.0..1.0).contains(&x), "u01 out of range: {x}");
        }
    }

    #[test]
    fn u01_deterministic() {
        assert_eq!(u01(1, 2, 3), u01(1, 2, 3));
    }

    #[test]
    fn u01_varies_each_field() {
        assert_ne!(u01(1, 2, 3), u01(2, 2, 3));
        assert_ne!(u01(1, 2, 3), u01(1, 3, 3));
        assert_ne!(u01(1, 2, 3), u01(1, 2, 4));
    }

    #[test]
    fn u01_roughly_uniform_mean() {
        let n = 100_000u64;
        let mut sum = 0.0f64;
        for id in 0..n {
            sum += u01(9, 0, id) as f64;
        }
        let mean = sum / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean} not near 0.5");
    }
}
