-- description: Graph master category list imports as cat:<displayName> tag labels with preset colours
-- @covers: architecture.folder_vs_label_semantics_are_explicit
-- @covers: glossary.folders_labels.label_rows_are_tags
-- @covers: glossary.folders_labels.storage_splits_folders_labels_and_groups
-- @covers: glossary.folders_labels.non_system_ids_keep_provider_prefixes
-- @covers: glossary.folders_labels.provider_terms_translate_to_folder_label_semantics
-- expected: pass
-- fixture: graph-categories-small.toml
-- protocol: graph
-- ceiling: 120s

local function label_by_id(labels, id)
    for _, label in ipairs(labels) do
        if label.id == id then
            return label
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_graph_master_category_label_sync")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-graph-categories@example.test",
    display_name = "Sync Graph Categories",
    account_name = "Sync Graph Categories",
    provider = "graph",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local result, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(result.result, "completed", result.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
})
harness.assert(state_err == nil, "TestQueryDbState failed")

-- Filter out folder-kind labels (Inbox etc) and the local "Harness"
-- label inserted by TestSeedAccount; assert only on cat-prefixed
-- tag-kind rows that graph_label_sync wrote.
local cat_labels = {}
for _, label in ipairs(state.labels) do
    if string.sub(label.id, 1, 4) == "cat:" then
        cat_labels[#cat_labels + 1] = label
    end
end
harness.assert_eq(#cat_labels, 4, "graph master-category count")

local work = label_by_id(cat_labels, "cat:Work")
harness.assert(work ~= nil, "missing cat:Work")
harness.assert_eq(work.account_id, account.account_id, "Work account_id")
harness.assert_eq(work.name, "Work", "Work name")
-- Post labels-unification split: rows live in `labels` (tag-only) and
-- carry server-supplied colours via `server_color_*`. Folder rows are
-- in `state.folders`; no synthesised label_kind / label_type fields.
-- label-colors preset0 = red (#e74c3c bg / #ffffff fg).
harness.assert_eq(work.server_color_bg, "#e74c3c", "Work server_color_bg from preset0")
harness.assert_eq(work.server_color_fg, "#ffffff", "Work server_color_fg from preset0")
harness.assert_eq(work.sort_order, 0, "Work sort_order (fixture index 0)")

local personal = label_by_id(cat_labels, "cat:Personal")
harness.assert(personal ~= nil, "missing cat:Personal")
-- preset2 = brown (#8b4513 / #ffffff).
harness.assert_eq(personal.server_color_bg, "#8b4513", "Personal server_color_bg from preset2")
harness.assert_eq(personal.server_color_fg, "#ffffff", "Personal server_color_fg from preset2")
harness.assert_eq(personal.sort_order, 1, "Personal sort_order (fixture index 1)")

local urgent = label_by_id(cat_labels, "cat:Urgent")
harness.assert(urgent ~= nil, "missing cat:Urgent")
-- preset15 = dark red (#8b0000 / #ffffff).
harness.assert_eq(urgent.server_color_bg, "#8b0000", "Urgent server_color_bg from preset15")
harness.assert_eq(urgent.server_color_fg, "#ffffff", "Urgent server_color_fg from preset15")
harness.assert_eq(urgent.sort_order, 2, "Urgent sort_order (fixture index 2)")

-- The fourth fixture row has no `color` field, which the Graph API
-- surfaces as preset "None". preset_to_hex skips that, so
-- server_color_bg and server_color_fg land NULL.
local uncategorised = label_by_id(cat_labels, "cat:Uncategorised")
harness.assert(uncategorised ~= nil, "missing cat:Uncategorised")
harness.assert(
    uncategorised.server_color_bg == nil,
    "Uncategorised server_color_bg should be nil when category has no preset"
)
harness.assert(
    uncategorised.server_color_fg == nil,
    "Uncategorised server_color_fg should be nil when category has no preset"
)
harness.assert_eq(uncategorised.sort_order, 3, "Uncategorised sort_order")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local master_category_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/outlook/masterCategories"
)
harness.assert(
    master_category_requests >= 1,
    "graph sync did not call /me/outlook/masterCategories"
)

harness.write_summary({
    correct = 1,
    category_label_count = #cat_labels,
    master_category_requests = master_category_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
