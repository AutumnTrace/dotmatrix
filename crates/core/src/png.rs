//! A small PNG writer so screenshots need no dependencies.
//!
//! It emits one fixed-Huffman deflate block with a basic hash-chain LZ77
//! matcher. That is nowhere near zlib's ratio in general, but Game Boy
//! frames are four colours with long runs, which this handles well.

/// DMG shade palettes, lightest first.
pub mod palette {
    /// The neutral greys used by the reference images of most test ROMs.
    pub const GRAYSCALE: [[u8; 3]; 4] =
        [[0xFF, 0xFF, 0xFF], [0xAA, 0xAA, 0xAA], [0x55, 0x55, 0x55], [0x00, 0x00, 0x00]];
    /// The pea-soup green of the original DMG screen.
    pub const DMG_GREEN: [[u8; 3]; 4] =
        [[0x9B, 0xBC, 0x0F], [0x8B, 0xAC, 0x0F], [0x30, 0x62, 0x30], [0x0F, 0x38, 0x0F]];
}

/// Converts a 160x144 shade buffer to RGB, scaled up by an integer factor.
pub fn shades_to_rgb(shades: &[u8], width: usize, palette: &[[u8; 3]; 4], scale: usize) -> Vec<u8> {
    let height = shades.len() / width;
    let mut out = Vec::with_capacity(width * height * scale * scale * 3);
    for row in shades.chunks_exact(width) {
        let mut line = Vec::with_capacity(width * scale * 3);
        for &s in row {
            for _ in 0..scale {
                line.extend_from_slice(&palette[usize::from(s & 3)]);
            }
        }
        for _ in 0..scale {
            out.extend_from_slice(&line);
        }
    }
    out
}

/// Encodes 8-bit RGB (`channels == 3`) or RGBA (`channels == 4`) pixels.
pub fn encode(width: u32, height: u32, channels: u8, pixels: &[u8]) -> Vec<u8> {
    assert!(channels == 3 || channels == 4, "only RGB and RGBA are supported");
    let stride = width as usize * usize::from(channels);
    assert_eq!(pixels.len(), stride * height as usize, "pixel buffer size mismatch");

    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in pixels.chunks_exact(stride) {
        raw.push(0); // filter type: none
        raw.extend_from_slice(row);
    }

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 3 { 2 } else { 6 }, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    bits: u32,
}

impl BitWriter {
    fn put(&mut self, value: u32, count: u32) {
        self.acc |= u64::from(value) << self.bits;
        self.bits += count;
        while self.bits >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.bits -= 8;
        }
    }

    /// Huffman codes are stored most-significant bit first.
    fn put_code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    fn literal(&mut self, v: u32) {
        match v {
            0..=143 => self.put_code(0x30 + v, 8),
            144..=255 => self.put_code(0x190 + v - 144, 9),
            256..=279 => self.put_code(v - 256, 7),
            _ => self.put_code(0xC0 + v - 280, 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

const WINDOW: usize = 32 * 1024;
const HASH_BITS: u32 = 15;
const MAX_CHAIN: usize = 64;

fn hash3(d: &[u8]) -> usize {
    let v = u32::from(d[0]) << 16 | u32::from(d[1]) << 8 | u32::from(d[2]);
    (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(data.len() / 4), acc: 0, bits: 0 };
    w.put(1, 1); // BFINAL
    w.put(1, 2); // BTYPE = fixed Huffman

    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut prev = vec![usize::MAX; data.len()];
    let insert = |head: &mut Vec<usize>, prev: &mut Vec<usize>, pos: usize| {
        if pos + 3 <= data.len() {
            let h = hash3(&data[pos..]);
            prev[pos] = head[h];
            head[h] = pos;
        }
    };

    let mut i = 0;
    while i < data.len() {
        let mut best_len = 0;
        let mut best_dist = 0;
        if i + 3 <= data.len() {
            let mut candidate = head[hash3(&data[i..])];
            let max_len = (data.len() - i).min(258);
            let mut chain = 0;
            while candidate != usize::MAX && i - candidate <= WINDOW && chain < MAX_CHAIN {
                let len = data[candidate..].iter().zip(&data[i..i + max_len]).take_while(|(a, b)| a == b).count();
                if len > best_len {
                    best_len = len;
                    best_dist = i - candidate;
                    if len == max_len {
                        break;
                    }
                }
                candidate = prev[candidate];
                chain += 1;
            }
        }
        if best_len >= 3 {
            let code = LEN_BASE.iter().rposition(|&b| usize::from(b) <= best_len).unwrap();
            w.literal(257 + code as u32);
            w.put((best_len - usize::from(LEN_BASE[code])) as u32, u32::from(LEN_EXTRA[code]));
            let dcode = DIST_BASE.iter().rposition(|&b| usize::from(b) <= best_dist).unwrap();
            w.put_code(dcode as u32, 5);
            w.put((best_dist - usize::from(DIST_BASE[dcode])) as u32, u32::from(DIST_EXTRA[dcode]));
            for p in i..i + best_len {
                insert(&mut head, &mut prev, p);
            }
            i += best_len;
        } else {
            w.literal(u32::from(data[i]));
            insert(&mut head, &mut prev, i);
            i += 1;
        }
    }
    w.literal(256);
    w.finish()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info, buf)
    }

    #[test]
    fn round_trips_through_a_real_decoder() {
        // Structured but not trivially repetitive: exercises literals,
        // short and long matches, and far distances.
        let (w, h) = (97u32, 61u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&[((x * 7) ^ y) as u8, (x / 8 + y / 4) as u8, ((x * y) % 251) as u8]);
            }
        }
        let bytes = encode(w, h, 3, &pixels);
        let (info, decoded) = decode(&bytes);
        assert_eq!((info.width, info.height), (w, h));
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn compresses_flat_frames() {
        let shades = vec![0u8; 160 * 144];
        let rgb = shades_to_rgb(&shades, 160, &palette::DMG_GREEN, 2);
        let bytes = encode(320, 288, 3, &rgb);
        assert!(bytes.len() < 4_000, "flat frame took {} bytes", bytes.len());
        assert_eq!(decode(&bytes).1, rgb);
    }
}
