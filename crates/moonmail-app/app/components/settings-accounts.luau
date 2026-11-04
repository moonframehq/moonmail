local dialog = use "dialog"
local PROVIDERS = {
	{ id = "fastmail", name = "Fastmail", icon = "mail", color = "#df643c", detail = "Sign in securely in your browser" },
	{ id = "icloud", name = "iCloud", icon = "cloud", color = "#369bf4" },
	{ id = "google", name = "Google", icon = "mail", color = "#4285f4" },
	{ id = "microsoft", name = "Microsoft Exchange", icon = "building-2", color = "#0078d4" },
	{ id = "yahoo", name = "Yahoo", icon = "mail", color = "#6001d2" },
	{ id = "aol", name = "AOL", icon = "mail", color = "#555555" },
	{ id = "other", name = "Other Mail Account", icon = "at-sign", color = "#868d98" },
}
local function badge(icon, color)
	return ui.row { width = 34, height = 34, shrink = false, align = "center", justify = "center", style = { background = color, radius = 8 }, ui.icon { name = icon, style = { color = "#ffffff" } } }
end

return component {
	title = "Internet Accounts",
	example = { state = { email = "", add_account = false }, mail = {} },
	build = function(props)
		local state, mail = props.state, props.mail
		local valid = string.match(state.email, "^[^%s@]+@[^%s@]+%.[^%s@]+$") ~= nil
		local selected
		local providers = {}
		for _, provider in PROVIDERS do
			if state.selected_provider == provider.id then selected = provider end
			table.insert(providers, ui.row { class = "mm-provider-row", height = 52, align = "center", selected = state.selected_provider == provider.id, msg = { provider = provider.id }, to = "settings",
				badge(provider.icon, provider.color),
				ui.column { grow = true, gap = "none", ui.text { provider.name, weight = "medium" }, ui.text { provider.detail or "Not yet supported", size = "small", tone = "muted" } },
				ui.icon { name = if state.selected_provider == provider.id then "check" else "chevron-right", size = "small", tone = "muted" },
			})
		end
		local busy = mail.busy or state.signing_in
		local can_connect = selected and selected.id == "fastmail" and valid and not busy and not mail.account and not mail.demo
		return ui.column { class = "mm-settings-content", grow = true, scroll = true,
			ui.column { class = "mm-group", shrink = false,
				if mail.account then ui.row { class = "mm-account-row", align = "center",
					badge("mail", "#df643c"), ui.column { grow = true, gap = "small", ui.text { "Fastmail", weight = "semibold" }, ui.text { mail.account, tone = "muted" }, ui.text { "Mail · Manual sync", tone = "muted", size = "small" } },
					ui.button { label = "Disconnect", msg = { action = "disconnect" }, to = "provider:mail", disabled = busy },
				} else ui.column { class = "mm-account-empty", align = "center", badge("at-sign", "#3478f6"), ui.text { "Add your first account", weight = "semibold", size = "large" }, ui.text { "Keep your mail close. Your messages are stored locally.", tone = "muted" } },
			},
			ui.row { justify = "end", ui.button { label = "Add Account…", msg = "add-account", to = "settings", disabled = busy or mail.account ~= nil } },
			ui.text { if mail.account then "Moonmail currently supports one connected account." else "Fastmail is available now. More providers are on the way.", class = "mm-caption" },
			if busy then ui.text { "Waiting for sign-in in your browser…", tone = "muted" } else nil,
			dialog { open = state.add_account, dismiss = "cancel-account", to = "settings", width = 540, height = if state.provider_list then 430 else 300, title = "Add Account", hint = false, close = false,
				buttons = { { label = if busy then "Close" else "Cancel", msg = "cancel-account" } },
				ui.column { class = "mm-add-account", gap = "small",
					ui.text { "Enter your email address", weight = "semibold" },
					ui.input { id = "account-email", value = state.email, placeholder = "user@example.com", change = "email", focus = state.add_account, submit = "discover-account", to = "settings" },
					ui.row { align = "center",
						ui.column { grow = true, gap = "none", ui.text { "Sign in to your mail account provider", tone = "muted", size = "small" }, ui.row { msg = "provider-list", to = "settings", ui.text { if state.provider_list then "Hide provider list" else "or choose from a list…", tone = "accent", size = "small" } } },
						ui.button { label = if state.lookup_pending or mail.discovering then "Looking up…" else "Continue", variant = "primary", msg = "discover-account", to = "settings", disabled = not valid or mail.discovering or busy },
					},
					if state.lookup_note then ui.text { state.lookup_note, tone = "muted", size = "small" } else nil,
					if selected then ui.column { class = "mm-account-result",
						ui.row { align = "center", badge(selected.icon, selected.color), ui.text { selected.name, weight = "semibold", size = "large" } },
						ui.text { if selected.id == "fastmail" then "Continue in your browser to authorize Moonmail. Credentials are stored in macOS Keychain." else "Sign-in for this provider isn’t available in Moonmail yet.", tone = "muted", size = "small" },
						if selected.id == "fastmail" then ui.row { justify = "end", ui.button { label = if busy then "Waiting for browser…" else "Sign In with Fastmail", variant = "primary", msg = "connect-account", to = "settings", disabled = not can_connect } } else nil,
					} else nil,
					if state.provider_list then ui.column { gap = "small", ui.text { "Choose your provider", weight = "semibold" }, ui.column { class = "mm-group", height = 260, scroll = true, table.unpack(providers) } } else nil,
					if mail.demo then ui.text { "You’re in the offline demo. Run Moonmail with just run to connect an account.", tone = "muted", size = "small" } else nil,
					if mail.error and mail.error ~= "" then ui.text { mail.error, tone = "danger", size = "small" } else nil,
				},
			},
		}
	end,
	styles = {
		[".mm-account-row"] = { padding = { 12, 14 }, gap = 12 },
		[".mm-account-empty"] = { padding = { 36, 20 }, gap = 14 },
		[".mm-add-account"] = { padding = { 8, 10, 14, 10 }, gap = 16 },
		[".mm-provider-row"] = { padding = { 8, 12 }, gap = 12, border = "bottom", border_color = "foreground/8" },
		[".mm-provider-row:hover"] = { background = "foreground/5" },
		[".mm-provider-row:selected"] = { background = "accent/10" },
		[".mm-account-result"] = { padding = 16, gap = 12, background = "accent/6", radius = 9 },
	},
}
