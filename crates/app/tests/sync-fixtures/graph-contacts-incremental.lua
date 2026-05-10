-- Incremental Graph contact-sync scenario. Mirrors
-- jmap-incremental.lua's new/change/delete shape but for the Graph
-- /contactFolders/{id}/contacts/delta surface.

fixture({ name = "graph-contacts-incremental", state = "cinc-0" })
account({ id = "account-1", name = "test@example.com" })

mailbox({ id = "mbx-inbox", name = "Inbox", role = "inbox", sort_order = 0 })

contact_folder({ id = "cf-default", display_name = "Contacts", is_default = true })

contact({
    id = "contact-001",
    folder_id = "cf-default",
    display_name = "Alice Anderson",
    emails = { { name = "Alice", address = "alice@example.com" } },
})
contact({
    id = "contact-002",
    folder_id = "cf-default",
    display_name = "Bob Bell",
    emails = { "bob@example.com" },
})

-- Step 1: a new contact arrives.
change({
    id = "new",
    contact_create = {
        {
            id = "contact-003",
            folder_id = "cf-default",
            display_name = "Carol Carver",
            emails = { "carol@example.com" },
        },
    },
})

-- Step 2: a contact's display name + emails change.
change({
    id = "change",
    contact_update = {
        {
            id = "contact-002",
            display_name = "Robert Bell",
            emails = {
                "bob@example.com",
                { name = "Robert (work)", address = "robert@work.example" },
            },
        },
    },
})

-- Step 3: a contact gets removed.
change({
    id = "delete",
    contact_destroy = { "contact-001" },
})
