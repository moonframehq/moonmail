local function build(props)
	local mail, state, draft = props.mail, props.state, props.draft
	local sender = if draft.identity then (if draft.identity.name ~= "" then draft.identity.name .. " <" .. draft.identity.address .. ">" else draft.identity.address) else draft.account
	local choices = mail.from_accounts or {}
	local rows = {}
	for index, choice in choices do
		local highlighted = state.from_index == index or (state.from_index == nil and choice.current)
		table.insert(rows, ui.row { class = "from-menu-item", height = 34, shrink = false, align = "center", gap = "small", selected = highlighted,
			tooltip = "Send using " .. choice.account,
			msg = { action = "draft_account", id = draft.id, account = choice.account, identity = choice.identity },
			ui.row { width = 17, shrink = false, justify = "center", if choice.current then ui.icon { name = "check", size = "small", style = { color = if highlighted then "#ffffff" else "accent" } } else nil },
			ui.column { grow = true, ui.text { choice.label, truncate = true, style = { color = if highlighted then "#ffffff" else "foreground" } } },
		})
	end
	return ui.popover { class = "from-menu-popover", open = state.from_open == true, dismiss = "close-from", side = "bottom", align = "start", gap = 4, width = 430,
		keys = { escape = "close-from", up = { from_step = -1, id = draft.id }, down = { from_step = 1, id = draft.id }, enter = { from_choose = true, id = draft.id } },
		content = ui.column { class = "from-menu", gap = "none",
			if #rows > 0 then ui.column { height = math.min(#rows * 34, 272), scroll = true, scroll_to = if state.from_index and state.from_index <= #rows then state.from_index else nil, gap = "none", table.unpack(rows) } else ui.row { height = 34, padding = "small", ui.text { "No connected accounts", tone = "muted" } },
			ui.row { class = "from-menu-divider", height = 1, shrink = false },
			ui.row { class = "from-menu-item", height = 34, shrink = false, align = "center", selected = state.from_index == #choices + 1, msg = "manage-accounts",
				ui.row { width = 17, shrink = false }, ui.text { "Manage accounts…", style = { color = if state.from_index == #choices + 1 then "#ffffff" else "foreground" } },
			},
		},
		ui.row { class = "from-menu-trigger", align = "center", gap = "small", selected = state.from_open == true, msg = "toggle-from", tooltip = "Choose the account to send from",
			ui.text { if sender ~= "" then sender else "Choose an account", truncate = true }, ui.icon { name = "chevron-down", size = "small", tone = "muted" },
		},
	}
end

return component { title = "From account menu", example = { mail = {}, state = {}, draft = { account = "you@example.com", id = "example" } }, build = build }
