//! Implement fixed length uint type for Hw simulation
//!
//! This type support basic arithmetics operation and infinite bitwidth.
//! It is backed by a vector of u64 and intermediate computation are done on u128
//!
//! It rely on const generic and need the `feature(generic_const_exprs)`.

/// Maximum of two usize computed at compiled time
/// => Use on const generic during sub-type conversion
pub const fn const_max(lw: usize, rw: usize) -> usize {
    if lw > rw {
        lw
    } else {
        rw
    }
}

/// Minimum of two usize computed at compiled time
/// => Use on const generic during sub-type conversion
pub const fn const_min(lw: usize, rw: usize) -> usize {
    if lw < rw {
        lw
    } else {
        rw
    }
}

/// Split u128 in slice of two u64
fn split(val: u128) -> (u64, u64) {
    let lsb = (val & !((!1_u128) << 64)) as u64;
    let msb = (val >> 64) as u64;
    (msb, lsb)
}

/// Add two sliced values
/// Enable addition of infinite bit-length values
fn sliced_add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let (long_op, short_op) = if a.len() >= b.len() { (a, b) } else { (b, a) };

    let mut res = vec![0_u64; long_op.len() + 1];

    // Compute long/short add
    for i in 0..short_op.len() {
        let tmp = res[i] as u128 + short_op[i] as u128 + long_op[i] as u128;

        (res[i + 1], res[i]) = split(tmp);
    }

    // expand with longest
    for i in short_op.len()..long_op.len() {
        let tmp = res[i] as u128 + long_op[i] as u128;

        (res[i + 1], res[i]) = split(tmp);
    }

    // Shrink to initial size and check for overflow
    let ovflw = res.pop().unwrap_or(0);
    debug_assert!(ovflw == 0, "Sliced operation overflowed");

    res
}

/// Two complement a sliced value
/// This operation is used to derived sliced_sub from sliced_add
fn sliced_two_complement(val: &[u64]) -> Vec<u64> {
    let mut res = Vec::with_capacity(val.len());

    // Complement input
    for s in val.iter() {
        res.push(!s);
    }

    // Add1 => Two's complement
    sliced_add(res.as_slice(), &[1_u64])
}

fn sliced_sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    let b_tc = sliced_two_complement(b);
    sliced_add(a, b_tc.as_slice())
}

/// Mul two sliced values
/// Enable multiplication of infinite bit-length values
fn sliced_mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    let max_op_len = std::cmp::max(a.len(), b.len());
    let res_len = a.len() + b.len();

    let mut res = vec![0_u64; res_len];
    for (i, sa) in a.iter().enumerate() {
        for (j, sb) in b.iter().enumerate() {
            let tmp = *sa as u128 * *sb as u128;
            let (msw, lsw) = split(tmp);
            res[(i + j)] += lsw;
            res[(i + j) + 1] = msw;
        }
    }

    // Shrink to initial size and check for overflow
    for i in std::cmp::max(a.len(), b.len())..res_len {
        debug_assert!(res[i] == 0, "Sliced operation overflowed");
    }
    res.resize(max_op_len, 0);
    res
}

/// Div two sliced values
/// Enable division of infinite bit-length values
fn sliced_div(_a: &[u64], _b: &[u64]) -> Vec<u64> {
    todo!();
}

/// Shift left (<<) without extending the slice size
fn sliced_shl(val: &[u64], lshift: usize) -> Vec<u64> {
    let shift_word = lshift.div_floor(64);
    let shift_rem = lshift % 64;

    let mut res = vec![0_u64; val.len() + shift_word];
    let mut shift_in = 0_u64;
    for i in 0..val.len() {
        res[i + shift_word] = val[i] << shift_rem;
        res[i + shift_word] += shift_in;
        shift_in = ((val[i] as u128) >> (64 - shift_rem)) as u64;
    }
    res.resize(val.len(), 0);
    res
}

/// Shift right (>>) without shrinking the slice size
fn sliced_shr(val: &[u64], rshift: usize) -> Vec<u64> {
    let shift_word = rshift.div_floor(64);
    let shift_rem = rshift % 64;

    let mut res = vec![0_u64; val.len()];
    let mut shift_in = 0_u64;
    for i in (0..val.len()).rev() {
        let val_isw = if (i + shift_word) < val.len() {
            val[i + shift_word]
        } else {
            0
        };
        res[i] = val_isw >> shift_rem;
        res[i] += ((shift_in as u128) << (64 - shift_rem)) as u64;
        shift_in = val_isw & !((!1_u64) << shift_rem);
    }
    res
}

/// Fixed sized unsigned integer
///
/// This type enable to represent arbitrary long unsigned integer with a fixed number of bit.
/// Underneath it's use a vector of u64 to store it's value.
/// Common arithmetic operations are define on it and basic type conversion
pub struct UInt<const W: usize>(Vec<u64>);

