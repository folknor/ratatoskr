use std::path::PathBuf;

#[test]
fn lockdown_trybuild_read_conn_no_execute() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/read_conn_no_execute.rs");
}

#[test]
fn lockdown_trybuild_read_conn_no_transaction() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/read_conn_no_transaction.rs");
}

#[test]
fn lockdown_trybuild_read_conn_no_unchecked_transaction() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/read_conn_no_unchecked_transaction.rs");
}

#[test]
fn lockdown_trybuild_read_statement_no_execute() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/read_statement_no_execute.rs");
}

#[test]
fn lockdown_trybuild_writer_types_do_not_resolve() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/writer_types_do_not_resolve.rs");
}

#[test]
fn lockdown_trybuild_read_conn_query_row() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/read_conn_query_row.rs");
}

#[test]
fn read_conn_query_row_rejects_mutating_sql() {
    let raw = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    raw.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER)", [])
        .expect("create table");
    raw.execute("INSERT INTO t (n) VALUES (1)", [])
        .expect("insert row");

    let read = db_read::ReadConn::from_raw(&raw);
    let err = read
        .query_row("UPDATE t SET n = 2 RETURNING n", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect_err("mutating query_row must fail");

    assert!(
        matches!(err, db_read::ReadError::NotReadOnly(sql) if sql == "UPDATE t SET n = 2 RETURNING n")
    );
}

#[test]
fn lockdown_trybuild_read_conn_prepare_select() {
    isolate_trybuild_cargo_home();
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/read_conn_prepare_select.rs");
}

fn isolate_trybuild_cargo_home() {
    let root = workspace_root();
    let isolated = root.join(".brokkr/trybuild-cargo-home");
    std::fs::create_dir_all(&isolated)
        .unwrap_or_else(|e| panic!("create {}: {e}", isolated.display()));

    let original = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".cargo"))
        });

    if let Some(original) = original {
        link_cargo_home_child(&original, &isolated, "registry");
        link_cargo_home_child(&original, &isolated, "git");
    }

    // trybuild shells out to `cargo metadata`, which reads the user's
    // global cargo target rustflags before trybuild can pass its own
    // sanitized `--config` values. Use a minimal cargo home that shares
    // the package cache but not config.toml.
    unsafe {
        std::env::set_var("CARGO_HOME", &isolated);
    }
}

fn link_cargo_home_child(original: &std::path::Path, isolated: &std::path::Path, name: &str) {
    let source = original.join(name);
    if !source.exists() {
        return;
    }
    let dest = isolated.join(name);
    if dest.exists() {
        return;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &dest)
        .unwrap_or_else(|e| panic!("symlink {} -> {}: {e}", dest.display(), source.display()));
    #[cfg(not(unix))]
    {
        let _ = (source, dest);
    }
}

#[test]
fn db_read_public_surface_does_not_reexport_rusqlite() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(root.join("crates/db-read/src/lib.rs"))
        .expect("read db-read lib");
    assert!(
        !lib.contains("pub use rusqlite"),
        "db-read must not publicly re-export rusqlite mutating types",
    );

    // Catch the indirect bypass: glob-re-exporting the writer crate's
    // `db` module pulls every public item from writer_db (including
    // `Connection`, `WriteConn`, `WriterPool`, `WriteTxn`, `Transaction`,
    // `Statement`, `CachedStatement`) into db-read's surface. The
    // grep-for-literal-tokens check above passes because the banned
    // strings never appear in db-read source, even though the writer
    // types become reachable through `db_read::db::...`.
    //
    // Allow narrowly-scoped, named re-exports from `writer_db::db` (read
    // queries, types, utility modules); forbid blanket globs.
    for line in lib.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        let glob_writer_db = trimmed.contains("pub use writer_db::db::*")
            || trimmed.contains("pub use writer_db::*");
        assert!(
            !glob_writer_db,
            "db-read must not glob-re-export from writer_db; it leaks mutating \
             types (Connection, WriteConn, WriterPool, Transaction, ...) through \
             db_read::db::*. Enumerate the read-safe items explicitly. \
             Offending line: {trimmed}",
        );

        let whole_writer_query_module = matches!(
            trimmed,
            "queries," | "queries_extra," | "pending_ops," | "migrations," | "action_journal,"
        );
        assert!(
            !whole_writer_query_module,
            "db-read must not re-export whole writer-db modules. Re-export the \
             read-safe functions/types explicitly instead. Offending line: {trimmed}",
        );
    }
}

#[test]
fn db_read_raw_rusqlite_access_is_quarantined() {
    let root = workspace_root();
    let src = root.join("crates/db-read/src");
    let banned = [
        "rusqlite::Connection",
        "rusqlite::Transaction",
        "rusqlite::CachedStatement",
        ".execute(",
        ".execute_batch(",
        "unchecked_transaction",
        ".transaction(",
        "pragma_update",
    ];

    for entry in std::fs::read_dir(&src).expect("read db-read src") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for needle in banned {
            assert!(
                !raw.contains(needle),
                "{} contains banned raw-rusqlite/read-write escape pattern `{needle}`",
                path.display(),
            );
        }
    }
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf()
}
