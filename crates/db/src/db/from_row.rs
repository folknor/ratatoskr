use rusqlite::Connection;

/// Trait for types that can be constructed from a `rusqlite::Row`.
pub trait FromRow: Sized {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self>;
}

/// Generate a `FromRow` implementation for a struct.
///
/// Each field uses one of these forms (separated by commas):
///
/// - `field_name` - reads column `"field_name"`
/// - `field_name as "col"` - reads column `"col"` into `field_name`
/// - `bool field_name` - reads column `"field_name"` as `i64`, converts `!= 0`
/// - `bool field_name as "col"` - reads column `"col"` as `i64`, converts `!= 0`
/// - `val field_name = expr` - uses the literal expression
///
/// # Example
///
/// ```ignore
/// impl_from_row!(DbLabel {
///     id,
///     account_id,
///     label_type as "type",
///     bool visible,
///     sort_order,
/// });
/// ```
#[macro_export]
macro_rules! impl_from_row {
    ($struct_name:ident { $($input:tt)* }) => {
        impl $crate::db::from_row::FromRow for $struct_name {
            fn from_row(
                __from_row_r: &::rusqlite::Row<'_>,
            ) -> ::rusqlite::Result<Self> {
                $crate::impl_from_row_munch! {
                    $struct_name; __from_row_r; [];
                    $($input)*
                }
            }
        }
    };
}

/// TT-muncher implementation (not for direct use).
#[doc(hidden)]
#[macro_export]
macro_rules! impl_from_row_munch {
    // ── bool with rename: `bool field as "col",` ────────────
    ($sn:ident; $r:ident; [$($out:tt)*];
     bool $field:ident as $col:literal, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : ($r.get::<_, i64>($col)? != 0),];
            $($rest)*
        }
    };

    // ── bool default: `bool field,` ─────────────────────────
    ($sn:ident; $r:ident; [$($out:tt)*];
     bool $field:ident, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : ($r.get::<_, i64>(stringify!($field))? != 0),];
            $($rest)*
        }
    };

    // ── optbool with rename: `optbool field as "col",` ───────
    // Reads an `Option<i64>` column and maps to `Option<bool>`.
    ($sn:ident; $r:ident; [$($out:tt)*];
     optbool $field:ident as $col:literal, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : ($r.get::<_, Option<i64>>($col)?.map(|v| v != 0)),];
            $($rest)*
        }
    };

    // ── optbool default: `optbool field,` ────────────────────
    // Reads an `Option<i64>` column and maps to `Option<bool>`.
    ($sn:ident; $r:ident; [$($out:tt)*];
     optbool $field:ident, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : ($r.get::<_, Option<i64>>(stringify!($field))?.map(|v| v != 0)),];
            $($rest)*
        }
    };

    // ── value expression: `val field = expr,` ───────────────
    ($sn:ident; $r:ident; [$($out:tt)*];
     val $field:ident = $expr:expr, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : $expr,];
            $($rest)*
        }
    };

    // ── renamed column: `field as "col",` ───────────────────
    ($sn:ident; $r:ident; [$($out:tt)*];
     $field:ident as $col:literal, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : $r.get($col)?,];
            $($rest)*
        }
    };

    // ── plain field: `field,` ───────────────────────────────
    ($sn:ident; $r:ident; [$($out:tt)*];
     $field:ident, $($rest:tt)*
    ) => {
        $crate::impl_from_row_munch! { $sn; $r;
            [$($out)* $field : $r.get(stringify!($field))?,];
            $($rest)*
        }
    };

    // ── Base case - emit the struct literal ─────────────────
    ($sn:ident; $r:ident; [$($out:tt)*]; ) => {
        Ok($sn {
            $($out)*
        })
    };
}

/// Execute a query and map all rows to `T` using `FromRow`.
pub fn query_as<T: FromRow>(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<Vec<T>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, T::from_row)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Execute a query and map the first row, returning `None` if no rows match.
pub fn query_one<T: FromRow>(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<Option<T>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params, T::from_row)
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(row) => Ok(Some(row.map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}
