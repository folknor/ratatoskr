fn main() {
    fn proves(c: &db_read::ReadConn<'_>) {
        let _ = c.transaction();
    }
    let _ = proves;
}
