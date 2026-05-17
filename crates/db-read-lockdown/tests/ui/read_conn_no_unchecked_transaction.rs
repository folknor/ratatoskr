fn main() {
    fn proves(c: &db_read::ReadConn<'_>) {
        let _ = c.unchecked_transaction();
    }
    let _ = proves;
}