impl<const WL: usize, const WR: usize> std::ops::Add<UInt<WR>> for UInt<WL>
where
    UInt<{ const_max(WL, WR) }>: Sized,
{
    type Output = UInt<{ const_max(WL, WR) }>;

    fn add(self, rhs: UInt<WR>) -> Self::Output {
        UInt::from_vec(sliced_add(self.0.as_slice(), rhs.0.as_slice()))
    }
}

impl<const W: usize> std::ops::Sub for UInt<W> {
    type Output = UInt<W>;

    fn sub(self, rhs: Self) -> Self {
        UInt::from_vec(sliced_sub(self.0.as_slice(), rhs.0.as_slice()))
    }
}
impl<const W: usize> std::ops::Mul for UInt<W> {
    type Output = UInt<W>;

    fn mul(self, rhs: Self) -> Self {
        UInt::from_vec(sliced_mul(self.0.as_slice(), rhs.0.as_slice()))
    }
}
impl<const W: usize> std::ops::Div for UInt<W> {
    type Output = UInt<W>;

    fn div(self, rhs: Self) -> Self {
        UInt::from_vec(sliced_div(self.0.as_slice(), rhs.0.as_slice()))
    }
}

impl<const W: usize> std::ops::Shl<usize> for UInt<W> {
    type Output = Self;

    fn shl(self, rhs: usize) -> Self {
        UInt::from_vec(sliced_shl(self.0.as_slice(), rhs))
    }
}

impl<const W: usize> std::ops::Shr<usize> for UInt<W> {
    type Output = Self;

    fn shr(self, rhs: usize) -> Self {
        UInt::from_vec(sliced_shr(self.0.as_slice(), rhs))
    }
}

impl<const W: usize> UInt<W> {
    pub fn from_vec(vec: Vec<u64>) -> Self {
        let mut raw = Self(vec);
        let ovflw = raw.normalize();
        debug_assert!(ovflw, "Given vector overflowed available bits");
        raw
    }

    /// Change the number of bit of the represented UInt
    /// Use custom function since implementing From trait between two
    /// UInt with distinct const generic wasn't possible.
    pub fn resize<const T: usize>(&self) -> UInt<T> {
        UInt::<T>::from_vec(self.0.clone())
    }

    /// Enforce that only Width bit is used and return an overflow status
    ///
    /// This function is used after a sliced operation to enforce that no
    /// overflow occurred in the most-significant word that could be incomplete
    /// (e.g. W parameters not a multiple of u64 bytes)
    fn normalize(&mut self) -> bool {
        // Length of underlying vector
        let trgt_len = W.div_ceil(64);

        // Number of bit encoded in the most significant word
        let msw_rem = W % 64;

        if self.0.len() < trgt_len {
            false
        } else {
            self.0.resize(trgt_len, 0);
            let msw = self.0[trgt_len - 1];
            let bitmask = !((!1_u64) << msw_rem);
            let overflow = msw >> std::mem::size_of::<u64>();
            self.0[trgt_len - 1] = msw & bitmask;
            overflow > 0
        }
    }

