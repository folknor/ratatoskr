fn open_test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE messages (id INTEGER PRIMARY KEY, seen INTEGER DEFAULT 0);
         CREATE TABLE labels (id INTEGER PRIMARY KEY, name TEXT);
         INSERT INTO messages (id, seen) VALUES (1, 0);",
    )
    .expect("create schema");
    conn
}

#[test]
fn prepare_rejects_update_returning() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    let err = match read.prepare("UPDATE messages SET seen = 1 RETURNING id") {
        Ok(_) => panic!("UPDATE ... RETURNING should not be prepareable on ReadConn"),
        Err(err) => err,
    };
    assert!(matches!(err, db_read::ReadError::NotReadOnly(_)));
}

#[test]
fn prepare_rejects_insert_returning() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    let err = match read.prepare("INSERT INTO labels (name) VALUES ('x') RETURNING id") {
        Ok(_) => panic!("INSERT ... RETURNING should not be prepareable on ReadConn"),
        Err(err) => err,
    };
    assert!(matches!(err, db_read::ReadError::NotReadOnly(_)));
}

#[test]
fn prepare_accepts_plain_select() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    assert!(read.prepare("SELECT id FROM messages WHERE seen = ?1").is_ok());
}

#[test]
fn query_row_rejects_update_returning() {
    let conn = open_test_db();
    let read = db_read::ReadConn::from_raw(&conn);
    let err = read
        .query_row("UPDATE messages SET seen = 1 RETURNING id", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect_err("query_row with mutating SQL should fail readonly check");
    assert!(matches!(err, db_read::ReadError::NotReadOnly(_)));
}
