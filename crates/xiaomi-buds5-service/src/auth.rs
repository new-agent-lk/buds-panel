//! Xiaomi/JieLi RCSP command-auth transform.
//!
//! A line-for-line port of `src/xiaomi_buds5_panel/auth.py` (itself adapted from
//! redmi-buds-linux, MIT). Pure standard library, no vendor binary. The three
//! 256-byte tables and the mixing network are reproduced exactly; the unit test
//! below checks the same three hardware-captured vectors the Python selftest uses.

// Faithful line-for-line port of the reference algorithm: keep its arithmetic
// form (`w = w + …`), rotate-by-constant and indexed loops rather than rewriting
// them to satisfy style lints — the byte-for-byte correspondence is the point.
#![allow(
    clippy::assign_op_pattern,
    clippy::identity_op,
    clippy::manual_rotate,
    clippy::needless_late_init,
    clippy::needless_range_loop
)]

use std::sync::OnceLock;

const TAB_EXPAND_HEX: &str = "64ac285ac9b337c50a10b7a3bab197463d05dc666ef69af80d589567c6aaabec\
a0689b96d4ebbf434936e96a89d8c38a946399bc7bbec122bb5c71d51f92575d\
8f44411d51e64017fbfd193234b8612aca236fda39f7a2017fd631e7de8004dd\
2c5982afa8e00fcda1123e30d11cd03a33722e4f9002130675ce87c2efb2ad7d\
3815e1529f7a6c2f27c4e281a9cf8dc0d7dfff6076148c5e5509e408c74220fc\
d25091d94c629ee8b9a6f91a00210bfa359c4e4b6948cb0ec8a45bea8407b418\
f4ae6bdba7cc3f8b4a0c3c25e5544d4583ed11f0b05393f27426b59d6d7cf32d\
f156247e471b86bd708e1e3b731603b6ac285ac9b337c50a10b7a3bab1974688";

const TAB_A_HEX: &str = "012de293be4515ae780387a4b838cf3f08670994eb26a86bbd18341bbbbf72f7\
4035489c512f3b55e3c09fd8d3f38db1ffa73edc8677d7a611fbf4ba92916483\
f133efda2cb5b22b88d199cb8c841d14819771ca5fa38b573c82c4525c1ce8a0\
04b4854af61354b6df0c1a8edee039fc209b244ea9989eabf260d06ceafac7d9\
00d41f6e43bcec5389fe7a5d49c932c2f99af86d16db599644e9cde646428f0a\
c1ccb965b0d2c6ac1e4162292e0e7450025ac3257b8a2a5bf0060d476f709d7e\
10ce1227d54c4fd679306836757de4ed806a9037a25e76aac57f3dafa5e51961\
fd4d7cb70beead4b22f5e7732321c805e166ddb3586963560fa1319517073a28";

const TAB_B_HEX: &str = "8000b00960efb9fd10129fe469baadf8c038c2654f0694fc19de6a1b5d4ea882\
70ede8ec72b315c3ffabb6474401ac25c9fa8e411a21cbd30d6efe2658da320f\
20a99d8498059cbb228c63e7c5e173c6af245b876627f757f496b1b75c8bd554\
79dfaaf63ea3f111caf5d1177b9383bcbd521eebaeccd63508c88ab4e2cdbfd9\
d050593f4d62340a4888b5564c2e6b9ed23d3c0313fb9751754a917123be762a\
5ff9d4550bdc37311674d777a7e607dba42f46f3614567e30ca23b1c8518041d\
29a08fb25ad8a67eee8d534ba19ac10e7a49a52c81c4c7362b7f439533f26c68\
6df00228cedd9bea5e997c1486cfe542b840782d3ae9641f92907d396fe08930";

/// The app always passes this fixed placeholder address (never the device MAC).
pub const ADDR6_PLACEHOLDER: [u8; 6] = [0x11, 0x22, 0x33, 0x33, 0x22, 0x11];

const XOR_MASK: u32 = 0x9999;

