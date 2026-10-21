//! CPython `random` module port — MT19937 with init_by_array seeding, so
//! `Random(seed).shuffle(ids)` produces the identical permutation the Python
//! grid route produces.

pub struct Mt19937 {
    mt: [u32; 624],
    idx: usize,
}

const N: usize = 624;

impl Mt19937 {
    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..N {
            self.mt[i] = 1812433253u32
                .wrapping_mul(self.mt[i - 1] ^ (self.mt[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        self.idx = N;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19650218);
        let mut i = 1usize;
        let mut j = 0usize;
        let mut k = N.max(key.len());
        while k > 0 {
            self.mt[i] = (self.mt[i]
                ^ (self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_mul(1664525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = N - 1;
        while k > 0 {
            self.mt[i] = (self.mt[i]
                ^ (self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_mul(1566083941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            k -= 1;
        }
        self.mt[0] = 0x8000_0000;
        self.idx = N;
    }

    /// CPython random.seed(int): key = little-endian 32-bit words of abs(n).
    pub fn seed_int(&mut self, n: i64) {
        let a = n.unsigned_abs();
        let mut key: Vec<u32> = Vec::new();
        let mut v = a;
        loop {
            key.push((v & 0xFFFF_FFFF) as u32);
            v >>= 32;
            if v == 0 {
                break;
            }
        }
        if key.is_empty() {
            key.push(0);
        }
        self.init_by_array(&key);
    }

    pub fn new_seeded(n: i64) -> Self {
        let mut m = Mt19937 { mt: [0; N], idx: N };
        m.seed_int(n);
        m
    }

    pub fn genrand_u32(&mut self) -> u32 {
        if self.idx >= N {
            for i in 0..N {
                let y = (self.mt[i] & 0x8000_0000) | (self.mt[(i + 1) % N] & 0x7FFF_FFFF);
                let mut next = self.mt[(i + 397) % N] ^ (y >> 1);
                if y & 1 != 0 {
                    next ^= 0x9908_B0DF;
                }
                self.mt[i] = next;
            }
            self.idx = 0;
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^= y >> 18;
        y
    }

    /// CPython getrandbits(k) for k <= 32 (sufficient for shuffle indices).
    pub fn getrandbits(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        self.genrand_u32() >> (32 - k)
    }

    /// CPython _randbelow_with_getrandbits(n > 0)
    fn randbelow(&mut self, n: usize) -> usize {
        let k = usize::BITS - n.leading_zeros(); // n.bit_length()
        loop {
            let r = self.getrandbits(k) as usize;
            if r < n {
                return r;
            }
        }
    }

    /// CPython random.shuffle(x) — Fisher-Yates from the end.
    pub fn shuffle<T>(&mut self, x: &mut [T]) {
        for i in (1..x.len()).rev() {
            let j = self.randbelow(i + 1);
            x.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_cpython() {
        let mut r = Mt19937::new_seeded(0);
        let got: Vec<u32> = (0..5).map(|_| r.getrandbits(32)).collect();
        assert_eq!(got, vec![3626764237, 1654615998, 3255389356, 3823568514, 1806341205]);

        let mut r = Mt19937::new_seeded(7);
        let got: Vec<u32> = (0..3).map(|_| r.getrandbits(16)).collect();
        assert_eq!(got, vec![21222, 62119, 9886]);

        let mut ids: Vec<i32> = (0..10).collect();
        Mt19937::new_seeded(0).shuffle(&mut ids);
        assert_eq!(ids, vec![7, 8, 1, 5, 3, 4, 2, 0, 9, 6]);

        let mut ids: Vec<i32> = (0..10).collect();
        Mt19937::new_seeded(123).shuffle(&mut ids);
        assert_eq!(ids, vec![8, 7, 5, 9, 2, 3, 6, 1, 4, 0]);

        let mut big: Vec<i32> = (0..5014).collect();
        Mt19937::new_seeded(7).shuffle(&mut big);
        assert_eq!(&big[..10], &[2862, 736, 2702, 840, 1803, 4063, 3, 4488, 2499, 3698]);
    }
}
