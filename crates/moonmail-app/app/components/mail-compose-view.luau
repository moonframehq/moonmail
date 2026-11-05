local from_menu = use "mail-from-menu"

local function field(label, name, draft, trailing)
	return ui.row { class = "compose-field", height = 40, shrink = false, align = "center", gap = "small",
		ui.column { width = 48, shrink = false, ui.text { label, tone = "muted", size = "small" } },
		ui.input { id = "compose-" .. name .. "-" .. draft.id, class = if name == "subject" then "compose-input compose-subject" else "compose-input", value = draft[name], grow = true, placeholder = if name == "subject" then "Subject" else "Add recipients", change = { action = "draft", id = draft.id, field = name }, to = "provider:mail" },
		trailing,
	}
end

local function tool(icon, tip, msg, disabled)
	return ui.button { icon = icon, tooltip = tip, small = true, variant = "ghost", msg = msg, disabled = disabled }
end

local function build(props)
	local mail, state = props.mail, props.state
	local draft = mail.draft
	if not draft then
		return ui.column { grow = true, gap = "none",
			ui.row { height = 60, drag = true, shrink = false },
			ui.column { class = "form", grow = true, justify = "center", align = "center",
				ui.text { "This draft is no longer open.", tone = "muted" },
				if mail.error and mail.error ~= "" then ui.text { mail.error, class = "error" } else nil,
				ui.button { label = "Close", msg = "close" },
			},
		}
	end
	local send_disabled = mail.busy or mail.demo
	local attachments = {}
	for _, item in draft.attachments or {} do
		table.insert(attachments, ui.row { class = "compose-attachment", height = 38, shrink = false, align = "center",
			ui.icon { name = "file-text", tone = "accent" },
			ui.column { grow = true, ui.text { item.name, truncate = true } },
			ui.text { if item.bytes < 1024 then tostring(item.bytes) .. " B" elseif item.bytes < 1048576 then string.format("%.1f KiB", item.bytes / 1024) else string.format("%.1f MiB", item.bytes / 1048576), size = "small", tone = "muted" },
			tool("close", "Remove attachment", { action = "draft_detach", id = draft.id, part = item.id }),
		})
	end
	return ui.column { class = "compose-window", grow = true, gap = "none",
		ui.row { class = "compose-toolbar", height = 62, shrink = false, align = "center", drag = true,
			ui.row { width = 78, shrink = false },
			ui.column { grow = true, gap = "none", ui.text { if draft.reply_id ~= "" then "Reply" else "New message", weight = "semibold" }, ui.text { "Saved locally as you type", size = "small", tone = "muted" } },
			tool("paperclip", "Attach files", { attach_draft = draft.id }),
			ui.row { class = "compose-send-group", height = 34, shrink = false, align = "center", gap = "none",
				ui.row { class = if send_disabled then "compose-send-action compose-send-disabled" else "compose-send-action", width = 84, height = 32, shrink = false, align = "center", justify = "center", gap = "small",
					state = if send_disabled then "disabled" else nil,
					tooltip = if mail.demo then "Sending is unavailable in demo mode" elseif mail.busy then "Wait for the current operation to finish" else "Send message",
					msg = if send_disabled then nil else { action = "queue", id = draft.id },
					ui.icon { name = "send", size = "large" }, ui.text { "Send", weight = "semibold" },
				},
				ui.row { class = "compose-send-divider", width = 1, height = 18, shrink = false },
				ui.row { class = "compose-send-options", width = 32, height = 32, shrink = false, align = "center", justify = "center", selected = state.send_options,
					tooltip = "Send options", msg = "toggle-send", ui.icon { name = "chevron-down", size = "small" },
				},
			},
		},
		if mail.error and mail.error ~= "" then ui.text { mail.error, class = "error" } else nil,
		if state.send_options then ui.row { class = "compose-options", justify = "end", shrink = false, ui.button { label = "Queue only", small = true, msg = { action = "queue_only", id = draft.id } } } else nil,
		ui.column { class = "compose-envelope", shrink = false, gap = "none",
			ui.row { class = "compose-field", height = 40, shrink = false, align = "center", gap = "small",
				ui.column { width = 48, shrink = false, ui.text { "From", tone = "muted", size = "small" } },
				from_menu { mail = mail, state = state, draft = draft },
			},
			field("To", "recipients", draft, ui.button { label = "Cc / Bcc", variant = "ghost", small = true, msg = "toggle-cc" }),
			if state.cc or draft.cc ~= "" or draft.bcc ~= "" then field("Cc", "cc", draft) else nil,
			if state.cc or draft.cc ~= "" or draft.bcc ~= "" then field("Bcc", "bcc", draft) else nil,
			field("Subject", "subject", draft),
		},
		ui.column { class = "compose-writing", grow = true,
			ui.input { id = "compose-body-" .. draft.id, class = "compose-body", value = draft.body, placeholder = "Write your message…", multiline = true, rows = 6, grow = true, change = { action = "draft", id = draft.id, field = "body" }, to = "provider:mail" },
		},
		if #attachments > 0 then ui.column { class = "compose-attachments", height = math.min(#attachments * 42 + 12, 96), shrink = false, scroll = true, gap = "small", table.unpack(attachments) } else nil,
		ui.row { class = "compose-footer", height = 45, shrink = false, align = "center",
			tool("paperclip", "Attach files", { attach_draft = draft.id }),
			tool("square-pen", "Insert signature", { action = "draft_signature", id = draft.id }, not draft.identity or draft.identity.signature == ""),
			ui.text { "Plain text", size = "small", tone = "muted" }, ui.spacer {},
			ui.button { label = "Save & close", small = true, variant = "ghost", msg = { action = "draft_close", id = draft.id } },
			tool("trash-2", "Discard draft", { action = "draft_discard", id = draft.id }),
		},
	}
end

return component { title = "Mail composer", example = { mail = {}, state = {} }, build = build }
