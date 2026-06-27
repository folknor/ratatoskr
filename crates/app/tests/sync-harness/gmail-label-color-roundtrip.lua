-- description: Gmail user-label server color survives attach through the bifrost containers_list seam
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s
--
-- B6a color-obstacle gate. The list sync moved off the legacy
-- sync_gmail_label_folder_map onto bifrost's containers_list
-- (bifrost::containers::sync_containers). The legacy pass carried a Gmail
-- user label's color.background_color / text_color into the labels table's
-- server_color_bg / server_color_fg; the rewrite must preserve that through
-- Container.style. This drives a colored label create via the harness-only
-- test.container_crud trigger, then a follow-up resync, and asserts the
-- label's server color round-tripped through the new seam end-to-end.

local function label_by_name(labels, name)
    for _, label in ipairs(labels) do
        if label.name == name then
            return label
        end
    end
    return nil
end

local dir = harness.data_dir("sync_gmail_label_color_roundtrip")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-color@example.test",
    display_name = "Sync Gmail Color",
    account_name = "Sync Gmail Color",
    provider = "gmail_api",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- Create a colored Gmail user label via the engine.
local created, created_err = client:request("TestContainerCrud", {
    account_id = account.account_id,
    op = "label_create",
    name = "Roundtrip",
    color_bg = "#16a766",
    color_fg = "#ffffff",
})
if created_err ~= nil then
    harness.assert(false, "TestContainerCrud label_create failed: " .. tostring(created_err.detail))
end
harness.assert(created.ok, "label_create outcome not Success: " .. tostring(created.outcome))

-- Round-trip: resync drives sync_containers, which must carry the server
-- color from Container.style back into the labels table.
local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil, "post-create resync failed")
harness.assert_eq(resync.result, "completed", resync.error or "post-create resync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
})
harness.assert(state_err == nil, "TestQueryDbState failed")

local label = label_by_name(state.labels, "Roundtrip")
harness.assert(label ~= nil, "created label missing after resync")
harness.assert(
    label.server_color_bg == "#16a766",
    "label server_color_bg did not round-trip through sync_containers: " .. tostring(label.server_color_bg)
)
harness.assert(
    label.server_color_fg == "#ffffff",
    "label server_color_fg did not round-trip through sync_containers: " .. tostring(label.server_color_fg)
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
