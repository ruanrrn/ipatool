// Port of ApplePackage's rng_f1.c (via AssppWeb random.ts). All arithmetic wraps at 32 bits.
use super::tables::Tables;

#[derive(Clone)]
pub struct SapRandom {
    pub words: [u32; 624],
    pub cursor: usize,
}

impl SapRandom {
    pub fn new() -> Self {
        SapRandom {
            words: [0; 624],
            cursor: 0,
        }
    }

    pub fn word(&mut self) -> u32 {
        for index in 0..624 {
            let value =
                (self.words[index] & 0x8000_0000) | (self.words[(index + 1) % 624] & 0x7fff_ffff);
            self.words[index] = self.words[(index + 397) % 624]
                ^ (value >> 1)
                ^ if value & 1 != 0 { 0x9908_b0df } else { 0 };
        }
        if self.cursor >= 624 {
            self.cursor = 0;
        }
        let mut value = self.words[self.cursor];
        self.cursor += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^ (value >> 18)
    }

    pub fn fill(
        &mut self,
        seed: u32,
        address: u32,
        table: &[u8],
        mask: u8,
        count: usize,
    ) -> Vec<u8> {
        let mut output = vec![0u8; count];
        let mut remaining = count;
        while remaining > 0 {
            let word = self.word();
            let mut shift = 0;
            while shift < 32 && remaining > 0 {
                let mut hash = (seed ^ address.wrapping_add(remaining as u32 - 1)) & 0x7fff_ffff;
                hash = hash.wrapping_mul(0x84ed_b2eb);
                hash ^= hash >> 16;
                hash = hash.wrapping_mul(0x84ed_b2eb);
                let value =
                    ((word >> shift) ^ hash ^ table[(hash >> 24) as usize] as u32 ^ mask as u32)
                        as u8;
                output[remaining - 1] = value;
                if value != 0 {
                    remaining -= 1;
                }
                shift += 8;
            }
        }
        output
    }

    pub fn initialize(
        &mut self,
        entropy: [u32; 3],
        seconds: i64,
        microseconds: i64,
        tables: &Tables,
    ) -> Result<[u32; 128], String> {
        if !(0..1_000_000).contains(&microseconds) {
            return Err("Invalid SAP entropy or time".to_string());
        }
        // Uint32Array semantics: values wrap modulo 2^32.
        let mut keys = [entropy[2], microseconds as u32, seconds as u32, entropy[1]];
        let words = &mut self.words;
        words[0] = entropy[0];
        for index in 1..624 {
            let previous = words[index - 1];
            words[index] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(index as u32);
        }
        let mut index = 1;
        let mut key = 0;
        for _ in 0..624 {
            let previous = words[index - 1];
            words[index] = (words[index] ^ (previous ^ (previous >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(keys[key])
                .wrapping_add(key as u32);
            index += 1;
            if index == 624 {
                words[0] = words[623];
                index = 1;
            }
            key += 1;
            if key == 4 {
                keys[0] ^= 0x200;
                key = 0;
            } else {
                keys[key] ^= 0x201;
            }
        }
        for _ in 0..623 {
            let previous = words[index - 1];
            words[index] = (words[index]
                ^ (previous ^ (previous >> 30)).wrapping_mul(1_566_083_941))
            .wrapping_sub(index as u32);
            index += 1;
            if index == 624 {
                words[0] = words[623];
                index = 1;
            }
        }
        words[0] = 0x8000_0000;
        self.cursor = 0;
        let mut pool = [0u32; 128];
        for value in pool.iter_mut() {
            *value = self.word();
        }
        for step in (0..64).step_by(4) {
            let other = pool[(pool[step] & 62) as usize];
            pool[step] |= (!other) << 16;
        }
        self.fill(pool[12], 0x7f_f9b0, &tables.initialize_whiten, 0x35, 1024);
        Ok(pool)
    }

    pub fn f1(&mut self, seed: u32, address: u32, tables: &Tables) -> Vec<u8> {
        let mut material = self.fill(seed, address, &tables.f1_whiten, 0x27, 60);
        material.extend(self.fill(
            seed,
            address.wrapping_add(0xaa),
            &tables.f1_whiten,
            0x27,
            18,
        ));
        let mut pre = tables.f1_template.clone();
        pre[68..128].copy_from_slice(&material[..60]);
        pre[238..256].copy_from_slice(&material[60..]);
        let g = &tables.f1_g;
        let mut output = vec![0u8; 256];
        for half in 0..2 {
            let mut low = pre[half * 128..half * 128 + 64].to_vec();
            let mut high = pre[half * 128 + 64..half * 128 + 128].to_vec();
            for _ in 0..7 {
                let mut state = [0u8; 80];
                state[..64].copy_from_slice(&high);
                state[64..].copy_from_slice(&tables.f1_rc);
                for (index, value) in state.iter_mut().enumerate() {
                    *value = (*value ^ 0x2d).wrapping_add(index as u8);
                }
                for (index, value) in state.iter_mut().enumerate() {
                    *value = g[0][*value as usize]
                        .wrapping_add(0x25)
                        .wrapping_add(index as u8);
                }
                for constant in [0x5du8, 0xae, 0xff, 0x50] {
                    for index in 0..80 {
                        state[index] ^= constant
                            .wrapping_add(index as u8)
                            .wrapping_add(g[1][state[(index + 75) % 80] as usize])
                            .wrapping_add(g[2][state[(index + 55) % 80] as usize]);
                    }
                }
                for (index, value) in state.iter_mut().enumerate() {
                    *value = g[3][*value as usize]
                        .wrapping_add(0xc4)
                        .wrapping_add(index as u8);
                }
                let next: Vec<u8> = low
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| byte ^ state[index])
                    .collect();
                low = high;
                high = next;
            }
            for (index, byte) in low.iter().enumerate() {
                output[half * 128 + index] = byte ^ 0x2d;
            }
            for (index, byte) in high.iter().enumerate() {
                output[half * 128 + 64 + index] = byte ^ 0x2d;
            }
        }
        output
    }
}
