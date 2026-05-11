-- description: handshake against a fake-version Service surfaces VersionMismatch
-- ceiling: 60s

-- --test-fake-version forces the Service to announce a bogus protocol number
-- in its boot.ready response. The client must reject the handshake rather
-- than continue with a peer it can't speak to.

local dir = harness.data_dir("version_mismatch")
local client, err = harness.spawn(dir, { "--test-fake-version=999" })

harness.assert(client == nil, "spawn unexpectedly succeeded")
harness.assert(err ~= nil, "spawn returned no error")
harness.assert_eq(err.kind, "VersionMismatch", "error kind")
harness.assert_eq(err.ui, harness.protocol_version, "ui version")
harness.assert_eq(err.service, 999, "service version")
