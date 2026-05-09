-- description: cold boot reads persisted bootstrap settings through Service IPC
-- ceiling: 45s

local dir = harness.data_dir("m6_cold_boot_bootstrap_snapshots")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "initial spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "initial boot.ready failed")
harness.assert(ready.ready, "initial boot.ready returned ready=false")

local defaults, defaults_err =
    client:request("internal.read_bootstrap_snapshots", {})
harness.assert(defaults_err == nil, "initial read_bootstrap_snapshots failed")
harness.assert_eq(defaults.ui.showSyncStatus, true, "default show sync status")
harness.assert_eq(
    defaults.settings.blockRemoteImages,
    true,
    "default block remote images"
)
harness.assert_eq(
    defaults.settings.phishingDetectionEnabled,
    true,
    "default phishing detection"
)

local _, set_err = client:request("settings.set", {
    values = {
        { type = "theme", value = "Dark" },
        { type = "reading_pane_position", value = "Bottom" },
        { type = "font_size", value = "Large" },
        { type = "show_sync_status", value = false },
        { type = "block_remote_images", value = false },
        { type = "phishing_detection_enabled", value = false },
        { type = "phishing_sensitivity", value = "low" },
    },
})
harness.assert(set_err == nil, "settings.set failed")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "initial shutdown failed")
harness.assert(shutdown_err == nil, "initial shutdown returned error")
client:drop()

local second, second_err = harness.spawn(dir)
harness.assert(second_err == nil, "second spawn failed")

local second_ready, second_ready_err = second:request("BootReady")
harness.assert(second_ready_err == nil, "second boot.ready failed")
harness.assert(second_ready.ready, "second boot.ready returned ready=false")

local snapshots, snapshots_err =
    second:request("internal.read_bootstrap_snapshots", {})
harness.assert(snapshots_err == nil, "read_bootstrap_snapshots failed")

harness.assert_eq(snapshots.ui.theme, "Dark", "theme")
harness.assert_eq(
    snapshots.ui.readingPanePosition,
    "Bottom",
    "reading pane position"
)
harness.assert_eq(snapshots.ui.fontSize, "Large", "font size")
harness.assert_eq(snapshots.ui.showSyncStatus, false, "show sync status")
harness.assert_eq(
    snapshots.settings.blockRemoteImages,
    false,
    "block remote images"
)
harness.assert_eq(
    snapshots.settings.phishingDetectionEnabled,
    false,
    "phishing detection"
)
harness.assert_eq(
    snapshots.settings.phishingSensitivity,
    "low",
    "phishing sensitivity"
)

local second_ok, second_shutdown_err = second:shutdown()
harness.assert(second_ok, "second shutdown failed")
harness.assert(second_shutdown_err == nil, "second shutdown returned error")
