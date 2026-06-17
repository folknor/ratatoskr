use std::ops::Bound;

/// Parsed date boundary for `before:` and `after:` search operators.
///
/// Boundary semantics are intentionally centralized here: both SQL and Tantivy
/// emit exclusive bounds.
///
/// ```compile_fail
/// let _ = types::DateBound { timestamp: 1, direction: todo!() };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateBound {
    timestamp: i64,
    direction: BoundDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BoundDirection {
    Before,
    After,
}

impl DateBound {
    pub fn before(timestamp: i64) -> Self {
        Self {
            timestamp,
            direction: BoundDirection::Before,
        }
    }

    pub fn after(timestamp: i64) -> Self {
        Self {
            timestamp,
            direction: BoundDirection::After,
        }
    }

    pub fn timestamp(self) -> i64 {
        self.timestamp
    }

    pub fn to_sql_clause(self, column: &str, param_idx: usize) -> (String, i64) {
        let operator = match self.direction {
            BoundDirection::Before => "<",
            BoundDirection::After => ">",
        };
        (format!("{column} {operator} ?{param_idx}"), self.timestamp)
    }

    pub fn to_range_bound<T>(self, make_bound_value: impl FnOnce(i64) -> T) -> Bound<T> {
        Bound::Excluded(make_bound_value(self.timestamp))
    }
}

#[cfg(test)]
mod tests {
    use super::DateBound;
    use std::ops::Bound;

    #[test]
    fn sql_clause_encodes_direction() {
        assert_eq!(
            DateBound::before(10).to_sql_clause("m.date", 1),
            ("m.date < ?1".to_string(), 10)
        );
        assert_eq!(
            DateBound::after(20).to_sql_clause("m.date", 2),
            ("m.date > ?2".to_string(), 20)
        );
    }

    #[test]
    fn range_bound_is_exclusive() {
        assert_eq!(
            DateBound::before(10).to_range_bound(|ts| ts),
            Bound::Excluded(10)
        );
        assert_eq!(
            DateBound::after(20).to_range_bound(|ts| ts),
            Bound::Excluded(20)
        );
    }
}
