//! Content-addressed blob hash, used as the identity primitive for the
//! attachment subsystem (problem-statement.md). One algorithm (BLAKE3),
//! one in-memory type (`[u8; 32]`), one SQLite repr (`BLOB(32)`), one
//! wire repr (lowercase hex).

use std::fmt;

use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum BlobHashError {
    #[error("expected 32 bytes, got {0}")]
    WrongByteLen(usize),
    #[error("expected 64 hex chars, got {0}")]
    WrongHexLen(usize),
    #[error("non-hex character in input")]
    NotHex,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlobHash([u8; 32]);

impl BlobHash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, BlobHashError> {
        if bytes.len() != 32 {
            return Err(BlobHashError::WrongByteLen(bytes.len()));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(bytes);
        Ok(Self(buf))
    }

    pub fn from_hex(s: &str) -> Result<Self, BlobHashError> {
        if s.len() != 64 {
            return Err(BlobHashError::WrongHexLen(s.len()));
        }
        let mut buf = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
            let hi = hex_nibble(chunk[0])?;
            let lo = hex_nibble(chunk[1])?;
            buf[i] = (hi << 4) | lo;
        }
        Ok(Self(buf))
    }

    pub fn hash(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in &self.0 {
            s.push(nibble_hex(byte >> 4));
            s.push(nibble_hex(byte & 0x0f));
        }
        s
    }
}

fn hex_nibble(c: u8) -> Result<u8, BlobHashError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(BlobHashError::NotHex),
    }
}

fn nibble_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + n - 10) as char,
        _ => unreachable!(),
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobHash({})", self.to_hex())
    }
}

impl ToSql for BlobHash {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Blob(&self.0)))
    }
}

impl FromSql for BlobHash {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let bytes = value.as_blob()?;
        Self::from_slice(bytes).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

impl Serialize for BlobHash {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for BlobHash {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct HexVisitor;
        impl<'de> Visitor<'de> for HexVisitor {
            type Value = BlobHash;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("64 lowercase hex chars (BLAKE3 hash)")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<BlobHash, E> {
                BlobHash::from_hex(v).map_err(de::Error::custom)
            }
        }
        de.deserialize_str(HexVisitor)
    }
}
