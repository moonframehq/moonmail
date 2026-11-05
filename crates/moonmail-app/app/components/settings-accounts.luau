local dialog = use "dialog"
local servers = use "account-server-fields"
local PROVIDERS = {
	{ id = "fastmail", name = "Fastmail", icon = "mail", color = "#df643c", detail = "Browser sign-in or app password", oauth = true },
	{ id = "google", name = "Google · Gmail", icon = "mail", color = "#4285f4", detail = "Gmail and Google Workspace", oauth = true, client = true },
	{ id = "icloud", name = "iCloud", icon = "cloud", color = "#369bf4", detail = "Connect with an app-specific password" },
	{ id = "microsoft", name = "Microsoft 365 / Outlook", icon = "building-2", color = "#0078d4", detail = "OAuth · requires enabled IMAP and SMTP AUTH", oauth = true, client = true },
	{ id = "yahoo", name = "Yahoo", icon = "mail", color = "#6001d2", detail = "Connect with an app password" },
	{ id = "aol", name = "AOL", icon = "mail", color = "#555555", detail = "Connect with an app password" },
	{ id = "other", name = "Other Mail Account", icon = "at-sign", color = "#868d98", detail = "Your IMAP and SMTP servers · TLS or STARTTLS" },
}
local function find_provider(id)
	for _, provider in PROVIDERS do if provider.id == id then return provider end end
	return PROVIDERS[#PROVIDERS]
end
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
				ui.column { grow = true, gap = "none", ui.text { provider.name, weight = "medium" }, ui.text { provider.detail, size = "small", tone = "muted" } },
				ui.icon { name = if state.selected_provider == provider.id then "check" else "chevron-right", size = "small", tone = "muted" },
			})
		end
		local busy = mail.busy or state.signing_in
		local can_connect = selected and valid and not busy and not mail.demo
		local accounts = {}
		for _, account in mail.account_details or {} do
			local provider = find_provider(account.provider)
			table.insert(accounts, ui.row { class = "mm-account-row", align = "center",
				badge(provider.icon, provider.color), ui.column { grow = true, gap = "small", ui.text { provider.name, weight = "semibold" }, ui.text { account.email, tone = "muted" }, ui.text { if account.email == mail.account then "Default for new messages" else "Available in the From menu", tone = "muted", size = "small" } },
				ui.column { gap = "small", align = "end",
					ui.row { ui.button { label = "Check", small = true, msg = { action = "account_check", account = account.email }, to = "provider:mail", disabled = busy or mail.demo }, ui.button { label = "Reconnect…", small = true, msg = { edit_account = account }, to = "settings", disabled = busy or mail.demo } },
					ui.button { label = "Disconnect", small = true, variant = "ghost", msg = { action = "disconnect", account = account.email }, to = "provider:mail", disabled = busy or mail.demo },
				},
			})
		end
		return ui.column { class = "mm-settings-content", grow = true, scroll = true,
			ui.column { class = "mm-group", shrink = false,
				if #accounts > 0 then ui.column { gap = "none", table.unpack(accounts) } else ui.column { class = "mm-account-empty", align = "center", badge("at-sign", "#3478f6"), ui.text { "Add your first account", weight = "semibold", size = "large" }, ui.text { "Keep your mail close. Your messages are stored locally.", tone = "muted" } },
			},
			ui.row { justify = "end", ui.button { label = "Add Account…", msg = "add-account", to = "settings", disabled = busy or mail.demo } },
			ui.text { "Choose a sending account in the composer’s From menu. Passwords and OAuth tokens stay in macOS Keychain.", class = "mm-caption" },
			if mail.status and mail.status ~= "" then ui.text { mail.status, tone = "muted", size = "small" } else nil,
			dialog { open = state.add_account, dismiss = "cancel-account", to = "settings", width = 620, height = if state.server_settings then 590 elseif state.provider_list then 540 elseif selected then 440 else 280, title = if state.editing_account then "Reconnect account" else "Add Account", hint = false, close = false,
				buttons = { { label = if busy then "Close" else "Cancel", msg = "cancel-account" } },
				ui.column { class = "mm-add-account", gap = "small",
					ui.text { "Enter your email address", weight = "semibold" },
					ui.input { id = "account-email", value = state.email, placeholder = "user@example.com", change = "email", focus = state.add_account, submit = "discover-account", to = "settings", disabled = state.editing_account or busy },
					ui.row { align = "center",
						ui.column { grow = true, gap = "none", ui.text { "Sign in to your mail account provider", tone = "muted", size = "small" }, ui.row { msg = "provider-list", to = "settings", ui.text { if state.provider_list then "Hide provider list" else "or choose from a list…", tone = "accent", size = "small" } } },
						ui.button { label = if state.lookup_pending or mail.discovering then "Looking up…" else "Continue", variant = "primary", msg = "discover-account", to = "settings", disabled = not valid or mail.discovering or busy },
					},
					if state.lookup_note then ui.text { state.lookup_note, tone = "muted", size = "small" } else nil,
					if selected then ui.column { class = "mm-account-result",
						ui.row { align = "center", badge(selected.icon, selected.color), ui.text { selected.name, weight = "semibold", size = "large" } },
						ui.text { if selected.id == "google" then "Read and reply through Gmail’s IMAP and SMTP services. Labels appear as mailboxes; All Mail is your archive." elseif selected.id == "microsoft" then "For accounts with IMAP and SMTP AUTH enabled. Uses delegated browser sign-in, not Exchange/EWS." elseif selected.id == "other" then "Enter your server settings below. Moonmail verifies IMAP and SMTP before connecting." else "Use browser sign-in where available, or enter an app password in a native secure dialog. Both mail services are checked before connecting.", tone = "muted", size = "small" },
						if selected.client then ui.column { gap = "small",
							ui.text { "Browser sign-in needs a registered desktop OAuth client. Import it once for this Mac.", tone = "muted", size = "small" },
							ui.row { ui.button { label = "Import OAuth client…", small = true, msg = "import-oauth-client", to = "settings", disabled = busy or mail.demo } },
						} else nil,
						ui.row { gap = "small", justify = "end",
							if selected.id ~= "microsoft" then ui.button { label = if selected.id == "other" then "Connect with password…" else "Use app password…", variant = if selected.oauth then "ghost" else "primary", msg = "connect-password", to = "settings", disabled = not can_connect } else nil,
							if selected.oauth then ui.button { label = "Sign in with " .. (if selected.id == "google" then "Google" elseif selected.id == "microsoft" then "Microsoft" else "Fastmail"), variant = "primary", msg = "connect-account", to = "settings", disabled = not can_connect } else nil,
						},
						ui.button { label = if state.server_settings then "Hide server settings" else "Server settings…", small = true, variant = "ghost", msg = "server-settings", to = "settings", disabled = busy },
						if state.server_settings then servers { config = state.server_config, email = state.email } else nil,
					} else nil,
					if state.provider_list then ui.column { gap = "small", ui.text { "Choose your provider", weight = "semibold" }, ui.column { class = "mm-group", height = 260, scroll = true, table.unpack(providers) } } else nil,
					if mail.demo then ui.text { "You’re in the offline demo. Run Moonmail with just run to connect an account.", tone = "muted", size = "small" } else nil,
					if not mail.demo and mail.status and mail.status ~= "" then ui.text { mail.status, tone = "muted", size = "small" } else nil,
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