fn hex_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16).expect("hex") as u8;
        let lo = (bytes[i + 1] as char).to_digit(16).expect("hex") as u8;
        out.push((hi << 4) | lo);
        i += 2;
    }
    out
}

fn cached<'a>(hex: &str, cell: &'a OnceLock<Vec<u8>>) -> &'a [u8] {
    cell.get_or_init(|| hex_decode(hex)).as_slice()
}

fn tab_expand() -> &'static [u8] {
    static C: OnceLock<Vec<u8>> = OnceLock::new();
    cached(TAB_EXPAND_HEX, &C)
}
fn tab_a() -> &'static [u8] {
    static C: OnceLock<Vec<u8>> = OnceLock::new();
    cached(TAB_A_HEX, &C)
}
fn tab_b() -> &'static [u8] {
    static C: OnceLock<Vec<u8>> = OnceLock::new();
    cached(TAB_B_HEX, &C)
}

#[inline]
fn rol8(b: u8, n: u32) -> u8 {
    ((b << n) | (b >> (8 - n))) & 0xFF
}

/// Expand a 16-byte key into the 272-byte round schedule (sub_1630).
fn expand_key(key: &[u8; 16]) -> [u8; 272] {
    let mut buf = [0u8; 272];
    buf[0..16].copy_from_slice(key);

    let mut v = [0u8; 16];
    for i in 0..16 {
        v[i] = if i < 8 { key[i] ^ key[i + 8] } else { key[i] };
    }
    let mut checksum: u8 = 0;
    for &b in &v[..8] {
        checksum ^= b;
    }

    let mut state = *key;
    for r in 0..16 {
        for b in state.iter_mut() {
            *b = rol8(*b, 3);
        }
        checksum = rol8(checksum & 0xFF, 3);
        let mut window = [0u8; 17];
        window[0..16].copy_from_slice(&state);
        window[16] = checksum & 0xFF;
        let table_base = r * 16;
        let out_base = 16 + r * 16;
        for j in 0..16 {
            buf[out_base + j] =
                tab_expand()[table_base + 15 - j].wrapping_add(window[(r + 1 + j) % 17]);
        }
    }
    buf
}

/// One unrolled linear mixing block over the 16 state bytes (sub_176c).
fn round16(b: [i32; 16]) -> [i32; 16] {
    let (b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11, b12, b13, b14, b15) = (
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15],
    );

    let mut w7 = b1 + 2 * b0;
    let mut w16 = b1 + b0;
    let mut w20 = b3 + 2 * b2;
    let mut w3 = b3 + b2;
    let mut w22 = b5 + 2 * b4;
    let mut w5 = b5 + b4;
    let mut w24 = b7 + 2 * b6;
    let mut w17 = b7 + b6;
    let mut w26 = b9 + 2 * b8;
    let mut w4 = b9 + b8;
    let mut w28 = b11 + 2 * b10;
    let mut w6 = b11 + b10;
    let mut w23 = b13 + 2 * b12;
    let mut w19 = b13 + b12;
    let mut w25 = b15 + 2 * b14;
    let mut w21 = b15 + b14;
    let mut w27;
    let w30;

    w27 = w6 + 2 * w26;
    w6 = w6 + w26;
    w26 = w21 + 2 * w23;
    w21 = w21 + w23;
    w23 = w16 + 2 * w20;
    w16 = w20 + w16;
    w20 = w5 + 2 * w24;
    w5 = w24 + w5;
    w24 = w4 + 2 * w28;
    w4 = w28 + w4;
    w28 = w19 + 2 * w25;
    w19 = w25 + w19;
    w25 = w17 + 2 * w7;
    w17 = w17 + w7;
    w7 = w3 + 2 * w22;
    w3 = w22 + w3;
    w22 = w19 + 2 * w24;
    w19 = w19 + w24;
    w24 = w3 + 2 * w25;
    w3 = w25 + w3;
    w25 = w16 + 2 * w20;
    w16 = w20 + w16;
    w20 = w4 + 2 * w28;
    w4 = w28 + w4;
    w28 = w17 + 2 * w7;
    w17 = w17 + w7;
    w30 = w20 + w17;
    w17 = w17 + 2 * w20;
    w7 = w5 + 2 * w27;
    w5 = w27 + w5;
    w27 = w21 + 2 * w23;
    w21 = w21 + w23;

    let mut o = [0i32; 16];
    o[0] = w17;
    w17 = w19 + w24;
    w23 = w26 + w6;
    w19 = w19 + 2 * w24;
    w20 = w21 + w7;
    w7 = w21 + 2 * w7;
    o[5] = w17;
    w17 = w23 + 2 * w25;
    o[4] = w19;
    w19 = w4 + w28;
    w4 = w4 + 2 * w28;
    o[2] = w7;
    o[6] = w17;
    w17 = w27 + w5;
    w7 = w23 + w25;
    w5 = w5 + 2 * w27;
    o[8] = w4;
    w4 = w22 + w16;
    w16 = w16 + 2 * w22;
    o[11] = w17;
    w17 = w6 + 2 * w26;
    o[1] = w30;
    o[13] = w4;
    w4 = w17 + w3;
    o[12] = w16;
    w16 = w3 + 2 * w17;
    o[3] = w20;
    o[7] = w7;
    o[9] = w19;
    o[10] = w5;
    o[15] = w4;
    o[14] = w16;
    let mut out = [0i32; 16];
    for i in 0..16 {
        out[i] = o[i] & 0xFF;
    }
    out
}

