local function action(label, name, extra)
	local msg = extra or {}
	msg.action = name
	return ui.button { label = label, small = true, msg = msg, to = "provider:mail" }
end

local ROW = 22

local function cell(text, width, heading, tone, last)
	return ui.column { width = width, height = if heading then 25 else ROW, gap = "none", grow = width == nil, shrink = width == nil,
		justify = "center", class = if heading then (if last then "mail-table-heading-cell mail-table-heading-last" else "mail-table-heading-cell") else "mail-table-cell",
		ui.text { text, truncate = true, tone = tone, weight = if heading then "semibold" else "normal" },
	}
end

local function heading(label, width, field, related, state, mail, last)
	local sorted = if related then state.related_sort else mail.sort or "date"
	local ascending = if related then state.related_ascending else mail.ascending
	return ui.row { width = width, height = 25, grow = width == nil, shrink = width == nil, align = "center", gap = "none",
		class = if last then "mail-table-heading-cell mail-table-heading-last" else "mail-table-heading-cell",
		msg = if related then { sort_related = field } else { action = "sort", field = field },
		ui.column { grow = true, ui.text { label, truncate = true, weight = "semibold" } },
		if sorted == field then ui.icon { name = if ascending then "chevron-up" else "chevron-down", size = "small" } else nil,
	}
end

local function table_header(related, state, mail)
	return ui.row { height = 25, shrink = false, gap = "none", class = "mail-table-heading",
		if not related then cell("●", 24, true) else nil,
		if not related then cell("⚑", 24, true) else nil,
		if not related then heading("From", 220, "sender", false, state, mail) else nil,
		heading("Subject", nil, "subject", related, state, mail),
		heading("Date", if related then 106 else 156, "date", related, state, mail, true),
	}
end

local function rows(messages, selected_id, related, key, threaded)
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
			return ui.row {
				height = ROW, shrink = false, gap = "none", padding = "none", align = "center",
				class = if i % 2 == 0 then "mail-table-row mail-table-row-even" else "mail-table-row",
				selected = selected_id == mail.id,
				msg = { action = if related then "related" else "select", id = mail.id },
				if not related then cell(if mail.unread then "●" else "", 24, false, "accent") else nil,
				if not related then cell(if mail.starred then "⚑" else "", 24, false) else nil,
				if not related then ui.row { width = 220, height = ROW, shrink = false, align = "center", gap = "none",
					if threaded then ui.row { width = 20 + (mail.depth or 0) * 14, shrink = false, justify = "end", align = "center",
						if (mail.thread_count or 1) > 1 then ui.row { width = 20, height = ROW, align = "center", justify = "center", msg = { action = "expand", id = mail.id },
							ui.icon { name = if mail.expanded then "chevron-down" else "chevron-right", size = "small" },
						} else nil,
					} else nil,
					cell(mail.sender, nil, false),
				} else nil,
				cell(mail.subject .. (if threaded and (mail.thread_count or 1) > 1 then "  (" .. tostring(mail.thread_count) .. ")" elseif mail.thread_capped then "  · First 100 messages" else ""), nil, false),
				cell(string.sub(mail.date_label, 6, if related then 16 else 22), if related then 106 else 156, false),
			}
		end,
	}
end

