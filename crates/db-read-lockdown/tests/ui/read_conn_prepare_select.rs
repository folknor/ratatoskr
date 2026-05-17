fn main() {
    fn proves(c: &db_read::ReadConn<'_>) -> Result<(), db_read::ReadError> {
        let _stmt = c.prepare("SELECT 1")?;
        Ok(())
    }
    let _ = proves;
}
