-- description: --rebuild-attachment-index flag is accepted on Service spawn without crashing boot
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 60s

-- Phase 8b (attachments roadmap): `--rebuild-attachment-index` is a
-- corruption-recovery primitive that walks every sealed pack's
-- frames + replays every tombstone log to repopulate
-- `attachment_blobs`. Rebuild *correctness* is covered by Rust unit
-- tests in `crates/stores/src/attachment_pack.rs` (see
-- `rebuild_index_repopulates_from_sealed_packs`,
-- `rebuild_index_is_idempotent`). This harness only asserts the CLI
-- flag is parsed and the boot path runs the rebuild without
-- crashing - the integration contract that the flag actually
-- reaches `PackStore::rebuild_index`.

local dir = harness.data_dir("sync_jmap_rebuild_attachment_index_flag")
local client, err = harness.spawn(dir, { "--rebuild-attachment-index" })
harness.assert(err == nil, "spawn with --rebuild-attachment-index failed: " .. tostring(err))

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed under --rebuild-attachment-index")
harness.assert(ready.ready, "boot.ready returned ready=false under --rebuild-attachment-index")

-- Boot completed cleanly with the flag set on a fresh data dir
-- (no sealed packs to walk -> packs_walked=0). The contract is "the
-- flag doesn't crash", not "something is rebuilt".

harness.write_summary({ correct = 1 })

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
