local function matches(mail, query, moving)
	local words = {}
	for word in string.gmatch(string.lower(query), "%S+") do table.insert(words, word) end
	local found = {}
	for _, folder in mail.mailboxes or {} do
		if not folder.selectable or (moving and (not mail.move_account or folder.account ~= mail.move_account or folder.id == mail.mailbox)) then continue end
		local label = string.lower(folder.account .. " " .. folder.name)
		local include = true
		for _, word in words do if not string.find(label, word, 1, true) then include = false; break end end
		if include then table.insert(found, folder) end
	end
	table.sort(found, function(a, b)
		local left, right = string.lower(a.name), string.lower(b.name)
		if left ~= right then return left < right end
		if a.account ~= b.account then return a.account < b.account end
		return a.id < b.id
	end)
	local total = #found
	while #found > 100 do table.remove(found) end
	return found, total
end

local function build(props)
	local state, mail = props.state, props.mail
	local copying = state.pane == "copy-chooser"
	local moving = state.pane == "move-chooser" or copying
	local folders, total = matches(mail, state.chooser_query or "", moving)
	local index = math.clamp(state.chooser_index or 1, 1, math.max(#folders, 1))
	local function choice(folder)
		return if folder then { action = if copying then "choose_copy" elseif moving then "choose_move" else "folder", id = folder.id, messages = mail.move_selection, scope = mail.mailbox } else "chooser-empty"
	end
	local items = {
		ui.text { if copying then "Copy to mailbox" elseif moving then "Move to mailbox" else "Go to mailbox", class = "title" },
		ui.text { if moving then tostring(#(mail.move_selection or {})) .. " selected · " .. (mail.move_account or "Select messages from one account") else "Search folder paths and account addresses", tone = "muted" },
		ui.input { id = "mailbox-chooser-" .. tostring(state.chooser_generation or 0), value = state.chooser_query or "", placeholder = "Type a mailbox name…", focus = true,
			change = "chooser-query", submit = choice(folders[index]),
			keys = { down = { chooser_step = 1, count = #folders }, up = { chooser_step = -1, count = #folders }, escape = "chooser-cancel" },
		},
		ui.row { ui.text { "↑ / ↓ select · Enter " .. (if copying then "copies" elseif moving then "moves" else "opens") .. " · Escape cancels", tone = "muted" }, ui.button { label = "Cancel", small = true, msg = "chooser-cancel" } },
		ui.text { tostring(total) .. " matching mailboxes" .. (if total > 100 then " · showing first 100; keep typing to narrow" else ""), tone = "muted" },
	}
	local rows = {}
	for position, folder in folders do
		table.insert(rows, ui.row { padding = "normal", shrink = false, align = "center", class = "chooser-row", selected = position == index,
			ui.text { if position == index then "›" else " ", tone = "accent" },
			ui.column { grow = true, ui.text { folder.name, weight = "semibold", truncate = true }, ui.text { folder.account, tone = "muted", truncate = true } },
			ui.text { tostring(folder.unread) .. " unread", tone = "muted" },
			ui.button { label = if copying then "Copy here" elseif moving then "Move here" else "Open", small = true, msg = choice(folder) },
		})
	end
	return ui.column { class = "form", grow = true,
		ui.column { shrink = false, table.unpack(items) },
		ui.column { grow = true, scroll = true, scroll_to = index, table.unpack(rows) },
	}
end

return component {
	title = "Mailbox chooser",
	example = { state = { pane = "mailbox-chooser" }, mail = {} },
	build = build,
}
