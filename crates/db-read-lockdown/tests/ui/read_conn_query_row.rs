fn main() {
    fn proves(c: &db_read::ReadConn<'_>) -> Result<i64, db_read::ReadError> {
        c.query_row("SELECT 1", [], |row| row.get(0))
    }
    let _ = proves;
}
