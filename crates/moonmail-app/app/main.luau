local function action(label, name, extra)
	local msg = extra or {}
	msg.action = name
	return ui.button { label = label, small = true, msg = msg, to = "provider:mail" }
end

local ROW = 22
local activity_bar = use "activity-bar"
local mailbox_chooser = use "mailbox-chooser"
local folder_manager = use "folder-manager"
local tags = use "keyword-manager"
local entry_rules = use "entry-rules"
local search_partitions = use "search-partitions"
local DEFAULT_COLUMNS = { { field = "sender", width = 220, visible = true }, { field = "subject", width = 0, visible = true }, { field = "date", width = 156, visible = true } }
local COLUMN_LABELS = { sender = "From", subject = "Subject", date = "Date", recipients = "To" }

local function columns(mail, related)
	if related then return { { field = "subject", width = 0 }, { field = "date", width = 106 } } end
	local visible = {}
	for _, column in (mail.preferences or {}).columns or DEFAULT_COLUMNS do
		if column.visible then table.insert(visible, column) end
	end
	return visible
end

local function cell(text, width, heading, tone, last)
	return ui.column { width = width, height = if heading then 25 else ROW, gap = "none", grow = width == nil, shrink = width == nil,
		justify = "center", class = if heading then (if last then "mail-table-heading-cell mail-table-heading-last" else "mail-table-heading-cell") else "mail-table-cell",
		ui.text { text, truncate = true, tone = tone, weight = if heading then "semibold" else "normal" },
	}
end

local function heading(label, width, field, related, state, mail, last)
	local sorted = if related then mail.related_sort else mail.sort or "date"
	local ascending = if related then mail.related_ascending else mail.ascending
	return ui.row { width = width, height = 25, grow = width == nil, shrink = width == nil, align = "center", gap = "none",
		class = if last then "mail-table-heading-cell mail-table-heading-last" else "mail-table-heading-cell",
		msg = if related then { action = "related_sort", field = field, anchor = mail.anchor } else { action = "sort", field = field },
		ui.column { grow = true, ui.text { label, truncate = true, weight = "semibold" } },
		if sorted == field then ui.icon { name = if ascending then "chevron-up" else "chevron-down", size = "small" } else nil,
	}
end

