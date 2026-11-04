use std::io::{Read, Seek, SeekFrom};

/// Helper trait for reading primitive types in little-endian order
pub trait ReaderExt: Read {
    fn read_u8(&mut self) -> std::io::Result<u8>;
    fn read_f32_le(&mut self) -> std::io::Result<f32>;
    fn read_f64_le(&mut self) -> std::io::Result<f64>;
    fn read_i16_le(&mut self) -> std::io::Result<i16>;
    fn read_u16_le(&mut self) -> std::io::Result<u16>;
    fn read_i32_le(&mut self) -> std::io::Result<i32>;
    fn read_u32_le(&mut self) -> std::io::Result<u32>;
    fn read_i64_le(&mut self) -> std::io::Result<i64>;
    fn read_u64_le(&mut self) -> std::io::Result<u64>;
    fn read_bytes(&mut self, len: usize) -> std::io::Result<Vec<u8>>;
    fn read_string(&mut self, len: usize) -> std::io::Result<String>;
    fn read_string_z(&mut self, max_len: usize) -> std::io::Result<String>;
    fn read_until_null(&mut self) -> std::io::Result<Vec<u8>>;
    fn read_magic(&mut self, len: usize) -> std::io::Result<String>;
}

impl<R: Read> ReaderExt for R {
    fn read_u8(&mut self) -> std::io::Result<u8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_f32_le(&mut self) -> std::io::Result<f32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(f32::from_le_bytes(buf))
    }

    fn read_f64_le(&mut self) -> std::io::Result<f64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(f64::from_le_bytes(buf))
    }

    fn read_i16_le(&mut self) -> std::io::Result<i16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(i16::from_le_bytes(buf))
    }

    fn read_u16_le(&mut self) -> std::io::Result<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    fn read_i32_le(&mut self) -> std::io::Result<i32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    fn read_u32_le(&mut self) -> std::io::Result<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn read_i64_le(&mut self) -> std::io::Result<i64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }

    fn read_u64_le(&mut self) -> std::io::Result<u64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn read_bytes(&mut self, len: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read_string(&mut self, len: usize) -> std::io::Result<String> {
        let bytes = self.read_bytes(len)?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    fn read_string_z(&mut self, max_len: usize) -> std::io::Result<String> {
        let mut result = Vec::new();
        for _ in 0..max_len {
            let mut buf = [0u8; 1];
            self.read_exact(&mut buf)?;
            if buf[0] == 0 {
                break;
            }
            result.push(buf[0]);
        }
        Ok(String::from_utf8_lossy(&result).to_string())
    }

    fn read_until_null(&mut self) -> std::io::Result<Vec<u8>> {
        let mut result = Vec::new();
        loop {
            let mut buf = [0u8; 1];
            self.read_exact(&mut buf)?;
            if buf[0] == 0 {
                break;
            }
            result.push(buf[0]);
        }
        Ok(result)
    }

    fn read_magic(&mut self, len: usize) -> std::io::Result<String> {
        let bytes = self.read_bytes(len)?;
        Ok(String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_string())
    }
}

/// A wrapper around a stream that provides convenience methods for reading binary data
/// in little-endian format, mirroring C#'s BinaryReader.
pub struct PtclReader<R: Read + Seek> {
    inner: R,
}

impl<R: Read + Seek> PtclReader<R> {
    pub fn new(inner: R) -> Self {
        PtclReader { inner }
    }

    pub fn get_inner(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Gets the current position in the stream.
    pub fn position(&mut self) -> u64 {
        self.inner.stream_position().unwrap()
    }

    /// Seeks to an absolute position in the stream.
    pub fn seek(&mut self, position: u64) {
        self.inner.seek(SeekFrom::Start(position)).unwrap();
    }

    /// Reads a single byte.
    pub fn read_u8(&mut self) -> u8 {
        let mut buf = [0u8; 1];
        self.inner.read_exact(&mut buf).unwrap();
        buf[0]
    }

    /// Reads a signed byte.
    pub fn read_i8(&mut self) -> i8 {
        self.read_u8() as i8
    }

    /// Reads a boolean (stored as a byte).
    pub fn read_bool(&mut self) -> bool {
        self.read_u8() != 0
    }

    /// Reads an unsigned 16-bit integer (little-endian).
    pub fn read_u16(&mut self) -> u16 {
        let mut buf = [0u8; 2];
        self.inner.read_exact(&mut buf).unwrap();
        u16::from_le_bytes(buf)
    }

    /// Reads a signed 16-bit integer (little-endian).
    pub fn read_i16(&mut self) -> i16 {
        self.read_u16() as i16
    }

    /// Reads an unsigned 24-bit value (3 bytes, little-endian) returned as u32.
    pub fn read_u24(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        self.inner.read_exact(&mut buf[..3]).unwrap();
        buf[3] = 0;
        u32::from_le_bytes(buf)
    }

    /// Reads a 32-bit floating point number (little-endian).
    pub fn read_f32(&mut self) -> f32 {
        let mut buf = [0u8; 4];
        self.inner.read_exact(&mut buf).unwrap();
        f32::from_le_bytes(buf)
    }

    /// Reads an unsigned 32-bit integer (little-endian).
    pub fn read_u32(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        self.inner.read_exact(&mut buf).unwrap();
        u32::from_le_bytes(buf)
    }

    /// Reads a signed 32-bit integer (little-endian).
    pub fn read_i32(&mut self) -> i32 {
        self.read_u32() as i32
    }

    /// Reads an unsigned 64-bit integer (little-endian).
    pub fn read_u64(&mut self) -> u64 {
        let mut buf = [0u8; 8];
        self.inner.read_exact(&mut buf).unwrap();
        u64::from_le_bytes(buf)
    }

    /// Reads a signed 64-bit integer (little-endian).
    pub fn read_i64(&mut self) -> i64 {
        self.read_u64() as i64
    }

    /// Reads `count` bytes into a Vec.
    pub fn read_bytes(&mut self, count: usize) -> Vec<u8> {
        let mut buf = vec![0u8; count];
        self.inner.read_exact(&mut buf).unwrap();
        buf
    }

    /// Reads `count` u16 values.
    pub fn read_u16s(&mut self, count: usize) -> Vec<u16> {
        (0..count).map(|_| self.read_u16()).collect()
    }

    /// Reads `count` i16 values.
    pub fn read_i16s(&mut self, count: usize) -> Vec<i16> {
        (0..count).map(|_| self.read_i16()).collect()
    }

    /// Reads `count` f32 values.
    pub fn read_f32s(&mut self, count: usize) -> Vec<f32> {
        (0..count).map(|_| self.read_f32()).collect()
    }

    /// Reads `count` u32 values.
    pub fn read_u32s(&mut self, count: usize) -> Vec<u32> {
        (0..count).map(|_| self.read_u32()).collect()
    }

    /// Reads `count` i32 values.
    pub fn read_i32s(&mut self, count: usize) -> Vec<i32> {
        (0..count).map(|_| self.read_i32()).collect()
    }

    /// Reads `count` u64 values.
    pub fn read_u64s(&mut self, count: usize) -> Vec<u64> {
        (0..count).map(|_| self.read_u64()).collect()
    }

    /// Reads `count` i64 values.
    pub fn read_i64s(&mut self, count: usize) -> Vec<i64> {
        (0..count).map(|_| self.read_i64()).collect()
    }

    /// Reads `count` f32 values as an array.
    pub fn read_f32_array(&mut self, count: usize) -> Vec<f32> {
        self.read_f32s(count)
    }

    /// Reads `count` u32 values as an array.
    pub fn read_u32_array(&mut self, count: usize) -> Vec<u32> {
        self.read_u32s(count)
    }

    /// Reads `count` i32 values as an array.
    pub fn read_i32_array(&mut self, count: usize) -> Vec<i32> {
        self.read_i32s(count)
    }

    /// Reads `count` bool values.
    pub fn read_bools(&mut self, count: usize) -> Vec<bool> {
        (0..count).map(|_| self.read_bool()).collect()
    }

    /// Reads a null-terminated UTF-8 string.
    pub fn read_string(&mut self) -> String {
        let mut bytes = Vec::new();
        loop {
            let byte = self.read_u8();
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Reads a fixed-length UTF-8 string of `len` characters, stripping nulls.
    pub fn read_fixed_string(&mut self, len: usize) -> String {
        let bytes = self.read_bytes(len);
        String::from_utf8_lossy(&bytes)
            .trim_matches('\0')
            .to_string()
    }

    /// Reads a magic string (4 ASCII characters).
    pub fn read_magic(&mut self) -> String {
        let bytes = self.read_bytes(4);
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Aligns the stream position to the next `alignment` boundary.
    pub fn align(&mut self, alignment: u64) {
        let pos = self.position();
        let remainder = pos % alignment;
        if remainder != 0 {
            let skip = alignment - remainder;
            self.inner.seek(SeekFrom::Current(skip as i64)).unwrap();
        }
    }

    /// Reads a block of `size` bytes, seeking to the block offset first.
    /// Returns the byte slice and restores position.
    pub fn read_block(&mut self, offset: u64, size: usize) -> Vec<u8> {
        let current_pos = self.position();
        self.seek(offset);
        let data = self.read_bytes(size);
        self.seek(current_pos);
        data
    }

    /// Reads boolean bits from i64 bitflags.
    pub fn read_bool_bits(&mut self, count: usize) -> Vec<bool> {
        let num_i64 = count.div_ceil(64);
        let bit_flags: Vec<i64> = (0..num_i64).map(|_| self.read_i64()).collect();
        let mut booleans = Vec::with_capacity(count);
        let mut idx = 0;
        for i in 0..count {
            if i != 0 && i % 64 == 0 {
                idx += 1;
            }
            booleans.push((bit_flags[idx] & (1i64 << i)) != 0);
        }
        booleans
    }

    /// Reads a string at the given offset and returns it, restoring position.
    pub fn read_string_at_offset(&mut self, offset: u64) -> String {
        let pos = self.position();
        self.seek(offset);
        let _size = self.read_u16(); // string size prefix
        let value = self.read_string();
        self.seek(pos);
        value
    }

    /// Reads multiple string offsets.
    pub fn read_string_offsets(&mut self, count: usize) -> Vec<String> {
        let mut strings = Vec::with_capacity(count);
        for _ in 0..count {
            let offset = self.read_u64();
            strings.push(self.read_string_at_offset(offset));
        }
        strings
    }

    /// Reads a half-precision float and returns it as f32.
    pub fn read_half_f32(&mut self) -> f32 {
        let bits = self.read_u16() as u32;
        let sign = (bits >> 15) as i32;
        let exp = ((bits >> 10) & 0x1F) as i32;
        let frac = (bits & 0x3FF) as f32;

        if exp == 0 {
            if sign == 0 {
                frac * 0.000_061_035_156
            } else {
                -(frac * 0.000_061_035_156)
            }
        } else {
            let e = exp - 15;
            let sign_factor = if sign == 0 { 1.0 } else { -1.0 };
            sign_factor * (frac + 1024.0) * 2.0f32.powi(e - 10)
        }
    }

    /// Reads a half-precision float (as u16) and converts to f32.
    pub fn read_half(&mut self) -> f32 {
        self.read_half_f32()
    }
}

/// Seek alignment helper
pub trait SeekExt: Seek {
    fn align_to(&mut self, alignment: u64) -> std::io::Result<u64> {
        let pos = self.stream_position()?;
        let aligned = pos.div_ceil(alignment) * alignment;
        if aligned != pos {
            self.seek(SeekFrom::Start(aligned))?;
        }
        Ok(aligned)
    }
}

impl<S: Seek> SeekExt for S {}
