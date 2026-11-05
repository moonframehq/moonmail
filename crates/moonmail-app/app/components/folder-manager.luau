local function build(props)
	local state, mail = props.state, props.mail
	local account = state.folder_account or mail.account or (if mail.demo then "you@example.com" else "local")
	local accounts, known = {}, {}
	for _, folder in mail.mailboxes or {} do
		if not known[folder.account] then known[folder.account] = true; table.insert(accounts, folder.account) end
	end
	for _, value in { account, mail.account or "local" } do
		if not known[value] then known[value] = true; table.insert(accounts, value) end
	end
	table.sort(accounts)
	local choices = {}
	for _, value in accounts do table.insert(choices, ui.button { label = value, small = true, msg = { folder_account = value }, variant = if value == account then "primary" else "ghost" }) end
	local items = {
		ui.text { "Manage folders", class = "title" },
		ui.text { "Remote changes are queued for Sync. Local/demo edits apply immediately. Rename includes descendants; delete child folders first.", tone = "muted" },
		ui.row { ui.button { label = "Back to mail", small = true, msg = "mail" }, ui.button { label = "Mail operations", small = true, msg = "operations" } },
		ui.column { gap = "small", table.unpack(choices) },
		ui.row { ui.input { id = "folder-new-name", value = state.folder_name or "", placeholder = "New folder path", grow = true, change = "folder-name" }, ui.button { label = "Create", small = true, msg = { action = "folder_create", account = account, name = state.folder_name or "" } } },
	}
	local edit = state.folder_edit
	if edit and edit.account == account then
		local deleting = edit.mode == "delete"
		table.insert(items, ui.text { (if deleting then "Delete " else "Rename ") .. edit.name, weight = "semibold" })
		if deleting then table.insert(items, ui.text { "Deletes this folder and its server contents. Downloaded original message bytes remain archived. Type the full folder name to proceed.", tone = "muted" }) end
		table.insert(items, ui.input { id = "folder-edit-" .. tostring(edit.id) .. edit.mode, value = state.folder_value or "", placeholder = if deleting then edit.name else "New full folder path", change = "folder-value" })
		table.insert(items, ui.row {
			ui.button { label = if deleting then "Delete folder & contents" else "Rename folder", small = true, disabled = deleting and state.folder_value ~= edit.name, msg = { action = if deleting then "folder_delete" else "folder_rename", id = edit.id, account = account, source = edit.name, epoch = edit.epoch, name = state.folder_value or "", confirm = state.folder_value or "" } },
			ui.button { label = "Cancel editing", small = true, msg = { folder_edit_cancel = true } },
		})
	end
	for _, folder in mail.mailboxes or {} do
		if folder.account ~= account then continue end
		table.insert(items, ui.row { padding = "small", shrink = false, align = "center",
			ui.column { grow = true, ui.text { folder.name, weight = "semibold", truncate = true }, ui.text { tostring(folder.total) .. " downloaded messages" .. (if not folder.selectable then " · structural folder" else ""), tone = "muted", size = "small" } },
			ui.button { label = "Rename…", small = true, disabled = string.lower(folder.name) == "inbox", msg = { folder_edit = folder, mode = "rename" } },
			ui.button { label = "Delete…", small = true, disabled = string.lower(folder.name) == "inbox", msg = { folder_edit = folder, mode = "delete" } },
		})
	end
	return ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
end

return component { title = "Folder manager", example = { state = {}, mail = {} }, build = build }
