local function build(props)
	local state, mail = props.state, props.mail
	local account = mail.keyword_account or "local"
	local keyword, label = state.tag_keyword or "", state.tag_label or ""
	local same = account == mail.move_account and #(mail.move_selection or {}) > 0
	local function edit(value, wire)
		return { action = "keyword_set", account = account, messages = mail.move_selection, scope = mail.mailbox, keyword = wire or keyword, value = value }
	end
	local items = {
		ui.text { "Tags & IMAP keywords", class = "title" },
		ui.text { "Edits apply to the selected messages in every live folder. Remote edits upload on Sync. Friendly labels stay local to this account.", tone = "muted" },
		ui.button { label = "Folder-entry rules…", small = true, msg = "rules" },
		ui.row { ui.button { label = "Back to mail", small = true, msg = "mail" }, ui.button { label = "Sync", small = true, disabled = mail.busy or mail.demo, msg = { action = "sync" } } },
	}
	local known = {}
	for _, folder in mail.mailboxes or {} do
		if known[folder.account] then continue end
		known[folder.account] = true
		table.insert(items, ui.button { label = folder.account, small = true, variant = if account == folder.account then "primary" else "ghost", msg = { action = "keyword_account", account = folder.account } })
	end
	table.insert(items, ui.text { "Selected message keywords: " .. (if #(mail.selected_keywords or {}) > 0 then table.concat(mail.selected_keywords, ", ") else "none"), tone = "muted" })
	table.insert(items, ui.text { tostring(#(mail.move_selection or {})) .. " selected · " .. account, weight = "semibold" })
	table.insert(items, ui.input { id = "tag-keyword", value = keyword, placeholder = "Wire keyword, e.g. $Label1 or project-work", change = "tag-keyword" })
	table.insert(items, ui.row {
		ui.button { label = "Add to selected", small = true, disabled = not same or keyword == "", msg = edit(true) },
		ui.button { label = "Remove from selected", small = true, disabled = not same or keyword == "", msg = edit(false) },
	})
	table.insert(items, ui.input { id = "tag-label", value = label, placeholder = "Friendly label (empty removes mapping)", change = "tag-label" })
	table.insert(items, ui.button { label = "Save label", small = true, disabled = keyword == "", msg = { action = "keyword_label", account = account, keyword = keyword, label = label } })
	table.insert(items, ui.text { "Account labels · first 500 alphabetically", weight = "semibold" })
	for _, tag in mail.keyword_labels or {} do
		table.insert(items, ui.row { shrink = false, align = "center",
			ui.column { grow = true, ui.text { tag.label, truncate = true }, ui.text { tag.keyword, tone = "muted", truncate = true } },
			ui.button { label = "Edit", small = true, msg = { tag_edit = tag } },
			ui.button { label = "Add", small = true, disabled = not same, msg = edit(true, tag.keyword) },
			ui.button { label = "Remove", small = true, disabled = not same, msg = edit(false, tag.keyword) },
		})
	end
	table.insert(items, ui.text { "Search with keyword:project-work or tag:\"Work projects\"", tone = "muted" })
	table.insert(items, ui.text { "Pending keyword edits · 100 per page", weight = "semibold" })
	for _, op in mail.keyword_operations or {} do
		table.insert(items, ui.column { gap = "small", shrink = false,
			ui.text { (if op.value then "Add " else "Remove ") .. op.keyword .. " · " .. op.mailbox .. " · UID " .. tostring(op.uid) .. " · " .. op.state },
			if op.detail ~= "" then ui.text { op.detail, tone = "muted" } else nil,
			if op.state == "blocked" then ui.row {
				ui.button { label = "Retry on Sync", small = true, disabled = mail.busy, msg = { action = "keyword_retry", account = account, id = op.id } },
				ui.button { label = "Discard intent", small = true, disabled = mail.busy, msg = { action = "keyword_discard", account = account, id = op.id } },
			} else nil,
		})
	end
	table.insert(items, ui.text { "Discarded edits return to server state after the next complete Sync.", tone = "muted" })
	table.insert(items, ui.row {
		ui.button { label = "First page", small = true, disabled = (mail.keyword_after or 0) == 0, msg = { action = "keyword_first", account = account } },
		ui.button { label = "Next page", small = true, disabled = not mail.keyword_next, msg = { action = "keyword_next", account = account, id = mail.keyword_next } },
	})
	return ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
end

return component { title = "Tags", example = { state = {}, mail = {} }, build = build }