local function tool(icon, tip, msg, to, disabled, on)
	return ui.row { width = 34, height = 34, shrink = false, align = "center", justify = "center", class = "mail-tool",
		state = if disabled then "disabled" elseif on then "on" else nil,
		tooltip = tip, msg = if disabled then nil else msg, to = if disabled then nil else to,
		ui.icon { name = icon, size = "large" },
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

return mod {
	window = { size = { width = 1500, height = 940 } },
	about = { version = "0.1", description = "Local-first mail, with Correspondence." },
	menus = {
		{ title = "Mail", items = {
			{ label = "Settings…", msg = "open", to = "settings", shortcut = "secondary-," },
			{ label = "Compose", msg = "compose", shortcut = "secondary-n" },
			{ label = "Search", msg = "focus-search", shortcut = "secondary-f" },
			{ label = "Sync", msg = { action = "sync" }, to = "provider:mail", shortcut = "secondary-r" },
			{ label = "Next Message", msg = { action = "move", value = "down" }, to = "provider:mail", shortcut = "secondary-down" },
			{ label = "Previous Message", msg = { action = "move", value = "up" }, to = "provider:mail", shortcut = "secondary-up" },
		} },
	},
	init = function(previous) return { query = "", email = "", pane = "mail", rich = false, thread = false, focus = false, closed = if previous then previous.closed or {} else {}, related_sort = "date", related_ascending = false } end,
	update = function(state, msg)
		if msg == "compose" or msg == "reply" then
			state.pane = "compose"
			return state, { { to = "provider:mail", msg = { action = msg } } }
		elseif msg == "settings" then return state, { { to = "settings", msg = "open" } }
		elseif msg == "mail" or msg == "outbox" then state.pane = msg
		elseif msg == "rich" then state.rich = not state.rich
		elseif msg == "thread" then state.thread = not state.thread
		elseif msg == "focus-search" then state.focus = not state.focus
		elseif type(msg) == "table" then
			if msg.folder_toggle then state.closed[msg.folder_toggle] = not state.closed[msg.folder_toggle]
			elseif msg.sort_related then
				state.related_ascending = if state.related_sort == msg.sort_related then not state.related_ascending else msg.sort_related ~= "date"
				state.related_sort = msg.sort_related
			elseif msg.action then
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
		local sidebar = {
			ui.text { "Mailboxes", class = "sidebar-label" },
			mailbox_row("All Messages", "mails", total_unread, not mail.mailbox and state.pane == "mail" and mail.filter == "all", { action = "folder", id = 0 }),
		}
		for _, name in { "INBOX", "Archive", "Sent", "Junk", "Trash" } do
			local children = {}
			local unread = 0
			for _, folder in mail.mailboxes or {} do
				if folder.name == name and folder.selectable then
					table.insert(children, { key = "shortcut:" .. tostring(folder.id), name = folder.account, id = folder.id, unread = folder.unread, icon = "at-sign" })
					unread += folder.unread
				end
			end
			if #children > 0 then
				folder_nodes(sidebar, { { key = "role:" .. name, name = name, id = if #children == 1 then children[1].id else nil, unread = unread, children = children } }, state, mail, 0)
			end
		end
		table.insert(sidebar, mailbox_row("Drafts", "file-text", if mail.draft then 1 else 0, state.pane == "compose", "compose"))
		table.insert(sidebar, mailbox_row("Outbox", "send", #(mail.outbox or {}), state.pane == "outbox", "outbox"))
		table.insert(sidebar, ui.text { "Views", class = "sidebar-label" })
		table.insert(sidebar, mailbox_row("Unread", "mail", nil, mail.filter == "unread", { action = "filter", value = "unread" }))
		table.insert(sidebar, mailbox_row("Flagged", "flag", nil, mail.filter == "starred", { action = "filter", value = "starred" }))
		table.insert(sidebar, ui.text { "Sources", class = "sidebar-label" })
		local sources = table.clone(mail.folder_tree or {})
		for _, source in sources do source.icon = "at-sign" end
		folder_nodes(sidebar, sources, state, mail, 0)
		if #sources == 0 then table.insert(sidebar, mailbox_row("Connect an account…", "at-sign", nil, state.pane == "settings", "settings")) end
		local related = table.clone((if state.thread then mail.thread else mail.correspondence) or {})
		table.sort(related, function(a, b)
			local av, bv = a[state.related_sort], b[state.related_sort]
			if type(av) == "string" then av, bv = string.lower(av), string.lower(bv) end
			if av == bv then av, bv = a.id, b.id end
			return if state.related_ascending then av < bv else av > bv
		end)
		local reader
		if state.pane == "settings" then
			-- The capture harness can mount the same window body for layout checks.
			reader = ui.mount "settings"
		elseif state.pane == "outbox" then
			local items = { ui.text { "Outbox", class = "title" }, ui.text { "Unknown delivery is never automatically retried. Check Sent before composing a replacement.", class = "muted" } }
			for _, item in mail.outbox or {} do
				table.insert(items, ui.column { gap = "small", padding = "normal",
					ui.text { item.subject, weight = "bold" }, ui.text { item.state .. " · " .. item.detail },
					if item.state == "queued" then action("Send queued message", "send", { id = item.id }) else nil,
				})
			end
			reader = ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
		elseif state.pane == "compose" then
			local draft = mail.draft
			reader = if draft then ui.column { class = "form", grow = true,
				ui.text { "A new conversation", class = "title" },
				ui.text { "Draft saved locally as you type. Recipients: comma-separated bare email addresses.", class = "muted" },
				ui.input { id = "to-" .. draft.id, value = draft.recipients, placeholder = "To", change = { action = "draft", field = "recipients" }, to = "provider:mail" },
				ui.input { id = "subject-" .. draft.id, value = draft.subject, placeholder = "Subject", change = { action = "draft", field = "subject" }, to = "provider:mail" },
				ui.input { id = "body-" .. draft.id, value = draft.body, placeholder = "Write something…", multiline = true, rows = 18, grow = true, change = { action = "draft", field = "body" }, to = "provider:mail" },
				ui.row { ui.button { label = "Send message", icon = "send", disabled = mail.busy or mail.demo, msg = { action = "queue" }, to = "provider:mail" }, ui.button { label = "Back to mail", msg = "mail" } },
			} else ui.column { class = "form", ui.text { "No open draft. Your queued message is in Outbox." }, ui.button { label = "New message", msg = "compose" }, ui.button { label = "Outbox", msg = "outbox" } }
		else
			reader = if selected then ui.column { grow = true, gap = "none",
				ui.column { class = "reader-head", gap = "none", shrink = false,
					header_field("From", selected.summary.sender .. " <" .. selected.summary.address .. ">"),
					header_field("Subject", selected.summary.subject),
					header_field("Date", selected.summary.date_label),
					header_field("To", selected.summary.recipients),
				},
				ui.row { class = "reader-notice", height = 27, shrink = false, align = "center",
					ui.icon { name = "shield", size = "small" },
					ui.text { if selected.html and state.rich then "HTML · External resources blocked" else "Plain text", size = "small" },
					ui.spacer {}, ui.button { label = if state.rich then "Plain text" else "HTML", small = true, variant = "ghost", msg = "rich", disabled = not selected.html },
				},
				ui.column { grow = true, ui.browser { id = "message-reader", url = if state.rich and mail.html_url ~= "" then mail.html_url else mail.plain_url } },
			} else ui.column { class = "form", ui.text { "No message selected", tone = "muted" } }
		end
		return ui.column { grow = true, gap = "none",
			ui.row { class = "toolbar", height = 46, shrink = false, align = "center",
				tool("chevron-up", "Previous message · ⌘↑", { action = "move", value = "up" }, "provider:mail"),
				tool("chevron-down", "Next message · ⌘↓", { action = "move", value = "down" }, "provider:mail"),
				tool("square-pen", "Compose · ⌘N", "compose"),
				tool("reply", "Reply", "reply", nil, not selected),
				tool("flag", "Toggle flag", { action = "star" }, "provider:mail", not selected),
				tool("mail", "Mark unread", { action = "unread" }, "provider:mail", not selected),
				tool("refresh-cw", "Sync · ⌘R", { action = "sync" }, "provider:mail", mail.busy or mail.demo),
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
			-- Panes own one inset border each. Split handles still resize, but
			-- their overlay hairlines would double borders and disappear behind WebKit.
			ui.hsplit { id = "mailmate-layout-v1", divider = false,
				ui.pane { size = 220, min = 170, max = 340, fixed = true, class = "sidebar mail-pane-right",
					ui.column { grow = true, scroll = true, gap = "none", table.unpack(sidebar) },
				},
				ui.pane { min = 600,
					if state.pane ~= "mail" then reader else ui.vsplit { id = "message-table-and-preview", divider = false,
						ui.pane { size = 380, min = 160, fixed = true, class = "mail-pane-bottom",
							ui.column { grow = true, gap = "none",
								table_header(false, state, mail),
								rows(mail.rows, mail.anchor, false, mail.list_key or "initial", mail.threaded),
								ui.row { height = 26, class = "mail-table-footer", shrink = false, align = "center",
									ui.text { tostring(mail.page_count or 0) .. (if mail.threaded then " threads" else " messages") .. " shown" .. (if mail.search_capped then " · First 1,000 search candidates" else ""), class = "muted" },
									ui.spacer {}, action("First", "first"),
									ui.button { label = "Next", small = true, disabled = not mail.more, msg = { action = "next" }, to = "provider:mail" },
								},
							},
						},
						ui.pane { min = 220,
							ui.hsplit { id = "correspondence-and-reader", divider = false,
								ui.pane { size = 390, min = 260, max = 650, fixed = true, class = "mail-pane-right",
									ui.column { grow = true, gap = "none",
										ui.row { height = 27, class = "correspondence-title", shrink = false, align = "center",
											ui.text { if state.thread then "Reply Thread" else "Correspondence", weight = "medium" },
											ui.spacer {}, ui.text { tostring(#(if state.thread then mail.thread or {} else mail.correspondence or {})), class = "muted" },
											ui.button { icon = "list", small = true, variant = "ghost", tooltip = if state.thread then "Show Correspondence" else "Show reply thread", msg = "thread" },
										},
										table_header(true, state, mail),
										rows(related, selected and selected.summary.id, true, tostring(mail.anchor or 0) .. (if state.thread then "-thread" else "-pairs")),
									},
								},
								ui.pane { min = 320, reader },
							},
						},
					},
				},
			},
			ui.row { class = "status", height = 22, shrink = false, align = "center", ui.text { mail.status or "Opening local archive…" }, ui.spacer {}, ui.text { "⌘N Compose   ⌘R Sync   ⌘↑/↓ Navigate" } },
		}
	end,
}