    /// Utility function to view Uint as u128 if possible
    /// Mainly use in testing
    fn _view_packed(&self) -> Option<u128> {
        if self.0.len() <= 2 {
            let mut packed_val = 0_u128;
            for (i, s) in self.0.iter().enumerate() {
                packed_val += (*s as u128) << (64 * i);
            }
            Some(packed_val)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod units_tests {
    use super::*;
    use rand::{
        rngs::StdRng,
        {Rng, SeedableRng},
    };
    const SLICED_ARITHM_ITER: usize = 100;

    /// Utility function to generate random sliced number
    fn gen_rand_vec(chunk_size: usize) -> Vec<u64> {
        let mut rng: StdRng = SeedableRng::from_os_rng();

        let mut rand = Vec::new();
        for _ in 0..chunk_size {
            let fr: u64 = rng.random();
            rand.push(fr >> 4);
        }
        rand
    }

    /// Utility function to packed Vec<u64> in u128
    /// Ease implementation of golden model arithmetics when vec.len() <2
    fn packed(sliced_val: &Vec<u64>) -> u128 {
        assert!(
            sliced_val.len() <= 2,
            "Couldn't packed in u128 vector is too large {}",
            sliced_val.len()
        );

        let mut packed_val = 0_u128;
        for (i, s) in sliced_val.iter().enumerate() {
            packed_val += (*s as u128) << (64 * i);
        }
        packed_val
    }

    #[test]
    fn test_sliced_add() {
        for _ in 0..SLICED_ARITHM_ITER {
            let a = gen_rand_vec(2);
            let b = gen_rand_vec(2);
            assert_eq!(
                packed(&a) + packed(&b),
                packed(&sliced_add(a.as_slice(), b.as_slice()))
            );
        }
    }

    #[test]
    fn test_sliced_sub() {
        for _ in 0..SLICED_ARITHM_ITER {
            let a = gen_rand_vec(2);
            let b = gen_rand_vec(2);
            let (lop, rop) = if packed(&a) >= packed(&b) {
                (a, b)
            } else {
                (b, a)
            };
            assert_eq!(
                packed(&lop) - packed(&rop),
                packed(&sliced_sub(lop.as_slice(), rop.as_slice()))
            );
        }
    }

    #[test]
    fn test_sliced_mul() {
        for _ in 0..SLICED_ARITHM_ITER {
            let a = gen_rand_vec(2);
            let b = gen_rand_vec(2);
            assert_eq!(
                packed(&a) * packed(&b),
                packed(&sliced_mul(a.as_slice(), b.as_slice()))
            );
        }
    }

    // #[test]
    // fn test_sliced_div() {
    // }

    #[test]
    fn test_sliced_shl() {
        let mut rng: StdRng = SeedableRng::from_os_rng();
        for _ in 0..SLICED_ARITHM_ITER {
            let val = gen_rand_vec(2);
            let lshift = rng.random_range(0..128);
            assert_eq!(
                packed(&val) << lshift,
                packed(&sliced_shl(val.as_slice(), lshift))
            );
        }
    }

    #[test]
    fn test_sliced_shr() {
        let mut rng: StdRng = SeedableRng::from_os_rng();
        for _ in 0..SLICED_ARITHM_ITER {
            let val = gen_rand_vec(2);
            let rshift = rng.random_range(0..128);
            assert_eq!(
                packed(&val) >> rshift,
                packed(&sliced_shr(val.as_slice(), rshift))
            );
        }
    }

    macro_rules! rand_uint {
        ($width: expr) => {{
            let mut rng: StdRng = SeedableRng::from_os_rng();
            // Length of underlying vector
            let trgt_len = ($width as usize).div_ceil(64);
            // Number of bit encoded in the most significant word
            let msw_rem = $width % 64;

            let mut raw_uint = Vec::new();
            // gen body
            for _ in 0..(trgt_len - 1) {
                raw_uint.push(rng.random());
            }
            // gen masked head
            let masked_msb = rng.random::<u64>() & !(!1_u64 << msw_rem);
            raw_uint.push(masked_msb);

            // convert to uint
            let uint: UInt<$width> = UInt::from_vec(raw_uint);
            uint
        }};
    }

    const UINT_A_WIDTH: usize = 120;
    const UINT_B_WIDTH: usize = 108;

    #[test]
    fn test_uint_add() {
        for _ in 0..SLICED_ARITHM_ITER {
            let a = rand_uint!(UINT_A_WIDTH);
            let b = rand_uint!(UINT_B_WIDTH);
            let golden_arith = a._view_packed().unwrap() + b._view_packed().unwrap();
            let uint_arith = a + b.resize::<UINT_A_WIDTH>();
            assert_eq!(golden_arith, uint_arith._view_packed().unwrap());
        }
    }
    #[test]
    fn test_uint_sub() {
        for _ in 0..SLICED_ARITHM_ITER {
            let a = rand_uint!(UINT_A_WIDTH);
            let b = rand_uint!(UINT_B_WIDTH);
            let golden_arith = a._view_packed().unwrap() - b._view_packed().unwrap();
            let uint_arith = a - b.resize::<UINT_A_WIDTH>();
            assert_eq!(golden_arith, uint_arith._view_packed().unwrap());
        }
    }

    #[test]
    fn test_uint_mul() {
        for _ in 0..SLICED_ARITHM_ITER {
            let a = rand_uint!(UINT_A_WIDTH);
            let b = rand_uint!(UINT_B_WIDTH);
            let golden_arith = a._view_packed().unwrap() * b._view_packed().unwrap()
                & !(!1 << std::cmp::max(UINT_A_WIDTH, UINT_B_WIDTH));
            let uint_arith = a * b.resize::<UINT_A_WIDTH>();
            assert_eq!(golden_arith, uint_arith._view_packed().unwrap());
        }
    }

    // #[test]
    // fn test_uint_div() {
    // }

    #[test]
    fn test_uint_shl() {
        let mut rng: StdRng = SeedableRng::from_os_rng();
        for _ in 0..SLICED_ARITHM_ITER {
            let val_a = rand_uint!(UINT_A_WIDTH);
            let lshift = rng.random_range(0..128);
            let golden_arith = (val_a._view_packed().unwrap() << lshift) & !(!1 << (UINT_A_WIDTH));
            let uint_arith = val_a << lshift;
            assert_eq!(golden_arith, uint_arith._view_packed().unwrap());
        }
    }

    #[test]
    fn test_uint_shr() {
        let mut rng: StdRng = SeedableRng::from_os_rng();
        for _ in 0..SLICED_ARITHM_ITER {
            let val_a = rand_uint!(UINT_A_WIDTH);
            let rshift = rng.random_range(0..128);
            let golden_arith = val_a._view_packed().unwrap() >> rshift;
            let uint_arith = val_a >> rshift;
            assert_eq!(golden_arith, uint_arith._view_packed().unwrap());
        }
    }
}
