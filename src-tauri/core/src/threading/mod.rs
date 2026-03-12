use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ThreadableMessage {
    pub id: String,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub subject: Option<String>,
    pub date: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadGroup {
    pub thread_id: String,
    pub message_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Container (internal)
// ---------------------------------------------------------------------------

struct Container {
    message_id: String,
    message: Option<ThreadableMessage>,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct Arena {
    containers: Vec<Container>,
    id_table: HashMap<String, usize>,
}

impl Arena {
    fn new() -> Self {
        Self {
            containers: Vec::new(),
            id_table: HashMap::new(),
        }
    }

    fn get_or_create(&mut self, message_id: &str) -> usize {
        if let Some(&idx) = self.id_table.get(message_id) {
            return idx;
        }
        let idx = self.containers.len();
        self.containers.push(Container {
            message_id: message_id.to_string(),
            message: None,
            parent: None,
            children: Vec::new(),
        });
        self.id_table.insert(message_id.to_string(), idx);
        idx
    }

    /// Check whether `ancestor_idx` is an ancestor of `idx` (or the same).
    fn is_ancestor(&self, idx: usize, ancestor_idx: usize) -> bool {
        let mut current = Some(idx);
        while let Some(c) = current {
            if c == ancestor_idx {
                return true;
            }
            current = self.containers[c].parent;
        }
        false
    }

    fn unlink_from_parent(&mut self, child_idx: usize) {
        if let Some(parent_idx) = self.containers[child_idx].parent {
            self.containers[parent_idx]
                .children
                .retain(|&c| c != child_idx);
            self.containers[child_idx].parent = None;
        }
    }

    fn link_parent_child(&mut self, parent_idx: usize, child_idx: usize) {
        // Don't create a cycle
        if self.is_ancestor(parent_idx, child_idx) {
            return;
        }
        // Already correct
        if self.containers[child_idx].parent == Some(parent_idx) {
            return;
        }
        self.unlink_from_parent(child_idx);
        self.containers[child_idx].parent = Some(parent_idx);
        self.containers[parent_idx].children.push(child_idx);
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Strip Re:/Fwd:/Fw: prefixes and [list-prefix] tags. Case-insensitive.
pub fn normalize_subject(subject: Option<&str>) -> String {
    let Some(subject) = subject else {
        return String::new();
    };

    let mut s = subject.trim().to_string();
    let mut changed = true;

    while changed {
        changed = false;

        // Strip leading [list-prefix] tags
        if s.starts_with('[')
            && let Some(end) = s.find(']')
        {
            let rest = s[end + 1..].trim_start().to_string();
            s = rest;
            changed = true;
        }

        // Strip leading Re:/Fwd:/Fw: (case-insensitive)
        let lower = s.to_lowercase();
        for prefix in &["re:", "fwd:", "fw:"] {
            if lower.starts_with(prefix) {
                s = s[prefix.len()..].trim_start().to_string();
                changed = true;
                break;
            }
        }
    }

    s.trim().to_string()
}

/// Parse a References header into individual Message-IDs.
pub fn parse_references(references: Option<&str>) -> Vec<String> {
    let Some(refs) = references else {
        return Vec::new();
    };
    let refs = refs.trim();
    if refs.is_empty() {
        return Vec::new();
    }

    let mut ids = Vec::new();

    // Match angle-bracket-delimited Message-IDs: <something@host>
    let mut in_bracket = false;
    let mut current = String::new();

    for ch in refs.chars() {
        if ch == '<' {
            in_bracket = true;
            current.clear();
        } else if ch == '>' && in_bracket {
            in_bracket = false;
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                ids.push(trimmed);
            }
        } else if in_bracket {
            current.push(ch);
        }
    }

    // Fallback: if no angle-bracket IDs found, split on whitespace
    if ids.is_empty() {
        for token in refs.split_whitespace() {
            let cleaned = token.trim_start_matches('<').trim_end_matches('>').trim();
            if !cleaned.is_empty() {
                ids.push(cleaned.to_string());
            }
        }
    }

    ids
}

/// djb2 hash → hex string.
fn djb2_hash(s: &str) -> String {
    let mut hash: u32 = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u32::from(byte));
    }
    format!("{hash:x}")
}

/// Generate a deterministic thread ID from a root Message-ID.
pub fn generate_thread_id(root_message_id: &str) -> String {
    format!("imap-thread-{}", djb2_hash(root_message_id))
}

// ---------------------------------------------------------------------------
// Main algorithm
// ---------------------------------------------------------------------------

/// Group messages into threads using the JWZ algorithm.
pub fn build_threads(messages: &[ThreadableMessage]) -> Vec<ThreadGroup> {
    if messages.is_empty() {
        return Vec::new();
    }

    let mut arena = Arena::new();

    // Step 1-2: Build containers and link parent-child via references
    for msg in messages {
        let container_idx = arena.get_or_create(&msg.message_id);
        arena.containers[container_idx].message = Some(msg.clone());

        // Build the reference chain: References + In-Reply-To
        let mut ref_ids = parse_references(msg.references.as_deref());
        if let Some(in_reply_to) = &msg.in_reply_to {
            let irt_ids = parse_references(Some(in_reply_to));
            for id in irt_ids {
                if !ref_ids.contains(&id) {
                    ref_ids.push(id);
                }
            }
        }

        // Walk the reference chain, linking parent → child
        let mut prev_idx: Option<usize> = None;
        for ref_id in &ref_ids {
            let ref_idx = arena.get_or_create(ref_id);
            if let Some(prev) = prev_idx {
                // Only set parent if the ref container doesn't already have one
                if arena.containers[ref_idx].parent.is_none() {
                    arena.link_parent_child(prev, ref_idx);
                }
            }
            prev_idx = Some(ref_idx);
        }

        // Current message's container is a child of the last reference
        if let Some(prev) = prev_idx
            && prev != container_idx
        {
            arena.link_parent_child(prev, container_idx);
        }
    }

    // Step 3: Find the root set
    let roots: Vec<usize> = (0..arena.containers.len())
        .filter(|&i| arena.containers[i].parent.is_none())
        .collect();

    // Step 4: Group by subject — merge roots with same normalized subject
    let mut subject_map: HashMap<String, usize> = HashMap::new();
    for &root_idx in &roots {
        let subject = get_subject_for_container(&arena, root_idx);
        let normalized = normalize_subject(subject.as_deref());
        if normalized.is_empty() {
            continue;
        }

        if let Some(&existing_idx) = subject_map.get(&normalized) {
            let existing_has_msg = arena.containers[existing_idx].message.is_some();
            let root_has_msg = arena.containers[root_idx].message.is_some();

            if !existing_has_msg && root_has_msg {
                // existing is phantom, make root a child of existing
                arena.link_parent_child(existing_idx, root_idx);
            } else if root_has_msg.eq(&false) && existing_has_msg {
                // root is phantom, make existing a child of root
                arena.link_parent_child(root_idx, existing_idx);
                subject_map.insert(normalized, root_idx);
            } else {
                // Both have messages — merge newer under older
                let existing_date = arena.containers[existing_idx]
                    .message
                    .as_ref()
                    .map_or(0, |m| m.date);
                let root_date = arena.containers[root_idx]
                    .message
                    .as_ref()
                    .map_or(0, |m| m.date);
                if existing_date <= root_date {
                    arena.link_parent_child(existing_idx, root_idx);
                } else {
                    arena.link_parent_child(root_idx, existing_idx);
                    subject_map.insert(normalized, root_idx);
                }
            }
        } else {
            subject_map.insert(normalized, root_idx);
        }
    }

    // Recompute roots after subject merging
    let final_roots: Vec<usize> = (0..arena.containers.len())
        .filter(|&i| arena.containers[i].parent.is_none())
        .collect();

    // Step 5: Collect thread groups
    let mut visited = vec![false; arena.containers.len()];
    let mut thread_groups = Vec::new();

    for root_idx in final_roots {
        let mut messages_in_thread: Vec<&ThreadableMessage> = Vec::new();
        collect_messages(&arena, root_idx, &mut messages_in_thread, &mut visited);

        if messages_in_thread.is_empty() {
            continue;
        }

        // Sort by date ascending
        messages_in_thread.sort_by_key(|m| m.date);

        let root_message_id = &arena.containers[root_idx].message_id;

        thread_groups.push(ThreadGroup {
            thread_id: generate_thread_id(root_message_id),
            message_ids: messages_in_thread.iter().map(|m| m.id.clone()).collect(),
        });
    }

    thread_groups
}

fn collect_messages<'a>(
    arena: &'a Arena,
    idx: usize,
    result: &mut Vec<&'a ThreadableMessage>,
    visited: &mut Vec<bool>,
) {
    if visited[idx] {
        return;
    }
    visited[idx] = true;

    if let Some(ref msg) = arena.containers[idx].message {
        result.push(msg);
    }

    for &child_idx in &arena.containers[idx].children {
        collect_messages(arena, child_idx, result, visited);
    }
}

fn get_subject_for_container(arena: &Arena, idx: usize) -> Option<String> {
    if let Some(ref msg) = arena.containers[idx].message
        && let Some(ref subject) = msg.subject
    {
        return Some(subject.clone());
    }
    for &child_idx in &arena.containers[idx].children {
        if let Some(s) = get_subject_for_container(arena, child_idx) {
            return Some(s);
        }
    }
    None
}

/// Given existing threads and new messages, incrementally update thread assignments.
pub fn update_threads(
    existing_threads: &[ThreadGroup],
    new_messages: &[ThreadableMessage],
) -> Vec<ThreadGroup> {
    if new_messages.is_empty() {
        return Vec::new();
    }

    // Build lookup maps
    let mut thread_to_message_ids: HashMap<String, Vec<String>> = HashMap::new();
    let existing_thread_ids: std::collections::HashSet<String> = existing_threads
        .iter()
        .map(|t| t.thread_id.clone())
        .collect();

    for thread in existing_threads {
        thread_to_message_ids.insert(thread.thread_id.clone(), thread.message_ids.clone());
    }

    // Build threads from just the new messages
    let new_threads = build_threads(new_messages);

    let mut result = Vec::new();

    for new_thread in &new_threads {
        if existing_thread_ids.contains(&new_thread.thread_id) {
            // Merge: add new message IDs to existing thread
            if let Some(existing_msg_ids) = thread_to_message_ids.get(&new_thread.thread_id) {
                let mut merged: Vec<String> = existing_msg_ids.clone();
                for id in &new_thread.message_ids {
                    if !merged.contains(id) {
                        merged.push(id.clone());
                    }
                }
                result.push(ThreadGroup {
                    thread_id: new_thread.thread_id.clone(),
                    message_ids: merged,
                });
            } else {
                result.push(new_thread.clone());
            }
        } else {
            // Check if any new message references a root of an existing thread
            let mut merged_into_existing = false;

            'outer: for msg in new_messages {
                if !new_thread.message_ids.contains(&msg.id) {
                    continue;
                }

                let mut refs = parse_references(msg.references.as_deref());
                if let Some(ref irt) = msg.in_reply_to {
                    let irt_ids = parse_references(Some(irt));
                    for id in irt_ids {
                        if !refs.contains(&id) {
                            refs.push(id);
                        }
                    }
                }

                for ref_id in &refs {
                    let potential_thread_id = generate_thread_id(ref_id);
                    if existing_thread_ids.contains(&potential_thread_id)
                        && let Some(existing_msg_ids) =
                            thread_to_message_ids.get(&potential_thread_id)
                    {
                        let mut merged: Vec<String> = existing_msg_ids.clone();
                        for id in &new_thread.message_ids {
                            if !merged.contains(id) {
                                merged.push(id.clone());
                            }
                        }
                        result.push(ThreadGroup {
                            thread_id: potential_thread_id,
                            message_ids: merged,
                        });
                        merged_into_existing = true;
                        break 'outer;
                    }
                }
            }

            if !merged_into_existing {
                result.push(new_thread.clone());
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(
        id: &str,
        message_id: &str,
        in_reply_to: Option<&str>,
        references: Option<&str>,
        subject: Option<&str>,
        date: i64,
    ) -> ThreadableMessage {
        ThreadableMessage {
            id: id.to_string(),
            message_id: message_id.to_string(),
            in_reply_to: in_reply_to.map(String::from),
            references: references.map(String::from),
            subject: subject.map(String::from),
            date,
        }
    }

    // -- normalize_subject --

    #[test]
    fn normalize_subject_null() {
        assert_eq!(normalize_subject(None), "");
    }

    #[test]
    fn normalize_subject_empty() {
        assert_eq!(normalize_subject(Some("")), "");
        assert_eq!(normalize_subject(Some("   ")), "");
    }

    #[test]
    fn normalize_subject_clean() {
        assert_eq!(normalize_subject(Some("Hello World")), "Hello World");
    }

    #[test]
    fn normalize_subject_re() {
        assert_eq!(normalize_subject(Some("Re: Hello")), "Hello");
        assert_eq!(normalize_subject(Some("RE: Hello")), "Hello");
        assert_eq!(normalize_subject(Some("re: Hello")), "Hello");
    }

    #[test]
    fn normalize_subject_fwd() {
        assert_eq!(normalize_subject(Some("Fwd: Hello")), "Hello");
        assert_eq!(normalize_subject(Some("FWD: Hello")), "Hello");
        assert_eq!(normalize_subject(Some("Fw: Hello")), "Hello");
        assert_eq!(normalize_subject(Some("FW: Hello")), "Hello");
    }

    #[test]
    fn normalize_subject_nested() {
        assert_eq!(normalize_subject(Some("Re: Re: Fwd: Hello")), "Hello");
        assert_eq!(
            normalize_subject(Some("RE: Fw: re: FWD: Subject")),
            "Subject"
        );
    }

    #[test]
    fn normalize_subject_list_prefix() {
        assert_eq!(
            normalize_subject(Some("[node-dev] Some topic")),
            "Some topic"
        );
        assert_eq!(
            normalize_subject(Some("[node-dev] Re: Some topic")),
            "Some topic"
        );
        assert_eq!(
            normalize_subject(Some("Re: [node-dev] Some topic")),
            "Some topic"
        );
        assert_eq!(normalize_subject(Some("[PATCH] [v2] Fix bug")), "Fix bug");
    }

    #[test]
    fn normalize_subject_no_space_after_colon() {
        assert_eq!(normalize_subject(Some("Re:Hello")), "Hello");
    }

    // -- parse_references --

    #[test]
    fn parse_references_none() {
        assert!(parse_references(None).is_empty());
    }

    #[test]
    fn parse_references_empty() {
        assert!(parse_references(Some("")).is_empty());
        assert!(parse_references(Some("   ")).is_empty());
    }

    #[test]
    fn parse_references_single() {
        assert_eq!(
            parse_references(Some("<abc@host.com>")),
            vec!["abc@host.com"]
        );
    }

    #[test]
    fn parse_references_multiple() {
        assert_eq!(
            parse_references(Some("<id1@host> <id2@host>")),
            vec!["id1@host", "id2@host"]
        );
    }

    #[test]
    fn parse_references_various_separators() {
        assert_eq!(
            parse_references(Some("<id1@host>\n<id2@host>\t<id3@host>")),
            vec!["id1@host", "id2@host", "id3@host"]
        );
    }

    #[test]
    fn parse_references_bare_ids_fallback() {
        assert_eq!(
            parse_references(Some("id1@host id2@host")),
            vec!["id1@host", "id2@host"]
        );
    }

    #[test]
    fn parse_references_preserves_order() {
        assert_eq!(
            parse_references(Some("<first@host> <second@host> <third@host>")),
            vec!["first@host", "second@host", "third@host"]
        );
    }

    // -- generate_thread_id --

    #[test]
    fn generate_thread_id_format() {
        let id = generate_thread_id("abc@host.com");
        assert!(id.starts_with("imap-thread-"));
    }

    #[test]
    fn generate_thread_id_deterministic() {
        let id1 = generate_thread_id("test@example.com");
        let id2 = generate_thread_id("test@example.com");
        assert_eq!(id1, id2);
    }

    #[test]
    fn generate_thread_id_different_inputs() {
        let id1 = generate_thread_id("msg1@host.com");
        let id2 = generate_thread_id("msg2@host.com");
        assert_ne!(id1, id2);
    }

    // -- build_threads --

    #[test]
    fn build_threads_empty() {
        assert!(build_threads(&[]).is_empty());
    }

    #[test]
    fn build_threads_standalone_messages() {
        let messages = vec![
            msg("l1", "msg1@host", None, None, Some("First"), 1000),
            msg("l2", "msg2@host", None, None, Some("Second"), 2000),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 2);
    }

    #[test]
    fn build_threads_simple_reply_chain() {
        let messages = vec![
            msg("la", "a@host", None, None, Some("Topic"), 1000),
            msg(
                "lb",
                "b@host",
                Some("<a@host>"),
                Some("<a@host>"),
                Some("Re: Topic"),
                2000,
            ),
            msg(
                "lc",
                "c@host",
                Some("<b@host>"),
                Some("<a@host> <b@host>"),
                Some("Re: Re: Topic"),
                3000,
            ),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids.len(), 3);
        assert!(threads[0].message_ids.contains(&"la".to_string()));
        assert!(threads[0].message_ids.contains(&"lb".to_string()));
        assert!(threads[0].message_ids.contains(&"lc".to_string()));
    }

    #[test]
    fn build_threads_sorts_by_date() {
        let messages = vec![
            msg(
                "lc",
                "c@host",
                Some("<b@host>"),
                Some("<a@host> <b@host>"),
                Some("Re: Topic"),
                3000,
            ),
            msg("la", "a@host", None, None, Some("Topic"), 1000),
            msg(
                "lb",
                "b@host",
                Some("<a@host>"),
                Some("<a@host>"),
                Some("Re: Topic"),
                2000,
            ),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids, vec!["la", "lb", "lc"]);
    }

    #[test]
    fn build_threads_fork() {
        let messages = vec![
            msg("la", "a@host", None, None, Some("Topic"), 1000),
            msg(
                "lb",
                "b@host",
                Some("<a@host>"),
                Some("<a@host>"),
                Some("Re: Topic"),
                2000,
            ),
            msg(
                "lc",
                "c@host",
                Some("<a@host>"),
                Some("<a@host>"),
                Some("Re: Topic"),
                3000,
            ),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids.len(), 3);
    }

    #[test]
    fn build_threads_phantom_parents() {
        let messages = vec![
            msg(
                "lb",
                "b@host",
                Some("<missing@host>"),
                Some("<missing@host>"),
                Some("Re: Topic"),
                2000,
            ),
            msg(
                "lc",
                "c@host",
                Some("<missing@host>"),
                Some("<missing@host>"),
                Some("Re: Topic"),
                3000,
            ),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids.len(), 2);
    }

    #[test]
    fn build_threads_subject_merge() {
        let messages = vec![
            msg("la", "a@host", None, None, Some("Meeting notes"), 1000),
            msg("lb", "b@host", None, None, Some("Re: Meeting notes"), 2000),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids.len(), 2);
    }

    #[test]
    fn build_threads_different_subjects_not_merged() {
        let messages = vec![
            msg("la", "a@host", None, None, Some("Meeting notes"), 1000),
            msg("lb", "b@host", None, None, Some("Lunch plans"), 2000),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 2);
    }

    #[test]
    fn build_threads_deterministic_ids() {
        let messages = vec![
            msg("la", "root@host", None, None, Some("Topic"), 1000),
            msg(
                "lb",
                "reply@host",
                Some("<root@host>"),
                Some("<root@host>"),
                Some("Re: Topic"),
                2000,
            ),
        ];
        let t1 = build_threads(&messages);
        let t2 = build_threads(&messages);
        assert_eq!(t1[0].thread_id, t2[0].thread_id);
        assert_eq!(t1[0].thread_id, generate_thread_id("root@host"));
    }

    #[test]
    fn build_threads_delta_sync_same_id() {
        // Full conversation
        let all = vec![
            msg("la", "original@host", None, None, Some("Hello"), 1000),
            msg(
                "lb",
                "reply@host",
                Some("<original@host>"),
                Some("<original@host>"),
                Some("Re: Hello"),
                2000,
            ),
        ];
        let initial = build_threads(&all);

        // Delta: only the reply
        let delta = vec![msg(
            "lb",
            "reply@host",
            Some("<original@host>"),
            Some("<original@host>"),
            Some("Re: Hello"),
            2000,
        )];
        let delta_threads = build_threads(&delta);

        assert_eq!(initial[0].thread_id, delta_threads[0].thread_id);
        assert_eq!(
            delta_threads[0].thread_id,
            generate_thread_id("original@host")
        );
    }

    #[test]
    fn build_threads_deep_reply_delta_sync() {
        let delta = vec![msg(
            "lc",
            "c@host",
            Some("<b@host>"),
            Some("<a@host> <b@host>"),
            Some("Re: Topic"),
            3000,
        )];
        let threads = build_threads(&delta);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, generate_thread_id("a@host"));
    }

    #[test]
    fn build_threads_null_subjects() {
        let messages = vec![
            msg("la", "a@host", None, None, None, 1000),
            msg(
                "lb",
                "b@host",
                Some("<a@host>"),
                Some("<a@host>"),
                None,
                2000,
            ),
        ];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids.len(), 2);
    }

    #[test]
    fn build_threads_single_message() {
        let messages = vec![msg("l1", "only@host", None, None, Some("Solo"), 1000)];
        let threads = build_threads(&messages);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_ids, vec!["l1"]);
    }

    // -- update_threads --

    #[test]
    fn update_threads_no_new_messages() {
        let existing = vec![ThreadGroup {
            thread_id: "imap-thread-abc".to_string(),
            message_ids: vec!["l1".to_string()],
        }];
        assert!(update_threads(&existing, &[]).is_empty());
    }

    #[test]
    fn update_threads_new_standalone() {
        let existing = vec![ThreadGroup {
            thread_id: "imap-thread-abc".to_string(),
            message_ids: vec!["l1".to_string()],
        }];
        let new_msgs = vec![msg("l2", "new@host", None, None, Some("New topic"), 2000)];
        let result = update_threads(&existing, &new_msgs);
        assert_eq!(result.len(), 1);
        assert!(result[0].message_ids.contains(&"l2".to_string()));
        assert_ne!(result[0].thread_id, "imap-thread-abc");
    }

    #[test]
    fn update_threads_merge_into_existing() {
        let root_id = "root@host";
        let existing_thread_id = generate_thread_id(root_id);
        let existing = vec![ThreadGroup {
            thread_id: existing_thread_id.clone(),
            message_ids: vec!["l1".to_string()],
        }];
        let new_msgs = vec![msg(
            "l2",
            "reply@host",
            Some(&format!("<{root_id}>")),
            Some(&format!("<{root_id}>")),
            Some("Re: Topic"),
            2000,
        )];
        let result = update_threads(&existing, &new_msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].thread_id, existing_thread_id);
        assert!(result[0].message_ids.contains(&"l1".to_string()));
        assert!(result[0].message_ids.contains(&"l2".to_string()));
    }
}