local function table_header(related, state, mail)
	local headings = {}
	local layout = columns(mail, related)
	for index, column in layout do
		local width = if column.width == 0 then nil else column.width
		table.insert(headings, if column.field == "recipients" then cell("To", width, true, nil, index == #layout) else heading(COLUMN_LABELS[column.field], width, column.field, related, state, mail, index == #layout))
	end
	return ui.row { height = 25, shrink = false, gap = "none", class = "mail-table-heading",
		if not related then cell("✓", 24, true) else nil,
		if not related then cell("●", 24, true) else nil,
		if not related then cell("⚑", 24, true) else nil,
		table.unpack(headings),
	}
end

local function rows(messages, selected_id, related, key, threaded, layout)
	local items = messages or {}
	if #items == 0 then
		return ui.column { grow = true, padding = "large", justify = "center", align = "center",
			ui.text { if related then "No related messages" else "No messages", tone = "muted" },
		}
	end
	return ui.list {
		id = (if related then "related-" else "messages-") .. key,
		count = #items, row_height = ROW, grow = true,
		row = function(i)
			local mail = items[i]
			if mail.thread_pager then
				return ui.row { height = ROW, shrink = false, gap = "normal", align = "center", padding = "none", class = "mail-table-row",
					ui.column { width = 74 }, ui.text { "Replies", tone = "muted" },
					ui.row { height = ROW, msg = { action = "thread_first", id = mail.id, cursor = mail.thread_cursor }, to = "provider:mail", ui.text { "First", tone = "accent" } },
					if mail.thread_more then ui.row { height = ROW, msg = { action = "thread_next", id = mail.id, cursor = mail.thread_cursor }, to = "provider:mail", ui.text { "Next 100", tone = "accent" } } else ui.text { "End of thread", tone = "muted" },
				}
			end
			local cells = {}
			local disclosure = "subject"
			for _, column in layout do if column.field == "sender" then disclosure = "sender" end end
			for _, column in layout do
				local width = if column.width == 0 then nil else column.width
				local value = if column.field == "date" then string.sub(mail.date_label, 6, if related then 16 else 22) else mail[column.field] or ""
				if column.field == "subject" and threaded and (mail.thread_count or 1) > 1 then value ..= "  (" .. tostring(mail.thread_count) .. ")" end
				if not related and threaded and column.field == disclosure then
					table.insert(cells, ui.row { width = width, height = ROW, grow = width == nil, shrink = width == nil, align = "center", gap = "none",
						ui.row { width = 20 + (mail.depth or 0) * 14, shrink = false, justify = "end", align = "center",
							if (mail.thread_count or 1) > 1 then ui.row { width = 20, height = ROW, align = "center", justify = "center", msg = { action = "expand", id = mail.id }, ui.icon { name = if mail.expanded then "chevron-down" else "chevron-right", size = "small" } } else nil,
						}, cell(value, nil, false),
					})
				else table.insert(cells, cell(value, width, false)) end
			end
			return ui.row {
				height = ROW, shrink = false, gap = "none", padding = "none", align = "center",
				class = if i % 2 == 0 then "mail-table-row mail-table-row-even" else "mail-table-row",
				selected = selected_id == mail.id,
				msg = { action = if related then "related" else "select", id = mail.id },
				if not related then ui.row { width = 24, height = ROW, align = "center", justify = "center", msg = { action = "toggle_selection", id = mail.id }, tooltip = "Select this message for a bulk move (a thread row selects only its representative)", ui.text { if mail.checked then "☑" else "□", tone = if mail.checked then "accent" else "muted" } } else nil,
				if not related then cell(if mail.unread then "●" else "", 24, false, "accent") else nil,
				if not related then cell(if mail.starred then "⚑" else "", 24, false) else nil,
				table.unpack(cells),
			}
		end,
	}
end

local function tool(icon, tip, msg, to, disabled, on)
	return ui.row { width = 40, height = 40, shrink = false, align = "center", justify = "center", class = "mail-tool",
		state = if disabled then "disabled" elseif on then "on" else nil,
		tooltip = tip, msg = if disabled then nil else msg, to = if disabled then nil else to,
		ui.icon { name = icon, size = "headline" },
	}
end

local function mailbox_row(label, icon, count, selected, msg)
	return ui.row { height = 26, shrink = false, align = "center", class = "mailbox-row", selected = selected, msg = msg,
		ui.icon { name = icon, size = "small", tone = "accent" },
		ui.column { grow = true, ui.text { label, truncate = true } },
		if count and count > 0 then ui.text { tostring(count), class = "mailbox-count" } else nil,
	}
end

local folder_icons = { INBOX = "inbox", Inbox = "inbox", Sent = "send", ["Sent Mail"] = "send", Archive = "archive", Drafts = "file-text", Junk = "ban", Spam = "ban", Trash = "trash-2" }

local function folder_nodes(children, nodes, state, mail, depth)
	for _, node in nodes do
		local branch = #(node.children or {}) > 0
		local closed = state.closed[node.key] == true
		table.insert(children, ui.row { height = 26, shrink = false, align = "center", gap = "none", class = "mailbox-row", selected = node.id ~= nil and mail.mailbox == node.id and state.pane == "mail",
			msg = if node.id then { action = "folder", id = node.id } elseif branch then { folder_toggle = node.key } else nil,
			ui.row { width = depth * 14 + 14, shrink = false, justify = "end", align = "center",
				if branch then ui.row { width = 14, height = 26, align = "center", msg = { folder_toggle = node.key }, ui.icon { name = if closed then "chevron-right" else "chevron-down", size = "small" } } else nil,
			},
			ui.row { width = 22, shrink = false, align = "center", ui.icon { name = node.icon or folder_icons[node.name] or "folder", size = "small", tone = "accent" } },
			ui.column { grow = true, ui.text { if node.name == "INBOX" then "Inbox" else node.name, truncate = true } },
			if node.unread and node.unread > 0 then ui.text { tostring(node.unread), class = "mailbox-count" } else nil,
		})
		if branch and not closed then folder_nodes(children, node.children, state, mail, depth + 1) end
	end
end

local function header_field(label, value)
	return ui.row { height = 19, shrink = false, gap = "small",
		ui.row { width = 52, shrink = false, justify = "end", ui.text { label .. ":", tone = "muted" } },
		ui.column { grow = true, ui.text { value, truncate = true, copy = value } },
	}
end

local function file_size(bytes)
	return if bytes < 1024 then tostring(bytes) .. " B" elseif bytes < 1048576 then string.format("%.1f KiB", bytes / 1024) else string.format("%.1f MiB", bytes / 1048576)
end

local function identities_view(state, mail)
	local items = { ui.text { "Sending identities & signatures", class = "title" },
		ui.text { "Configure addresses authorized by your mail provider. Credentials stay with the account. Existing drafts retain their saved sender and signature text.", tone = "muted" },
		ui.text { "Account: " .. (mail.identity_account or ""), tone = "muted" },
		ui.row { ui.button { label = "New identity", small = true, disabled = not mail.identity_new_id or mail.identity_account == "", msg = { edit_identity = { id = mail.identity_new_id, account = mail.identity_account, name = "", address = "", reply_to = "", signature = "" } } }, ui.button { label = "Back to mail", small = true, msg = "mail" }, if mail.draft then ui.button { label = "Back to draft", small = true, msg = { action = "draft_open", id = mail.draft.id } } else nil },
	}
	for _, account in mail.accounts or {} do
		table.insert(items, ui.button { label = account, small = true, variant = if account == mail.identity_account then "primary" else "ghost", msg = { action = "identity_account", account = account } })
	end
	if mail.draft and mail.draft.account == mail.identity_account then
		table.insert(items, ui.button { label = "Use account address for this draft", small = true, msg = { action = "draft_identity", id = mail.draft.id, identity = "" } })
	end
	for _, identity in mail.identities or {} do
		table.insert(items, ui.row { shrink = false, align = "center", padding = "small",
			ui.column { grow = true, ui.text { identity.name .. " <" .. identity.address .. ">" .. (if identity.is_default then " · default" else ""), truncate = true }, ui.text { if identity.reply_to ~= "" then "Reply-To: " .. identity.reply_to else "Replies go to the sending address", tone = "muted" } },
			ui.button { label = "Edit", small = true, msg = { edit_identity = identity } },
			action("Make default", "identity_default", { account = identity.account, id = identity.id }),
			action("Remove", "identity_delete", { account = identity.account, id = identity.id }),
			if mail.draft and mail.draft.account == identity.account then ui.button { label = "Use for draft", small = true, msg = { action = "draft_identity", id = mail.draft.id, identity = identity.id } } else nil,
		})
	end
	local edit = state.identity_edit
	if edit then
		table.insert(items, ui.text { "Edit identity", weight = "bold" })
		for _, field in { { "Display name", "name" }, { "Sending address", "address" }, { "Reply-To (optional)", "reply_to" }, { "Signature", "signature" } } do
			table.insert(items, ui.text { field[1], tone = "muted" })
			table.insert(items, ui.input { id = "identity-" .. edit.id .. "-" .. field[2], value = edit[field[2]] or "", multiline = field[2] == "signature", rows = if field[2] == "signature" then 4 else nil, change = { identity_field = field[2] } })
		end
		table.insert(items, action("Save identity", "identity_save", { id = edit.id, account = edit.account, name = edit.name, address = edit.address, reply_to = edit.reply_to, signature = edit.signature }))
	end
	return ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
end

return mod {
	window = { size = { width = 1500, height = 940 }, titlebar = "hidden", traffic_lights = { x = 20, y = 20 } },
	about = { version = "0.1", description = "Local-first mail, with Correspondence." },
	menus = {
		{ title = "Mail", items = {
			{ label = "Settings…", msg = "open", to = "settings", shortcut = "secondary-," },
			{ label = "Compose", msg = "compose", shortcut = "secondary-n" },
			{ label = "Reply", msg = "reply" },
			{ label = "Reply All", msg = "reply_all" },
			{ label = "Forward", msg = "forward", shortcut = "secondary-shift-f" },
			{ label = "Forward text only", msg = "forward_text" },
			{ label = "Drafts", msg = "drafts" },
			{ label = "Identities & signatures", msg = "identities" },
			{ label = "Saved searches…", msg = "searches" },
			{ label = "Search", msg = "focus-search", shortcut = "secondary-f" },
			{ label = "Go to mailbox…", msg = "mailbox-chooser", shortcut = "secondary-l" },
			{ label = "Move to mailbox…", msg = "move-chooser", shortcut = "secondary-shift-m" },
			{ label = "Copy to mailbox…", msg = "copy-chooser", shortcut = "secondary-alt-m" },
			{ label = "Manage folders…", msg = "folders" },
			{ label = "Tags & keywords…", msg = "tags" },
			{ label = "Folder-entry rules…", msg = "rules" },
			{ label = "Export selected originals…", msg = "export-selected" },
			{ label = "Archive selected", msg = { action = "archive_messages" }, to = "provider:mail", shortcut = "secondary-shift-a" },
			{ label = "Mark unread", msg = { action = "unread" }, to = "provider:mail", shortcut = "secondary-shift-u" },
			{ label = "Sync", msg = { action = "sync" }, to = "provider:mail", shortcut = "secondary-r" },
			{ label = "Stop Sync", msg = { action = "cancel_sync" }, to = "provider:mail" },
			{ label = "Next Message", msg = { action = "move", value = "down" }, to = "provider:mail", shortcut = "secondary-down" },
			{ label = "Previous Message", msg = { action = "move", value = "up" }, to = "provider:mail", shortcut = "secondary-up" },
			{ label = "Next unread", msg = { action = "seek_unread", value = "down" }, to = "provider:mail", shortcut = "secondary-alt-down" },
			{ label = "Previous unread", msg = { action = "seek_unread", value = "up" }, to = "provider:mail", shortcut = "secondary-alt-up" },
			{ label = "Next thread", msg = { action = "seek_thread", value = "down" }, to = "provider:mail", shortcut = "secondary-shift-down" },
			{ label = "Previous thread", msg = { action = "seek_thread", value = "up" }, to = "provider:mail", shortcut = "secondary-shift-up" },
			{ label = "Mark entire thread read", msg = { action = "thread_read" }, to = "provider:mail" },
			{ label = "Mark entire thread unread", msg = { action = "thread_unread" }, to = "provider:mail" },
		} },
	},
	init = function(previous) return { query = "", email = "", pane = "mail", rich = false, thread = false, focus = false, closed = if previous then previous.closed or {} else {}, related_sort = "date", related_ascending = false } end,
	update = function(state, msg)
		if msg == "compose" or msg == "reply" or msg == "reply_all" or msg == "forward" or msg == "forward_text" then
			return state, { { to = "provider:mail", msg = { action = msg } } }
		elseif msg == "settings" then return state, { { to = "settings", msg = "open" } }
		elseif msg == "mail" or msg == "outbox" or msg == "organize" or msg == "operations" or msg == "drafts" or msg == "links" or msg == "identities" or msg == "searches" or msg == "folders" then
			state.pane = msg
			return state, { { to = "provider:mail", msg = { action = "partition_close" } } }
		elseif msg == "rich" then state.rich = not state.rich
		elseif msg == "focus-search" then state.focus = not state.focus
		elseif msg == "mailbox-chooser" or msg == "move-chooser" or msg == "copy-chooser" then
			state.chooser_return = state.pane; state.pane = msg; state.chooser_query = ""; state.chooser_index = 1; state.chooser_generation = (state.chooser_generation or 0) + 1
		elseif msg == "chooser-cancel" then state.pane = state.chooser_return or "mail"
		elseif msg == "export-selected" then state.pane = "export"
		elseif msg == "tags" then state.pane = "tags"
		elseif msg == "rules" then state.pane = "rules"
		elseif type(msg) == "table" then
			if msg.cancelled then return state
			elseif msg.rule_edit then state.rule_edit = table.clone(msg.rule_edit)
			elseif msg.rule_folder and state.rule_edit then state.rule_edit.mailbox = msg.rule_folder
			elseif msg.rule_field and state.rule_edit then state.rule_edit[msg.rule_field] = msg.value
			elseif msg.rule_toggle and state.rule_edit then state.rule_edit[msg.rule_toggle] = not state.rule_edit[msg.rule_toggle]
			elseif msg.msg == "tag-keyword" then state.tag_keyword = msg.value
			elseif msg.msg == "tag-label" then state.tag_label = msg.value
			elseif msg.tag_edit then state.tag_keyword = msg.tag_edit.keyword; state.tag_label = msg.tag_edit.label
			elseif msg.folder_account then state.folder_account = msg.folder_account; state.folder_edit = nil
			elseif msg.folder_edit then state.folder_edit = table.clone(msg.folder_edit); state.folder_edit.mode = msg.mode; state.folder_value = if msg.mode == "rename" then msg.folder_edit.name else ""
			elseif msg.folder_edit_cancel then state.folder_edit = nil
			elseif msg.msg == "folder-name" then state.folder_name = msg.value
			elseif msg.msg == "folder-value" then state.folder_value = msg.value
			elseif msg.msg == "chooser-query" then state.chooser_query = msg.value; state.chooser_index = 1
			elseif msg.chooser_step then state.chooser_index = math.clamp((state.chooser_index or 1) + msg.chooser_step, 1, math.max(msg.count or 0, 1))
			elseif msg.msg == "search-name" then state.search_name = msg.value
			elseif msg.search_source then
				state.search_sources = state.search_sources or {}
				local found = table.find(state.search_sources, msg.search_source)
				if found then table.remove(state.search_sources, found) else table.insert(state.search_sources, msg.search_source) end
			elseif msg.clear_search_sources then state.search_sources = {}
			elseif msg.edit_identity then state.identity_edit = table.clone(msg.edit_identity)
			elseif msg.identity_field and state.identity_edit then state.identity_edit[msg.identity_field] = msg.value
			elseif msg.attach_draft then
				return state, { { to = "app:pick", msg = { kind = "open", multiple = true, prompt = "Attach files", reply = { action = "draft_attach", id = msg.attach_draft } } } }
			elseif msg.save_attachment then
				return state, { { to = "app:pick", msg = { kind = "save", name = msg.name, reply = { action = "attachment_save", message = msg.message, part = msg.save_attachment } } } }
			elseif msg.save_message then
				return state, { { to = "app:pick", msg = { kind = "save", name = "message-" .. tostring(msg.save_message) .. ".eml", reply = { action = "message_save", message = msg.save_message } } } }
			elseif msg.export_messages then
				return state, { { to = "app:pick", msg = { kind = "folder", prompt = "Export original messages", reply = { action = "messages_save", messages = msg.export_messages, scope = msg.scope } } } }
			elseif msg.folder_toggle then state.closed[msg.folder_toggle] = not state.closed[msg.folder_toggle]
			elseif msg.action then
				if msg.action == "identity_account" then state.identity_edit = nil end
				if msg.action == "partition_open" then state.pane = "partitions" end
				if msg.action == "partition_choose" then state.pane = "mail" end
				if msg.action == "choose_move" or msg.action == "choose_copy" then state.pane = "mail" end
				if msg.action == "search_open" then
					state.pane = if msg.edit then "searches" else "mail"
					state.query = msg.source or ""; state.search_name = msg.name or ""; state.search_sources = table.clone(msg.sources or {})
				end
				if msg.action == "outbox_sent" then state.pane = "mail" end
				if msg.action == "reader_images" then state.rich = true end
				if msg.action == "attachments" then state.pane = "attachments" end
				if msg.action == "draft_discard" then state.pane = "drafts" end
				if msg.action == "folder" then state.query = "" end
				if msg.action == "folder" or msg.action == "filter" or msg.action == "select" or msg.action == "related" or msg.action == "search" then state.pane = "mail" end
				return state, { { to = "provider:mail", msg = msg } }
			elseif msg.msg == "query" then state.query = msg.value
			elseif msg.msg == "email" then state.email = msg.value end
		end
	end,
	view = function(state, app)
		local mail = app.mail or {}
		ROW = math.max(22, ((mail.preferences or {}).font_size or 12) + 10)
		local selected = mail.selected
		local title = "All Messages"
		local total_unread = 0
		for _, folder in mail.mailboxes or {} do
			total_unread += folder.unread
			if mail.mailbox == folder.id then title = if folder.name == "INBOX" then "Inbox" else folder.name end
		end
		for _, saved in mail.saved_searches or {} do
			if saved.id == mail.active_saved_search then title = saved.name .. (if mail.partition_value then " · " .. mail.partition_value else "") end
		end
		local sidebar = {
			ui.text { "Mailboxes", class = "sidebar-label" },
			mailbox_row("All Messages", "mails", total_unread, not mail.mailbox and state.pane == "mail" and mail.filter == "all", { action = "folder", id = 0 }),
		}
		for _, name in { "INBOX", "Archive", "Sent", "Junk", "Trash" } do
			local children = {}
			local unread = 0
			for _, folder in mail.mailboxes or {} do
				if folder.role == string.lower(name) and folder.selectable then
					table.insert(children, { key = "shortcut:" .. tostring(folder.id), name = folder.account, id = folder.id, unread = folder.unread, icon = "at-sign" })
					unread += folder.unread
				end
			end
			if #children > 0 then
				folder_nodes(sidebar, { { key = "role:" .. name, name = name, id = if #children == 1 then children[1].id else nil, unread = unread, children = children } }, state, mail, 0)
			end
		end
		table.insert(sidebar, mailbox_row("Drafts", "file-text", mail.draft_count or 0, state.pane == "drafts", "drafts"))
		table.insert(sidebar, mailbox_row("Outbox", "send", mail.outbox_count or 0, state.pane == "outbox", "outbox"))
		table.insert(sidebar, mailbox_row("Saved searches", "search", #(mail.saved_searches or {}), state.pane == "searches", "searches"))
		table.insert(sidebar, mailbox_row("Mail operations", "list-checks", mail.unresolved_operations or 0, state.pane == "operations", "operations"))
		table.insert(sidebar, mailbox_row("Move & folders", "folder-open", nil, state.pane == "organize", "organize"))
		table.insert(sidebar, ui.text { "Views", class = "sidebar-label" })
		table.insert(sidebar, mailbox_row("Unread", "mail", nil, mail.filter == "unread", { action = "filter", value = "unread" }))
		table.insert(sidebar, mailbox_row("Flagged", "flag", nil, mail.filter == "starred", { action = "filter", value = "starred" }))
		table.insert(sidebar, ui.text { "Sources", class = "sidebar-label" })
		local sources = table.clone(mail.folder_tree or {})
		for _, source in sources do source.icon = "at-sign" end
		folder_nodes(sidebar, sources, state, mail, 0)
		if #sources == 0 then table.insert(sidebar, mailbox_row("Connect an account…", "at-sign", nil, state.pane == "settings", "settings")) end
		local related = table.clone((mail.related or {}).rows or {})
		if mail.related and mail.related.anchor then table.insert(related, 1, mail.related.anchor) end
		local reader
		if state.pane == "settings" then
			-- The capture harness can mount the same window body for layout checks.
			reader = ui.mount "settings"
		elseif state.pane == "mailbox-chooser" or state.pane == "move-chooser" or state.pane == "copy-chooser" then
			reader = mailbox_chooser { state = state, mail = mail }
		elseif state.pane == "folders" then
			reader = folder_manager { state = state, mail = mail }
		elseif state.pane == "tags" then
			reader = tags { state = state, mail = mail }
		elseif state.pane == "rules" then
			reader = entry_rules { state = state, mail = mail }
		elseif state.pane == "partitions" then
			reader = search_partitions { mail = mail }
		elseif state.pane == "export" then
			reader = ui.column { class = "form", grow = true,
				ui.text { "Export original messages", class = "title" },
				ui.text { tostring(#(mail.move_selection or {})) .. " selected messages", weight = "semibold" },
				ui.text { "Choose a destination folder. Moonmail creates a new private subfolder containing exact, hash-verified .eml files and a completion manifest. Existing files are preserved.", tone = "muted" },
				ui.text { "Exports include the selected message on each checked row, not hidden replies. Import the resulting folder to recover message contents. Folder membership, flags, drafts, settings and credentials are not part of this export.", tone = "muted" },
				ui.row { ui.button { label = "Choose destination…", disabled = #(mail.move_selection or {}) == 0, msg = { export_messages = mail.move_selection, scope = mail.mailbox } }, ui.button { label = "Back to mail", msg = "mail" } },
			}
		elseif state.pane == "organize" then
			local items = { ui.text { "Move & folders", class = "title" },
				ui.text { "Move the selected message or checked rows. Remote moves run on the next Sync; local demo/import moves apply immediately.", class = "muted" },
				ui.text { "Archive and Trash use server folder roles. Assign an override below if a role is missing or ambiguous.", class = "muted" },
				ui.row { action("Archive", "archive_messages"), action("Trash", "trash_messages"), ui.button { label = "Manage folders…", msg = "folders", small = true }, ui.button { label = "Mail operations", msg = "operations", small = true }, ui.button { label = "Back to mail", msg = "mail", small = true } },
				ui.row {
					action("Mark selected read", "selection_flag", { messages = mail.move_selection, scope = mail.mailbox, kind = "seen", value = true }),
					action("Mark selected unread", "selection_flag", { messages = mail.move_selection, scope = mail.mailbox, kind = "seen", value = false }),
					action("Flag selected", "selection_flag", { messages = mail.move_selection, scope = mail.mailbox, kind = "flagged", value = true }),
					action("Unflag selected", "selection_flag", { messages = mail.move_selection, scope = mail.mailbox, kind = "flagged", value = false }),
				},
				ui.button { label = "Tags & keywords…", small = true, msg = "tags" },
			}
			for _, folder in mail.mailboxes or {} do
				if folder.selectable then table.insert(items, ui.row { padding = "small", align = "center",
					ui.column { grow = true, ui.text { folder.name, weight = "semibold" }, ui.text { folder.account .. (if folder.role then " · " .. folder.role else ""), tone = "muted", size = "small" } },
					action("Move here", "move_messages", { id = folder.id }),
					action("Copy here", "copy_messages", { id = folder.id }),
					action("Use as Archive", "folder_role", { id = folder.id, role = "archive" }),
					action("Use as Trash", "folder_role", { id = folder.id, role = "trash" }),
					action("Use as Sent", "folder_role", { id = folder.id, role = "sent" }),
					action("Use as Junk", "folder_role", { id = folder.id, role = "junk" }),
					action("Server default", "folder_role", { id = folder.id, role = "default" }),
				}) end
			end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "operations" then
			local items = { ui.text { "Mail operations", class = "title" },
				ui.text { "Queued moves survive restart. Sync processes up to 100 per run. Uncertain moves are never automatically repeated.", class = "muted" },
				ui.text { tostring(mail.unresolved_operations or 0) .. " unresolved · showing up to 100 transfers and 100 folder changes, unresolved first in each group", class = "muted" },
				ui.row { ui.button { label = "Sync now", small = true, disabled = mail.busy or mail.demo, msg = { action = "sync" }, to = "provider:mail" }, ui.button { label = "Tags & keyword recovery", small = true, msg = "tags" }, ui.button { label = "Back to mail", small = true, msg = "mail" } },
			}
			for _, item in mail.move_operations or {} do
				table.insert(items, ui.column { padding = "normal", gap = "small",
					ui.text { item.subject, weight = "semibold" },
					ui.text { (if item.kind == "copy" then "Copy · " else "Move · ") .. item.account .. " · " .. item.source .. " → " .. item.destination },
					ui.text { item.state .. (if item.detail ~= "" then " · " .. item.detail else " · Waiting for Sync"), tone = if item.state == "unknown" or item.state == "failed" then "accent" else "muted" },
					if item.state == "queued" or item.state == "failed" then action("Cancel", "cancel_move", { id = item.id }) else nil,
					if item.state == "unknown" then action("I checked both folders — dismiss", "dismiss_move", { id = item.id }) else nil,
				})
			end
			for _, item in mail.folder_operations or {} do
				local change = item.change
				table.insert(items, ui.column { padding = "normal", gap = "small",
					ui.text { "Folder " .. change.kind .. " · " .. item.account, weight = "semibold" },
					ui.text { if change.kind == "rename" then change.source .. " → " .. change.destination else change.name },
					ui.text { item.state .. " · " .. (if item.detail ~= "" then item.detail else "Waiting for Sync"), tone = if item.state == "unknown" or item.state == "failed" then "accent" else "muted" },
					if item.state == "queued" or item.state == "failed" then action("Cancel folder change", "folder_cancel", { id = item.id }) else nil,
					if item.state == "unknown" then action("I inspected the folder outcome — dismiss", "folder_dismiss", { id = item.id }) else nil,
				})
			end
			if #(mail.move_operations or {}) == 0 and #(mail.folder_operations or {}) == 0 then table.insert(items, ui.text { "No transfer or folder operations yet. Keyword edits are shown in Tags.", tone = "muted" }) end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "outbox" then
			local items = { ui.text { "Outbox", class = "title" }, ui.text { "Unknown delivery cannot be retried here. Check Sent before composing a replacement.", class = "muted" },
				ui.row { align = "center", ui.text { tostring(mail.outbox_count or 0) .. " records · newest first", tone = "muted" }, action("First", "outbox_first"), ui.button { label = "Next", small = true, disabled = not mail.outbox_more, msg = { action = "outbox_next" }, to = "provider:mail" } },
			}
			for _, item in mail.outbox or {} do
				table.insert(items, ui.column { gap = "small", padding = "normal",
					ui.text { if item.subject ~= "" then item.subject else "(No subject)", weight = "bold", truncate = true },
					ui.text { item.account .. " · " .. tostring(#item.recipients) .. " recipient(s)", tone = "muted", truncate = true },
					ui.text { item.state .. " · " .. item.detail },
					if item.sent_copy_state and item.sent_copy_state ~= "server" then ui.text { "IMAP Sent copy: " .. item.sent_copy_state .. (if item.sent_copy_detail ~= "" then " · " .. item.sent_copy_detail else " · Sync to finish saving"), tone = if item.sent_copy_state == "unknown" then "accent" else "muted" } else nil,
					if item.state == "sent" or item.state == "unknown" then ui.text { ({ visible = "Sent copy: matching Message-ID visible in synced Sent", missing = "Sent copy: not found in synced Sent; this does not prove non-delivery", ambiguous = "Sent copy: multiple matches; inspect Sent", unavailable = "Sent copy: configure an unambiguous Sent folder role", pending = "Sent copy: awaiting Sync", untracked = "Sent copy: older entry has no tracked Message-ID" })[item.sent_status or "untracked"], tone = "muted" } else nil,
					if (item.state == "queued" or item.state == "failed") and not item.editable then ui.text { "Older entry: no saved draft snapshot available for editing.", tone = "muted" } else nil,
					ui.row {
						if item.sent_message then ui.button { label = "Show Sent copy", small = true, msg = { action = "outbox_sent", id = item.id } } else nil,
						if item.state == "queued" then ui.button { label = "Send queued message", small = true, disabled = mail.demo or mail.busy, msg = { action = "send", id = item.id }, to = "provider:mail" } else nil,
						if item.state == "failed" then ui.button { label = "Retry submission", small = true, disabled = mail.demo or mail.busy, msg = { action = "outbox_retry", id = item.id }, to = "provider:mail" } else nil,
						if item.editable then ui.button { label = "Return to drafts", small = true, msg = { action = "outbox_edit", id = item.id } } else nil,
						if item.state == "queued" or item.state == "failed" then action("Cancel", "outbox_cancel", { id = item.id }) else nil,
					},
				})
			end
			if #(mail.outbox or {}) == 0 then table.insert(items, ui.text { "No outbox records on this page.", tone = "muted" }) end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "links" then
			local links = mail.reader_links or { items = {} }
			local items = { ui.text { "Message links", class = "title" }, ui.text { "Open an HTTP(S) link in your system browser. The email reader stays local.", tone = "muted" }, ui.row { ui.button { label = "Back to mail", small = true, msg = "mail" } } }
			for index, url in links.items or {} do
				table.insert(items, ui.row { align = "center", shrink = false, padding = "normal", ui.column { grow = true, ui.text { url, truncate = true, copy = url, tooltip = url } }, ui.button { label = "Open in browser", small = true, msg = { action = "reader_link", message = if selected then selected.summary.id else 0, index = index - 1 }, to = "provider:mail" } })
			end
			if links.truncated then table.insert(items, ui.text { "Showing up to 100 links from the first 512 KiB of each message alternative.", tone = "muted" }) end
			if #(links.items or {}) == 0 then table.insert(items, ui.text { "No supported links found.", tone = "muted" }) end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "attachments" then
			local items = { ui.text { "Attachments", class = "title" },
				ui.text { if selected then selected.summary.subject else "Select a message", tone = "muted" },
				ui.text { "Save decoded files locally. Choose a new filename; existing files are not replaced.", tone = "muted" },
				ui.button { label = "Back to mail", msg = "mail", small = true },
			}
			for _, item in mail.attachments or {} do
				table.insert(items, ui.row { padding = "normal", align = "center", shrink = false,
					ui.column { grow = true, gap = "small", ui.text { item.name, weight = "semibold", truncate = true }, ui.text { item.media_type .. " · " .. file_size(item.bytes) .. (if item.inline then " · inline" else ""), tone = "muted", truncate = true } },
					ui.button { label = "Save As…", small = true, msg = { save_attachment = item.id, message = mail.attachment_message, name = item.name } },
				})
			end
			if mail.attachment_message and #(mail.attachments or {}) == 0 then table.insert(items, ui.text { "This message has no attachments.", tone = "muted" }) end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "drafts" then
			local page = mail.drafts or { rows = {} }
			local items = page.rows or {}
			reader = ui.column { grow = true, gap = "none",
				ui.row { class = "drafts-heading", height = 48, shrink = false, align = "center",
					ui.text { "Drafts", class = "title" }, ui.text { tostring(mail.draft_count or 0) .. " saved", tone = "muted", size = "small" }, ui.spacer {},
					ui.button { label = "New message", icon = "square-pen", msg = "compose", small = true },
				},
				ui.row { height = 25, shrink = false, gap = "none", class = "mail-table-heading", cell("To", 240, true), cell("Subject", nil, true), cell("Account", 200, true), cell("Modified · UTC ↓", 180, true, nil, true) },
				if #items > 0 then ui.list { id = "drafts-list", count = #items, row_height = ROW, grow = true,
					row = function(i)
						local draft = items[i]
						return ui.row { height = ROW, shrink = false, align = "center", gap = "none", class = if i % 2 == 0 then "mail-table-row mail-table-row-even" else "mail-table-row",
							selected = mail.draft ~= nil and mail.draft.id == draft.id, tooltip = "Open in the composer window", msg = { action = "draft_open", id = draft.id },
							cell(if draft.recipients ~= "" then draft.recipients else "No recipients", 240, false), cell(if draft.subject ~= "" then draft.subject else "(No subject)", nil, false), cell(draft.account, 200, false, "muted"), cell(string.sub(draft.updated_label or "", 6, 22), 180, false, "muted"),
						}
					end,
				} else ui.column { grow = true, padding = "large", align = "center", justify = "center", ui.text { "No drafts on this page", tone = "muted" } },
				ui.row { class = "mail-table-footer", height = 30, shrink = false, align = "center", ui.text { "Newest edits first · click a draft to continue writing", size = "small", tone = "muted" }, ui.spacer {}, action("First", "drafts_first"), ui.button { label = "Next", small = true, disabled = not page.next, msg = { action = "drafts_next" }, to = "provider:mail" } },
			}
		elseif state.pane == "searches" then
			local items = { ui.text { "Saved searches", class = "title" },
				ui.text { "Search fields: subject:, body:, from:, to:, cc:, account:, in:, keyword:, tag:, sender-address:, recipient-address:, is:unread/read/starred, after:YYYY-MM-DD, before:YYYY-MM-DD.", tone = "muted" },
				ui.text { 'Use uppercase AND, OR, NOT and parentheses. Adjacent terms mean AND. Double quotes keep phrases together. Dates use UTC midnight; after is inclusive, before exclusive.', tone = "muted" },
				ui.text { "Current query: " .. (mail.search or ""), weight = "semibold" },
				ui.input { id = "saved-search-name", value = state.search_name or "", placeholder = "Name this query", change = "search-name" },
				ui.row { action("Save query", "search_save", { name = state.search_name or "", sources = state.search_sources or {} }), ui.button { label = "Back to mail", small = true, msg = "mail" } },
				ui.text { "Saving the same name replaces its query and sources. Selected sources are combined with OR, then filtered by this query. No sources means all mail.", tone = "muted" },
				ui.row { ui.text { tostring(#(state.search_sources or {})) .. " sources selected for saving", tone = "muted" }, ui.button { label = "Clear sources", small = true, msg = { clear_search_sources = true } } },
			}
			for _, saved in mail.saved_searches or {} do
				local source_names = {}
				for _, id in saved.sources or {} do
					for _, source in mail.saved_searches or {} do if source.id == id then table.insert(source_names, source.name) end end
				end
				table.insert(items, ui.row { padding = "normal", shrink = false, align = "center",
					ui.column { grow = true, ui.text { saved.name, weight = "semibold", truncate = true }, ui.text { saved.source, tone = "muted", truncate = true }, ui.text { "Sources: " .. (if #source_names == 0 then "All mail" else table.concat(source_names, " OR ")), tone = "muted", truncate = true } },
					ui.button { label = if table.find(state.search_sources or {}, saved.id) then "Source ✓" else "Use source", small = true, msg = { search_source = saved.id } },
					ui.button { label = "Open", small = true, msg = { action = "search_open", id = saved.id, source = saved.source, name = saved.name, sources = saved.sources } },
					ui.button { label = "Partitions", small = true, msg = { action = "partition_open", id = saved.id } },
					ui.button { label = "Edit", small = true, msg = { action = "search_open", id = saved.id, source = saved.source, name = saved.name, sources = saved.sources, edit = true } },
					action("Remove", "search_delete", { id = saved.id }),
				})
			end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "identities" then
			reader = identities_view(state, mail)
		else
			reader = if selected then ui.column { grow = true, gap = "none",
				ui.column { class = "reader-head", gap = "none", shrink = false,
					header_field("From", selected.summary.sender .. " <" .. selected.summary.address .. ">"),
					header_field("Subject", selected.summary.subject),
					header_field(if mail.date_header then "Date" else "Sort date", selected.summary.date_label),
					if mail.received_date and mail.received_date ~= "" then header_field("Received", mail.received_date) else nil,
					header_field("To", selected.summary.recipients),
				},
				ui.row { class = "reader-notice", height = 27, shrink = false, align = "center",
					ui.icon { name = "shield", size = "small" },
					ui.text { if selected.html and state.rich then "HTML · External resources blocked" else "Plain text", size = "small" },
					ui.spacer {}, ui.button { label = "Save .eml…", small = true, variant = "ghost", msg = { save_message = selected.summary.id } }, ui.button { label = "Links…", small = true, variant = "ghost", msg = "links" }, ui.button { label = "Attachments…", small = true, variant = "ghost", msg = { action = "attachments", message = selected.summary.id } }, ui.button { label = "Embedded images", small = true, variant = "ghost", msg = { action = "reader_images", message = selected.summary.id }, disabled = not selected.html }, ui.button { label = if state.rich then "Plain text" else "HTML", small = true, variant = "ghost", msg = "rich", disabled = not selected.html },
				},
				if mail.reader_images and mail.reader_images ~= "" then ui.text { mail.reader_images, size = "small", tone = "muted" } else nil,
				if #(mail.thread_warnings or {}) > 0 then ui.text { table.concat(mail.thread_warnings, "\n"), size = "small", tone = "muted" } else nil,
				ui.column { grow = true, ui.browser { id = "message-reader", url = if state.rich and mail.html_url ~= "" then mail.html_url else mail.plain_url } },
			} else ui.column { class = "form", ui.text { "No message selected", tone = "muted" } }
		end
		return ui.column { grow = true, gap = "none",
			ui.row { class = "toolbar", height = 54, shrink = false, align = "center", drag = true,
				ui.row { width = 90, shrink = false },
				tool("chevron-up", "Previous message · ⌘↑", { action = "move", value = "up" }, "provider:mail"),
				tool("chevron-down", "Next message · ⌘↓", { action = "move", value = "down" }, "provider:mail"),
				tool("square-pen", "Compose · ⌘N", "compose"),
				tool("reply", "Reply", "reply", nil, not selected),
				ui.button { label = "Reply All", icon = "reply", small = true, msg = "reply_all", disabled = not selected },
				ui.button { label = "Forward", small = true, msg = "forward", disabled = not selected },
				tool("flag", "Toggle flag", { action = "star" }, "provider:mail", not selected or mail.selected_moving),
				tool("mail", "Mark unread", { action = "unread" }, "provider:mail", not selected or mail.selected_moving),
				tool("archive", "Archive selected or checked messages", { action = "archive_messages" }, "provider:mail", not selected and (mail.checked_count or 0) == 0),
				tool("trash-2", "Move selected or checked messages to Trash", { action = "trash_messages" }, "provider:mail", not selected and (mail.checked_count or 0) == 0),
				tool("folder-open", "Move to folder / configure folder roles", "organize"),
				tool("refresh-cw", "Sync · ⌘R", { action = "sync" }, "provider:mail", mail.busy or mail.demo),
				if mail.syncing then action("Stop Sync", "cancel_sync") else nil,
				ui.button { label = title, small = true, variant = "ghost", msg = "mail" },
				ui.spacer {},
				tool("list-tree", if mail.threaded then "Show individual messages" else "Group messages into expandable threads", { action = "threaded" }, "provider:mail", false, mail.threaded),
				if mail.demo then ui.text { "Demo", class = "muted" } else nil,
				ui.input { id = "search", value = state.query, width = 270, placeholder = "Search messages · Enter", change = "query", submit = { action = "search" }, focus = state.focus },
				tool("search", "Search", { action = "search", value = state.query }),
				tool("settings", "Appearance, account & storage", "settings"),
				ui.mount "feedback",
			},
			if mail.error and mail.error ~= "" then ui.text { mail.error, class = "error" } else nil,
			if (mail.checked_count or 0) > 0 then ui.row { height = 30, shrink = false, padding = "small", align = "center",
				ui.text { tostring(mail.checked_count) .. " message(s) checked · thread rows select only their representative", tone = "accent" },
				ui.spacer {}, action("Clear selection", "clear_selection"),
			} else nil,
			-- Panes own one inset border each. Split handles still resize, but
			-- their overlay hairlines would double borders and disappear behind WebKit.
			ui.row { grow = true, gap = "none",
				activity_bar {
					selected = state.pane,
					items = {
						{ id = "mail", icon = "mails", tooltip = "Mail", count = total_unread, msg = "mail" },
						{ id = "drafts", icon = "file-text", tooltip = "Drafts", count = mail.draft_count or 0, msg = "drafts" },
						{ id = "outbox", icon = "send", tooltip = "Outbox", count = mail.outbox_count or 0, msg = "outbox" },
						{ id = "searches", icon = "search", tooltip = "Saved searches", msg = "searches" },
						{ id = "operations", icon = "list-checks", tooltip = "Mail operations", count = mail.unresolved_operations or 0, msg = "operations" },
					},
					bottom = { { id = "settings", icon = "settings", tooltip = "Settings", msg = "settings" } },
				},
				ui.hsplit { id = "mailmate-layout-v1", divider = false,
					ui.pane { size = 220, min = 170, max = 340, fixed = true, class = "sidebar mail-pane-right",
						ui.column { grow = true, scroll = true, gap = "none", table.unpack(sidebar) },
					},
					ui.pane { min = 600,
						if state.pane ~= "mail" then reader else ui.vsplit { id = "message-table-and-preview", divider = false,
							ui.pane { size = 380, min = 160, fixed = true, class = "mail-pane-bottom",
								ui.column { grow = true, gap = "none",
									table_header(false, state, mail),
									rows(mail.rows, mail.anchor, false, mail.list_key or "initial", mail.threaded, columns(mail, false)),
									ui.row { height = 26, class = "mail-table-footer", shrink = false, align = "center",
										ui.text { tostring(mail.page_count or 0) .. (if mail.threaded then " threads" else " messages") .. " shown", class = "muted" },
										ui.spacer {}, action("First", "first"),
										ui.button { label = "Previous", small = true, disabled = not mail.previous, msg = { action = "previous" }, to = "provider:mail" },
										ui.button { label = "Next", small = true, disabled = not mail.more, msg = { action = "next" }, to = "provider:mail" },
									},
								},
							},
							ui.pane { min = 220,
								ui.hsplit { id = "correspondence-and-reader", divider = false,
									ui.pane { size = 390, min = 260, max = 650, fixed = true, class = "mail-pane-right",
										ui.column { grow = true, gap = "none",
											ui.row { height = 27, class = "correspondence-title", shrink = false, align = "center",
												ui.text { if mail.related_thread then "Reply Thread" else "Correspondence", weight = "medium" },
												ui.spacer {}, ui.text { tostring(#related) .. " · anchor pinned", class = "muted" },
												ui.button { icon = "list", small = true, variant = "ghost", tooltip = if mail.related_thread then "Show Correspondence" else "Show reply thread", msg = { action = "related_mode", anchor = mail.anchor }, to = "provider:mail" },
											},
											table_header(true, state, mail),
											rows(related, selected and selected.summary.id, true, (mail.related_key or "") .. (if mail.related_thread then "-thread" else "-pairs"), false, columns(mail, true)),
											ui.row { height = 26, shrink = false, action("First", "related_first", { anchor = mail.anchor }), ui.button { label = "Next", small = true, disabled = not mail.related or not mail.related.next, msg = { action = "related_next", anchor = mail.anchor }, to = "provider:mail" } },
										},
									},
									ui.pane { min = 320, reader },
								},
							},
						},
					},
				},
			},
			ui.row { class = "status", height = 22, shrink = false, align = "center", ui.text { mail.status or "Opening local archive…" }, ui.spacer {}, ui.text { "⌘N Compose   ⌘R Sync   ⌘↑/↓ Navigate" } },
		}
	end,
}
