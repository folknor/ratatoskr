#![allow(clippy::unwrap_used)]

use db_read::blob_hash::BlobHash;
use rusqlite::Connection;

#[test]
fn hash_is_deterministic() {
    let a = BlobHash::hash(b"hello world");
    let b = BlobHash::hash(b"hello world");
    assert_eq!(a, b);
    assert_eq!(a.to_hex(), b.to_hex());
}

#[test]
fn hash_differs_on_different_input() {
    let a = BlobHash::hash(b"hello");
    let b = BlobHash::hash(b"world");
    assert_ne!(a, b);
}

#[test]
fn hex_round_trip() {
    let h = BlobHash::hash(b"round trip");
    let s = h.to_hex();
    assert_eq!(s.len(), 64);
    assert!(s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    let h2 = BlobHash::from_hex(&s).expect("parse");
    assert_eq!(h, h2);
}

#[test]
fn from_hex_rejects_bad_input() {
    assert!(BlobHash::from_hex("").is_err());
    assert!(BlobHash::from_hex("abc").is_err());
    assert!(BlobHash::from_hex(&"z".repeat(64)).is_err());
    assert!(BlobHash::from_hex(&"a".repeat(63)).is_err());
    assert!(BlobHash::from_hex(&"a".repeat(65)).is_err());
}

#[test]
fn from_slice_rejects_wrong_len() {
    assert!(BlobHash::from_slice(&[0u8; 31]).is_err());
    assert!(BlobHash::from_slice(&[0u8; 33]).is_err());
    assert!(BlobHash::from_slice(&[0u8; 32]).is_ok());
}

#[test]
fn sql_round_trip() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE t (h BLOB(32) NOT NULL)", []).unwrap();
    let h = BlobHash::hash(b"sql round trip");
    conn.execute("INSERT INTO t (h) VALUES (?1)", [&h]).unwrap();
    let got: BlobHash = conn
        .query_row("SELECT h FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(h, got);
}

#[test]
fn sql_rejects_wrong_blob_len() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE t (h BLOB NOT NULL)", []).unwrap();
    conn.execute("INSERT INTO t (h) VALUES (?1)", [&[0u8; 31][..]]).unwrap();
    let err: Result<BlobHash, _> = conn.query_row("SELECT h FROM t", [], |row| row.get(0));
    assert!(err.is_err());
}

#[test]
fn serde_round_trip() {
    let h = BlobHash::hash(b"serde");
    let json = serde_json::to_string(&h).unwrap();
    let expected = format!("\"{}\"", h.to_hex());
    assert_eq!(json, expected);
    let h2: BlobHash = serde_json::from_str(&json).unwrap();
    assert_eq!(h, h2);
}
