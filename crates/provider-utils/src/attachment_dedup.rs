use std::collections::HashMap;
use std::hash::Hash;

pub fn dedup_by_key<K, T, FKey, FMerge>(items: Vec<T>, key_for: FKey, mut merge: FMerge) -> Vec<T>
where
    K: Eq + Hash,
    FKey: Fn(&T) -> K,
    FMerge: FnMut(&mut T, T),
{
    let mut seen: HashMap<K, T> = HashMap::new();
    for item in items {
        let key = key_for(&item);
        if let Some(existing) = seen.get_mut(&key) {
            merge(existing, item);
        } else {
            seen.insert(key, item);
        }
    }
    seen.into_values().collect()
}

pub fn prefer_non_placeholder_filename(
    existing: &mut String,
    new: &str,
    existing_is_placeholder: bool,
    new_is_placeholder: bool,
) {
    if existing_is_placeholder && !new_is_placeholder {
        existing.clear();
        existing.push_str(new);
    }
}

pub fn prefer_missing_clone<T: Clone>(existing: &mut Option<T>, new: &Option<T>) {
    if existing.is_none() && new.is_some() {
        existing.clone_from(new);
    }
}

pub fn prefer_missing_take<T>(existing: &mut Option<T>, new: &mut Option<T>) {
    if existing.is_none() && new.is_some() {
        *existing = new.take();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        dedup_by_key, prefer_missing_clone, prefer_missing_take, prefer_non_placeholder_filename,
    };

    #[test]
    fn dedups_and_merges_by_key() {
        let merged = dedup_by_key(
            vec![
                ("a".to_string(), 1usize),
                ("b".to_string(), 2usize),
                ("a".to_string(), 3usize),
            ],
            |item| item.0.clone(),
            |existing, new| existing.1 += new.1,
        );

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|(key, value)| key == "a" && *value == 4));
    }

    #[test]
    fn prefers_non_placeholder_filename() {
        let mut filename = "inline".to_string();
        prefer_non_placeholder_filename(&mut filename, "photo.png", true, false);
        assert_eq!(filename, "photo.png");
    }

    #[test]
    fn prefers_missing_clone() {
        let mut existing = None;
        let new = Some("value".to_string());
        prefer_missing_clone(&mut existing, &new);
        assert_eq!(existing.as_deref(), Some("value"));
    }

    #[test]
    fn prefers_missing_take() {
        let mut existing = None;
        let mut new = Some(vec![1u8, 2, 3]);
        prefer_missing_take(&mut existing, &mut new);
        assert_eq!(existing.as_deref(), Some(&[1u8, 2, 3][..]));
        assert!(new.is_none());
    }
}
