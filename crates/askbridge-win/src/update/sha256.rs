const INITIAL: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Incremental hashing for streamed downloads: feed chunks as they arrive from
/// the network and finalize once the stream ends.
pub(super) struct Sha256Stream(Sha256);

impl Sha256Stream {
    pub(super) fn new() -> Self {
        Self(Sha256::new())
    }

    pub(super) fn update(&mut self, input: &[u8]) {
        self.0.update(input);
    }

    pub(super) fn finish_hex(&mut self) -> String {
        self.0.finish_hex()
    }
}

struct Sha256 {
    hash: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            hash: INITIAL,
            block: [0; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.total_len = self
            .total_len
            .checked_add(input.len() as u64)
            .expect("SHA-256 input length overflow");
        if self.block_len > 0 {
            let needed = 64 - self.block_len;
            let copied = needed.min(input.len());
            self.block[self.block_len..self.block_len + copied].copy_from_slice(&input[..copied]);
            self.block_len += copied;
            input = &input[copied..];
            if self.block_len < 64 {
                return;
            }
            compress(&mut self.hash, &self.block);
            self.block_len = 0;
        }
        while input.len() >= 64 {
            let block: &[u8; 64] = input[..64].try_into().expect("block");
            compress(&mut self.hash, block);
            input = &input[64..];
        }
        self.block[..input.len()].copy_from_slice(input);
        self.block_len = input.len();
    }

    fn finish_hex(&mut self) -> String {
        let bit_len = self.total_len.checked_mul(8).expect("SHA-256 bit length");
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            compress(&mut self.hash, &self.block);
            self.block = [0; 64];
            self.block_len = 0;
        } else {
            self.block[self.block_len..56].fill(0);
        }
        self.block[56..].copy_from_slice(&bit_len.to_be_bytes());
        compress(&mut self.hash, &self.block);
        self.hash.iter().map(|word| format!("{word:08X}")).collect()
    }
}

fn compress(hash: &mut [u32; 8], block: &[u8; 64]) {
    let mut words = [0_u32; 64];
    for (index, chunk) in block.chunks_exact(4).enumerate() {
        words[index] = u32::from_be_bytes(chunk.try_into().expect("word"));
    }
    for index in 16..64 {
        let s0 = words[index - 15].rotate_right(7)
            ^ words[index - 15].rotate_right(18)
            ^ (words[index - 15] >> 3);
        let s1 = words[index - 2].rotate_right(17)
            ^ words[index - 2].rotate_right(19)
            ^ (words[index - 2] >> 10);
        words[index] = words[index - 16]
            .wrapping_add(s0)
            .wrapping_add(words[index - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *hash;
    for index in 0..64 {
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choice = (e & f) ^ (!e & g);
        let temporary1 = h
            .wrapping_add(sum1)
            .wrapping_add(choice)
            .wrapping_add(K[index])
            .wrapping_add(words[index]);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temporary2 = sum0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temporary1);
        d = c;
        c = b;
        b = a;
        a = temporary1.wrapping_add(temporary2);
    }
    for (value, addition) in hash.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *value = value.wrapping_add(addition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_all(input: &[u8]) -> String {
        let mut state = Sha256Stream::new();
        state.update(input);
        state.finish_hex()
    }

    #[test]
    fn matches_known_sha256_vectors() {
        for (input, expected) in [
            (
                b"".as_slice(),
                "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
            ),
            (
                b"abc".as_slice(),
                "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
            ),
            // Standard 56-byte vector: padding must spill into a second block.
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".as_slice(),
                "248D6A61D20638B8E5C026930C3E6039A33CE45964FF2167F6ECEDD419DB06C1",
            ),
            // Input longer than one full block exercises the streaming path.
            (
                b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmno".as_slice(),
                "2FF100B36C386C65A1AFC462AD53E25479BEC9498ED00AA5A04DE584BC25301B",
            ),
        ] {
            assert_eq!(hash_all(input), expected);
        }
    }

    #[test]
    fn streamed_updates_match_one_shot_hashing() {
        let input = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let mut streamed = Sha256Stream::new();
        for chunk in input.chunks(7) {
            streamed.update(chunk);
        }
        assert_eq!(streamed.finish_hex(), hash_all(input));
    }
}
