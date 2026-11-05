local LABELS = { sender = "From", subject = "Subject", date = "Date", recipients = "To" }

return component { title = "Message columns", example = { mail = {} }, build = function(props)
	local mail = props.mail
	local layout = (mail.preferences or {}).columns or {}
	local function button(label, operation, column, disabled, width)
		return ui.button { label = label, small = true, disabled = disabled, msg = { action = "columns_edit", base = mail.columns_key, operation = operation, field = column and column.field, width = width }, to = "provider:mail" }
	end
	local items = { ui.text { "Message columns", weight = "semibold" }, ui.text { "Changes apply immediately to the main list. Subject fills the remaining space. To is display-only; From, Subject and Date support sorting.", tone = "muted", size = "small" } }
	for index, column in layout do
		table.insert(items, ui.column { padding = "normal", shrink = false,
			ui.row { align = "center", ui.column { grow = true, ui.text { LABELS[column.field], weight = "semibold" } },
				button(if column.visible then "Hide" else "Show", "toggle", column, column.field == "subject"),
				button("←", "left", column, index == 1), button("→", "right", column, index == #layout),
			},
			if column.field ~= "subject" then ui.row { align = "center", ui.text { tostring(column.width) .. " pt", tone = "muted" }, button("−", "width", column, column.width <= 80, math.max(80, column.width - 20)), button("+", "width", column, column.width >= 300, math.min(300, column.width + 20)) } else ui.text { "Always visible · flexible width", tone = "muted", size = "small" },
		})
	end
	table.insert(items, button("Restore column defaults", "reset"))
	return ui.column { shrink = false, table.unpack(items) }
end }
