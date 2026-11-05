local function build(props)
	local config, email = props.config or {}, props.email or ""
	local rows = {}
	for _, kind in { "imap", "smtp" } do
		local host, port, username, tls = kind .. "_host", kind .. "_port", kind .. "_username", kind .. "_tls"
		table.insert(rows, ui.column { gap = "small", shrink = false,
			ui.text { if kind == "imap" then "Incoming · IMAP" else "Outgoing · SMTP", weight = "semibold", size = "small" },
			ui.row { align = "center", gap = "small",
				ui.input { id = "server-" .. host, value = config[host] or "", placeholder = if kind == "imap" then "imap.example.com" else "smtp.example.com", grow = true, change = { account_field = host }, to = "settings" },
				ui.input { id = "server-" .. port, value = config[port] or "", placeholder = "Port", width = 66, change = { account_field = port }, to = "settings" },
				ui.button { label = "TLS", small = true, variant = if config[tls] == "implicit" then "primary" else "ghost", msg = { account_tls = kind, value = "implicit" }, to = "settings" },
				ui.button { label = "STARTTLS", small = true, variant = if config[tls] == "starttls" then "primary" else "ghost", msg = { account_tls = kind, value = "starttls" }, to = "settings" },
			},
			ui.input { id = "server-" .. username, value = config[username] or "", placeholder = "Username · " .. email, change = { account_field = username }, to = "settings" },
		})
	end
	table.insert(rows, ui.column { gap = "small", shrink = false,
		ui.text { "Sent messages", weight = "semibold", size = "small" },
		ui.row {
			ui.button { label = "Provider saves a copy", small = true, variant = if config.sent_copy == "server" then "primary" else "ghost", msg = { account_field = "sent_copy", value = "server" }, to = "settings" },
			ui.button { label = "Save through IMAP", small = true, variant = if config.sent_copy == "append" then "primary" else "ghost", msg = { account_field = "sent_copy", value = "append" }, to = "settings" },
		},
	})
	return ui.column { gap = "normal", shrink = false, table.unpack(rows) }
end

return component { title = "Mail server settings", example = {}, build = build }
