// SAP white-box block cipher (port of AssppWeb cipher.ts, from ApplePackage).
use super::source::{prepare_source, PreparedSource, INITIAL_ORDER, TERMINAL_ORDER};
use super::tables::Tables;

const TERMINAL_DESTINATIONS: [usize; 16] = [13, 5, 10, 2, 14, 1, 11, 8, 6, 9, 0, 15, 4, 7, 3, 12];

pub struct SapCipher {
    tables: &'static Tables,
    source: PreparedSource,
    seeds: [u8; 7],
    raw_words: Vec<[u32; 4]>,
    round_keys: Vec<[u8; 16]>,
    terminal_words: Vec<[u32; 256]>,
    initial_tables: Vec<[u8; 256]>,
    carry_frames: [[u8; 4]; 3],
    terminal_cache: Vec<u8>,
    lane_keys: [u32; 16],
    /// Indices into `tables.bytes` for each stage and lane.
    nonlinear: Vec<[usize; 16]>,
}

impl SapCipher {
    pub fn new(input: &[u8], tables: &'static Tables) -> Result<Self, String> {
        let source = prepare_source(input, tables)?;
        let m = source.material;
        let pair_word = |at: usize, mask: usize| -> u32 {
            let mut word = 0u32;
            for byte in 0..4 {
                word |= ((m[at + byte] ^ m[0x120 + (mask + byte) % 16]) as u32) << (8 * byte);
            }
            word
        };

        let mut initial = source.initial_cache;
        let mut terminal = source.terminal_cache;
        for (row, &slot) in INITIAL_ORDER.iter().enumerate() {
            if initial[slot] == 0 {
                initial[slot] = pair_word(0x180 + 4 * row, 4 * (row % 4)) ^ 0xe628_18fc;
            }
        }
        for (row, &slot) in TERMINAL_ORDER.iter().enumerate() {
            if terminal[slot] == 0 {
                let offset = if row < 5 {
                    0
                } else if row < 13 {
                    4
                } else {
                    8
                };
                terminal[slot] =
                    pair_word(0x130 + 4 * row + offset, 4 * row + offset) ^ 0xaf61_5514;
            }
        }

        let mut initial_field = [0u8; 16];
        for (index, value) in initial_field.iter_mut().enumerate() {
            let left = (3 * index + 7) % 16;
            let right = (11 * index + 2) % 16;
            *value = (m[0x1e0 + left] ^ m[0x70 + left])
                .wrapping_sub(m[0x20 + right] ^ m[0x70 + right])
                ^ 0x2d;
        }
        let seeds = [
            m[0x1f2] ^ m[0x72],
            m[0x1f7] ^ m[0x1e7],
            0x24,
            m[0x1eb] ^ m[0x2b],
            m[0x75] ^ m[0x1f5],
            0x11,
            m[0x23] ^ m[0x73],
        ];
        let mut raw_words: Vec<[u32; 4]> = (0..9)
            .map(|row| {
                let physical: Vec<u32> = (0..4)
                    .map(|column| {
                        let index = 4 * row + 36 * column;
                        pair_word(0x130 + index, index).wrapping_add(0x2186_d6ff)
                    })
                    .collect();
                [physical[1], physical[3], physical[0], physical[2]]
            })
            .collect();
        raw_words.reverse();

        let pairs = [
            (0, 1),
            (8, 9),
            (22, 23),
            (28, 29),
            (20, 21),
            (30, 31),
            (24, 25),
            (14, 15),
            (2, 3),
            (12, 13),
            (6, 7),
            (16, 17),
            (4, 5),
            (26, 27),
            (18, 19),
            (10, 11),
        ];
        let round_keys = [0x100, 0x30, 0xa0, 0x80, 0x50, 0, 0xc0, 0x1c0, 0x200]
            .iter()
            .map(|&offset| {
                let mut key = [0u8; 16];
                for (lane, (a, b)) in pairs.iter().enumerate() {
                    key[lane] = m[offset + a].wrapping_add(m[offset + b]);
                }
                key
            })
            .collect();

        let terminal_words = (0..16)
            .map(|slot| {
                let mask = (m[0x1f0 + slot] ^ m[0x70 + slot])
                    .wrapping_add(m[0x20 + slot] ^ m[0x70 + slot]);
                let indices = &tables.bytes[40 + (79 * slot) % 144];
                let mut words = [0u32; 256];
                for (index, word) in words.iter_mut().enumerate() {
                    *word = source.auxiliary0[(indices[index] ^ mask) as usize];
                }
                words
            })
            .collect();

        let byte_caches: Vec<Vec<u8>> = (0..16)
            .map(|slot| {
                let index = (13 * slot) % 16;
                let mask = (m[0x1e0 + index] ^ m[0x70 + index])
                    .wrapping_sub(m[0x20 + index] ^ m[0x70 + index])
                    ^ 0xfe;
                let table = &tables.bytes[184 + (9 * slot) % 16];
                source
                    .cache2
                    .iter()
                    .map(|&value| table[(value ^ mask) as usize])
                    .collect()
            })
            .collect();
        let initial_tables = (0..16)
            .map(|slot| {
                let mut result = [0u8; 256];
                for (byte, value) in result.iter_mut().enumerate() {
                    let word = source.auxiliary1[byte ^ 0x2d] ^ initial[INITIAL_ORDER[slot]];
                    let index = ((word ^ 0x4ca8_8d12) & source.projection_mask) as usize
                        % source.cache2.len();
                    *value = byte_caches[(5 * slot) % 16][index];
                }
                result
            })
            .collect();

        let field_words = [
            raw_words[3][2],
            raw_words[3][0],
            raw_words[3][3],
            raw_words[3][1],
        ]
        .map(|word| word.wrapping_add(0xde79_2901));
        let mut updated_field = [0u8; 16];
        for (index, value) in updated_field.iter_mut().enumerate() {
            let modulus = [233, 239, 241, 251][index % 4];
            *value = initial_field[index]
                ^ tables.masks[index % 4][(field_words[index / 4] % modulus) as usize]
                ^ [0xf6, 0x19, 0x50, 0x8e][index % 4];
        }
        let pick = |field: &[u8; 16], indices: [usize; 4]| indices.map(|index| field[index]);
        let carry_frames = [
            pick(&initial_field, [0, 2, 11, 12]),
            pick(&updated_field, [6, 1, 3, 14]),
            pick(&updated_field, [8, 9, 13, 15]),
        ];
        let terminal_cache = source
            .cache3
            .iter()
            .enumerate()
            .map(|(index, &value)| value ^ ((index >> 2) & 255) as u8 ^ 0xfa)
            .collect();
        let coefficient_order = [3, 1, 6, 8, 10, 4, 0, 12, 7, 2, 13, 5, 14, 15, 9, 11];
        let mut lane_keys = [0u32; 16];
        for (lane, &destination) in TERMINAL_DESTINATIONS.iter().enumerate() {
            lane_keys[lane] = terminal[coefficient_order[lane]]
                ^ source.auxiliary0[(updated_field[destination] ^ 0x2d) as usize]
                ^ 0xaf61_5514;
        }
        let key_order = [12, 0, 1, 8, 5, 7, 9, 4, 2, 14, 11, 13, 3, 10, 15, 6];
        let rounds = [2, 4, 8, 3, 7, 2, 6, 1, 5, 0, 4, 8, 3, 7, 2, 6];
        let nonlinear = (0..8)
            .map(|stage| {
                let mut result = [0usize; 16];
                for (lane, &position) in [4, 0, 15, 8, 11, 9, 7, 12, 14, 2, 5, 3, 13, 6, 1, 10]
                    .iter()
                    .enumerate()
                {
                    let round = (rounds[position] + 2 * stage) % 9;
                    result[key_order[lane]] = 40 + 16 * round + position;
                }
                result
            })
            .collect();

        Ok(SapCipher {
            tables,
            source,
            seeds,
            raw_words,
            round_keys,
            terminal_words,
            initial_tables,
            carry_frames,
            terminal_cache,
            lane_keys,
            nonlinear,
        })
    }