/// The 8-round mixing network (sub_176c).
fn mix(state: &[u8; 16], sched: &[u8; 272], flag: bool) -> [u8; 16] {
    let mut out: [i32; 16] = std::array::from_fn(|i| state[i] as i32);
    let orig = out;

    let sel = |i: usize| ((1u32 << i) & XOR_MASK) != 0;

    for rnd in 0..8usize {
        let base = rnd * 32;
        if flag && rnd == 2 {
            for i in 0..16 {
                out[i] = if sel(i) {
                    out[i] ^ orig[i]
                } else {
                    out[i] + orig[i]
                };
            }
        }
        for i in 0..16 {
            let v = sched[base + i] as i32;
            out[i] = if sel(i) { out[i] ^ v } else { out[i] + v };
        }
        for i in 0..16 {
            let t = (out[i] & 0xFF) as usize;
            out[i] = (if sel(i) { tab_a() } else { tab_b() })[t] as i32;
        }
        for i in 0..16 {
            let v = sched[base + 16 + i] as i32;
            out[i] = if sel(i) { out[i] + v } else { out[i] ^ v };
        }
        out = round16(std::array::from_fn(|i| out[i] & 0xFF));
    }

    for i in 0..16 {
        let v = sched[0x100 + i] as i32;
        out[i] = if sel(i) { out[i] ^ v } else { out[i] + v };
    }
    std::array::from_fn(|i| (out[i] & 0xFF) as u8)
}

/// The 16-byte auth response for a 16-byte challenge.
pub fn transform(addr6: &[u8; 6], challenge: &[u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..6].copy_from_slice(addr6);
    out[6..12].copy_from_slice(addr6);
    out[12..16].copy_from_slice(&addr6[0..4]);
    let mut tmp = *challenge;
    tmp[15] ^= 0x06;
    let sched = expand_key(&tmp);
    mix(&out, &sched, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_vectors_match() {
        let cases = [
            (
                "385c2aecb03bfb32af3c54ec18db5c02",
                "99a16782b8e05df000099bc2cb3417bf",
            ),
            (
                "1afe43fbfaaa3afb29d1e6053c7c9475",
                "f011ccd271c9d429d14bd4887d825611",
            ),
            (
                "acbb8a57b2d58c8aa24d2fb232cbddb7",
                "e696a4c36f460b55ca6f5b8fb0a4c7f8",
            ),
        ];
        for (chal, want) in cases {
            let got = transform(&ADDR6_PLACEHOLDER, &hex_decode(chal).try_into().unwrap());
            let got_hex: String = got.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(got_hex, want, "challenge {chal}");
        }
    }
}
