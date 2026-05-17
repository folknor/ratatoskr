fn main() {
    let _ = std::any::type_name::<db_read::WriteConn<'_>>();
    let _ = std::any::type_name::<db_read::WriteTxn<'_>>();
    let _ = std::any::type_name::<db_read::WriterPool>();
}
