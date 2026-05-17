pub fn user_visible_label_exists_fragment(
    account_column: &str,
    thread_column: &str,
    label_expr: &str,
) -> String {
    format!(
        "((EXISTS (SELECT 1 FROM thread_labels tl \
              WHERE tl.account_id = {account_column} \
                AND tl.thread_id = {thread_column} \
                AND tl.label_id = {label_expr} \
                AND NOT EXISTS (SELECT 1 FROM pending_thread_label_intents pli_rm \
                  WHERE pli_rm.account_id = tl.account_id \
                    AND pli_rm.thread_id = tl.thread_id \
                    AND pli_rm.label_id = tl.label_id \
                    AND pli_rm.op = 'Remove'))) \
          OR EXISTS (SELECT 1 FROM pending_thread_label_intents pli_add \
              WHERE pli_add.account_id = {account_column} \
                AND pli_add.thread_id = {thread_column} \
                AND pli_add.label_id = {label_expr} \
                AND pli_add.op = 'Add'))"
    )
}

pub fn user_visible_label_group_rendered_fragment(
    account_column: &str,
    thread_column: &str,
    group_predicate: &str,
) -> String {
    let visible_member = user_visible_label_exists_fragment(
        account_column,
        thread_column,
        "lgm.label_id",
    );
    format!(
        "EXISTS (SELECT 1 FROM label_group_members lgm \
           JOIN label_groups lg ON lg.id = lgm.group_id \
           WHERE lgm.account_id = {account_column} \
             AND {group_predicate} \
             AND {visible_member})"
    )
}