    pub fn encrypt(&self, block: &[u8]) -> Result<[u8; 16], String> {
        if block.len() != 16 {
            return Err("SAP cipher requires a 16-byte block".to_string());
        }
        let tables = self.tables;
        let source = &self.source;
        let mut masked = [0u8; 16];
        for (index, value) in masked.iter_mut().enumerate() {
            *value = block[index] ^ 0x2d;
        }
        let mut state = [0u8; 17];
        for (index, &destination) in [4, 5, 1, 13, 15, 0, 9, 8, 16, -1, 12, 10, 11, 3, 2, 14]
            .iter()
            .enumerate()
        {
            if destination >= 0 {
                state[destination as usize] = self.initial_tables[index][masked[index] as usize];
            }
        }
        state[6] = state[4] ^ state[14] ^ state[15];
        state[7] = state[6] ^ self.initial_tables[9][masked[9] as usize];

        let mut carries = [0u8; 3];
        for (index, frame) in self.carry_frames.iter().enumerate() {
            let offset = 4 * index;
            let mut value = (frame[0] ^ 0x2d)
                .wrapping_add(masked[offset] ^ [0x20, 0x26, 0x28][index])
                .wrapping_sub(frame[1] ^ 0x2d);
            value ^= masked[offset + 1] ^ 0x2d;
            value = value
                .wrapping_add(frame[2] ^ 0x2d)
                .wrapping_sub(masked[offset + 2] ^ 0x2d);
            carries[index] = (value ^ frame[3] ^ 0x2d).wrapping_add(masked[offset + 3] ^ 0x2d);
        }
        let mut expanded = [0u8; 17];
        for (index, value) in expanded.iter_mut().enumerate() {
            *value = (masked[(3 + 7 * index) % 16] ^ 0x2d).wrapping_add(index as u8) ^ 0x2d;
        }

        let mut slices: [[u8; 17]; 9] = [[0; 17]; 9];
        slices[0] = expanded;
        slices[8] = expanded;
        let index_sources = [15, 5, 2, 10, 11, 7, 9, 13, 4, 3, 12, 8, 16, 0, 1, 14];
        let first_tables: [i32; 16] =
            [28, 39, 50, 61, 28, 39, 50, 61, 56, 67, 6, 17, 56, 67, 6, 17];
        let columns: [i32; 17] = [1, 3, 1, 2, 3, 3, 2, 3, -1, 0, 2, 0, 0, 2, 1, 0, 1];
        let shifts = [24, 0, 8, 0, 8, 24, 24, 16, 0, 8, 8, 16, 0, 16, 0, 24, 16];
        let masks = [
            0x2d, 0x2d, 0x2d, 0x2d, 0x2d, 0x2d, 0x2d, 0x2d, 0, 0xa0, 0x2d, 0x75, 0x32, 0, 0x2d,
            0xbb, 0x2d,
        ];
        for stage in 0..9 {
            let carry = carries[if stage < 4 {
                0
            } else if stage < 8 {
                1
            } else {
                2
            }];
            let previous = slices[[8, 0, 0, 1, 1, 1, 4, 2, 2][stage]];
            let key = &self.round_keys[stage];
            let mut accumulators = self.raw_words[stage].map(|word| word.wrapping_add(0xde79_2901));
            accumulators[0] ^= 0x9658_8d1f;
            for lane in 0..16 {
                let index =
                    state[index_sources[lane]] ^ key[lane] ^ if lane == 5 { state[6] } else { 0 };
                let table = ((first_tables[lane] - 16 * stage as i32 + 144) % 72) as usize;
                accumulators[lane / 4] ^= tables.words[table][index as usize];
            }
            let mut mixed = [0u8; 17];
            for position in 0..17 {
                if position != 8 {
                    mixed[position] = (accumulators[columns[position] as usize] >> shifts[position])
                        as u8
                        ^ masks[position];
                }
            }
            mixed[8] = previous[16]
                ^ previous[8]
                ^ ((accumulators[0] >> 16) & 255) as u8
                ^ 0x75
                ^ state[9]
                ^ key[6]
                ^ carry;
            mixed[13] ^= mixed[8];

            if stage == 8 {
                let coupled = mixed[8] ^ mixed[13];
                let left = ((((mixed[12] ^ 0x31) as i32) + 0x6a) ^ 0x2d) - 0x80;
                let mut middle = (((mixed[14] ^ 0xd2) as i32) + 0x4c + left) & 255;
                middle = (((middle ^ mixed[13] as i32) + 1) ^ 0x7e) & 255;
                let mixing = ((middle + (mixed[15] ^ 0x2d) as i32) & 255) as u8;
                let mut result = source.selectors.map(|selector| self.seeds[selector]);
                for (lane, &position) in [4, 10, -1, 7, 11, 2, 0, 1, 16, 9, 3, 6, 12, 5, 15, 14]
                    .iter()
                    .enumerate()
                {
                    let index = if position < 0 {
                        coupled
                    } else {
                        mixed[position as usize] ^ 0x2d
                    };
                    let destination = TERMINAL_DESTINATIONS[lane];
                    let mut word = self.terminal_words[destination][index as usize];
                    if position < 0 {
                        let aux = &source.auxiliary0;
                        word ^= aux[(coupled ^ mixing) as usize]
                            ^ aux[mixing as usize]
                            ^ aux[coupled as usize]
                            ^ aux[0];
                    }
                    word ^= self.lane_keys[lane];
                    result[destination] ^= self.terminal_cache
                        [(word & source.projection_mask) as usize % self.terminal_cache.len()];
                }
                return Ok(result.map(|value| value ^ 0x2d));
            }

            slices[stage] = mixed;
            let nonlinear = &self.nonlinear[stage];
            let lookup = |slot: usize, value: u8| tables.bytes[nonlinear[slot]][value as usize];
            state = [0u8; 17];
            let destinations = [0, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16];
            let order = [9, 6, 1, 0, 5, 3, 14, 7, 12, 2, 15, 11, 4, 8];
            let inputs = [9, 16, 2, 3, 10, 15, 11, 5, 14, 7, 6, 0, 12, 1];
            for (lane, &destination) in destinations.iter().enumerate() {
                state[destination] = lookup(order[lane], mixed[inputs[lane]] ^ 0x2d);
            }
            state[1] = lookup(10, mixed[8] ^ mixed[13]);
            state[6] = carry ^ state[3] ^ state[4] ^ state[5];
            state[7] = state[6] ^ lookup(13, mixed[4] ^ 0x2d);
        }
        Err("Invalid SAP cipher stage".to_string())
    }
}
