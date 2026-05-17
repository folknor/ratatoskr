fn main() {
    fn proves(c: &db_read::ReadConn<'_>) {
        c.execute("UPDATE x SET y = 1", []);
    }
    let _ = proves;
}
