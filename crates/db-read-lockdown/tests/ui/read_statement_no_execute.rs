fn main() {
    fn proves(stmt: &mut db_read::ReadStatement<'_>) {
        let _ = stmt.execute([]);
    }
    let _ = proves;
}
