local fonts = use "settings-fonts"
local accounts = use "settings-accounts"

local DEFAULTS = { interface_family = ".AppleSystemUIFont", list_family = "JetBrains Mono", font_size = "12", reader_family = "Helvetica", reader_size = "12", threaded = false, mark_read = true, app_icon = "wax-seal" }
local PAGES = { { "appearance", "Appearance", "palette", "#8b5cf6" }, { "fonts", "Fonts", "case-sensitive", "#f28b35" }, { "reading", "Reading & Lists", "list-tree", "#3894ef" }, { "account", "Internet Accounts", "at-sign", "#3478f6" }, { "storage", "Storage", "archive", "#8b929b" } }
local function values(p)
	p = p or {}
	return { interface_family = p.interface_family or DEFAULTS.interface_family, list_family = p.list_family or DEFAULTS.list_family,
		font_size = tostring(p.font_size or 12), reader_family = (p.reader or {}).family or "Helvetica", reader_size = tostring((p.reader or {}).size or 12), threaded = p.threaded == true, mark_read = p.mark_read ~= false, app_icon = p.app_icon or DEFAULTS.app_icon }
end
local function preview(state)
	local size = if state.font_target == "reader_family" then state.values.reader_size else state.values.font_size
	return { to = "provider:mail", msg = { action = "font_preview", family = state.values[state.font_target], size = size } }
end
local function open_fonts(state)
	state.page = "fonts"
	return state, { { to = "provider:mail", msg = { action = "fonts_load" } }, preview(state) }
end
local function row(title, description, control)
	return ui.row { class = "mm-setting-row", align = "center", shrink = false,
		ui.column { grow = true, gap = "none", ui.text { title }, if description then ui.text { description, tone = "muted", size = "small" } else nil }, control,
	}
end
-- A row that opens something: the current value and a chevron, no button.
local function link_row(title, value, msg)
	return ui.row { class = "mm-link-row", align = "center", shrink = false, msg = msg,
		ui.text { title }, ui.spacer {}, ui.text { value, tone = "muted" }, ui.icon { name = "chevron-right", size = "small", tone = "muted" },
	}
end
local function switch(on, msg)
	return ui.row { class = "mm-switch", selected = on, width = 38, height = 22, shrink = false, align = "center", justify = if on then "end" else "start", msg = msg,
		ui.row { class = "mm-switch-knob", width = 18, height = 18, shrink = false } }
end
local function segmented(options, current, to)
	local segments = {}
	for _, option in options do
		table.insert(segments, ui.row { class = "mm-segment", selected = current == option[1], width = 62, height = 24, align = "center", justify = "center", msg = option[1], to = to, ui.text { option[2] } })
	end
	return ui.row { class = "mm-segments", gap = "none", shrink = false, table.unpack(segments) }
end
local function hairline() return ui.row { class = "mm-hairline", height = 1, shrink = false } end
local function group(...) return ui.column { class = "mm-group", shrink = false, ... } end
local function caption(text) return ui.text { text, class = "mm-caption" } end
local function page(...)
	return ui.column { class = "mm-settings-content", grow = true, scroll = true, ... }
end
local ICONS = { { "wax-seal", "Wax Seal" }, { "moon-at", "Moon @" }, { "orbit", "Orbit" } }
local function icon_picker(current)
	local tiles = {}
	for _, icon in ICONS do
		table.insert(tiles, ui.column { class = "mm-icon-choice", align = "center", gap = "none", shrink = false, selected = current == icon[1], msg = { app_icon = icon[1] },
			ui.image { src = "icons/" .. icon[1] .. ".png", width = 76, height = 76 }, ui.text { icon[2], size = "small", class = "mm-icon-label" } })
	end
	return ui.column { class = "mm-icon-picker", shrink = false,
		ui.column { gap = "none", ui.text { "App icon" }, ui.text { "Shows in the Dock the next time Moonmail opens.", tone = "muted", size = "small" } },
		ui.row { class = "mm-icon-tiles", justify = "center", shrink = false, table.unpack(tiles) } }
end
-- Everything but fonts applies at once, as System Settings does. Fonts keep
-- their own Apply, so browsing them doesn't restyle the app under the user.
local function save(state, also)
	local request = table.clone(state.values); request.action = "settings_save"
	state.dirty = true; state.saved = true
	local effects = { { to = "provider:mail", msg = request } }
	if also then table.insert(effects, also) end
	return state, effects
end
local function family(name) return if name == ".AppleSystemUIFont" then "System" else name end
local function action(label, name) return ui.button { label = label, msg = { action = name }, to = "provider:mail" } end

return mod {
	title = "Moonmail Settings",
	window = { size = { width = 980, height = 740 }, min_size = { width = 860, height = 640 }, titlebar = "hidden", traffic_lights = { x = 20, y = 20 } },
	subscribe = { "provider/mail" },
	init = function() return { page = "appearance", values = table.clone(DEFAULTS), dirty = false, saved = false, email = "", font_target = "list_family", font_search = "", add_account = false, provider_list = false, selected_provider = nil, lookup_pending = false, signing_in = false } end,
	update = function(state, msg)
		if msg == "open" then return state, { { to = "app:window", msg = { open = "settings" } } }
		elseif msg == "close" then return state, { { to = "app:window", msg = { close = "settings" } } }
		elseif msg == "defaults" then
			state.values = table.clone(DEFAULTS)
			return save(state, if state.page == "fonts" then preview(state) else nil)
		elseif msg == "reset-font" then
			state.values[state.font_target] = DEFAULTS[state.font_target]
			local size = if state.font_target == "reader_family" then "reader_size" else "font_size"
			state.values[size] = DEFAULTS[size]; state.dirty = true
			return state, { preview(state) }
		elseif msg == "apply" then
			return save(state)
		elseif msg == "add-account" then
			state.add_account = true; state.provider_list = false; state.selected_provider = nil; state.manual_provider = false; state.email = ""; state.lookup_pending = false; state.lookup_note = nil; state.signing_in = false
		elseif msg == "cancel-account" then state.add_account = false; state.lookup_pending = false
		elseif msg == "provider-list" then state.provider_list = not state.provider_list
		elseif msg == "discover-account" then
			if not string.match(state.email, "^[^%s@]+@[^%s@]+%.[^%s@]+$") then state.lookup_note = "Enter a valid email address."; return end
			state.lookup_pending = true; state.lookup_note = nil; state.selected_provider = nil; state.manual_provider = false
			return state, { { to = "provider:mail", msg = { action = "account_discover", email = state.email } } }
		elseif msg == "connect-account" then
			if state.selected_provider ~= "fastmail" or not string.match(state.email, "^[^%s@]+@[^%s@]+%.[^%s@]+$") then return end
			state.signing_in = true
			return state, { { to = "provider:mail", msg = { action = "login", email = state.email } } }
		elseif type(msg) == "table" then
			if msg.topic == "provider/mail" then
				local mail = msg.value or {}
				state.stored = values(mail.preferences)
				if state.saved and mail.status == "Settings saved" and mail.error == "" then state.dirty = false; state.saved = false
				elseif state.saved and mail.error and mail.error ~= "" then state.saved = false
				elseif not state.dirty then state.values = values(mail.preferences) end
				if state.lookup_pending and not mail.discovering then
					local found = mail.discovery
					if found and found.email == state.email then
						state.selected_provider = if found.provider ~= "unknown" then found.provider else nil
						state.provider_list = found.provider == "unknown"
						state.lookup_note = if found.provider == "unknown" then "We couldn’t identify this provider. Choose one below." else "Identified from " .. found.source .. "."
					elseif mail.discovery_error and mail.discovery_error ~= "" then state.lookup_note = mail.discovery_error
					elseif mail.error and mail.error ~= "" then state.lookup_note = mail.error end
					state.lookup_pending = false
				end
				if state.signing_in and not mail.busy then
					state.signing_in = false
					if mail.account == state.email and mail.error == "" then state.add_account = false end
				end
			elseif msg.page then
				-- Leaving Fonts drops a selection that was only being previewed.
				if state.page == "fonts" and msg.page ~= "fonts" and state.dirty and not state.saved and state.stored then state.values = table.clone(state.stored); state.dirty = false end
				state.page = msg.page
				if msg.page == "fonts" then return open_fonts(state) end
			elseif msg.font_target then state.font_target = msg.font_target; state.font_search = ""; return open_fonts(state)
			elseif msg.font_family then state.values[state.font_target] = msg.font_family; state.dirty = true; return state, { preview(state) }
			elseif msg.font_step then
				local reader = state.font_target == "reader_family"
				local field = if reader then "reader_size" else "font_size"
				state.values[field] = tostring(math.clamp((tonumber(state.values[field]) or 12) + msg.font_step, if reader then 8 else 10, if reader then 32 else 16))
				state.dirty = true; return state, { preview(state) }
			elseif msg.provider then state.selected_provider = msg.provider; state.manual_provider = true; state.provider_list = false; state.lookup_note = nil; state.lookup_pending = false
			elseif msg.field then state.values[msg.field] = msg.value; return save(state)
			elseif msg.app_icon then state.values.app_icon = msg.app_icon; return save(state)
			elseif msg.toggle then state.values[msg.toggle] = not state.values[msg.toggle]; return save(state)
			elseif msg.msg == "font-search" then state.font_search = msg.value
			elseif msg.msg == "email" then
				state.email = string.gsub(msg.value, "^%s*(.-)%s*$", "%1")
				if not state.manual_provider then state.selected_provider = nil end
				state.lookup_note = nil; state.lookup_pending = false
			end
		end
	end,
	view = function(state, app)
		local mail = app.mail or {}
		local v = state.values
		local mode = (app.appearance or {}).mode or "system"
		-- The sidebar runs to the window's top edge: its first row is the title
		-- bar the traffic lights sit in.
		local nav = { ui.row { height = 44, shrink = false, drag = true }, ui.row { class = "mm-identity", align = "center", shrink = false,
			ui.image { src = "icons/" .. v.app_icon .. ".png", width = 40, height = 40 },
			ui.column { gap = "none", ui.text { "Moonmail", weight = "semibold" }, ui.text { "Settings", tone = "muted", size = "small" } } } }
		for _, entry in PAGES do
			table.insert(nav, ui.row { class = "mm-settings-nav", height = 30, align = "center", shrink = false, selected = state.page == entry[1], msg = { page = entry[1] },
				ui.row { width = 20, height = 20, shrink = false, justify = "center", align = "center", style = { background = entry[4], radius = 5 }, ui.icon { name = entry[3], size = "small", style = { color = "#ffffff" } } }, ui.text { entry[2] } })
		end
		local title = "Settings"
		for _, entry in PAGES do if entry[1] == state.page then title = entry[2] end end
		local content
		if state.page == "fonts" then content = fonts { state = state, mail = mail }
		elseif state.page == "account" then content = accounts { state = state, mail = mail }
		elseif state.page == "appearance" then
			local function font_row(field, title, size)
				return link_row(title, family(v[field]) .. " · " .. size .. " pt", { font_target = field })
			end
			content = page(
				group(row("Color scheme", nil, segmented({ { "system", "Auto" }, { "light", "Light" }, { "dark", "Dark" } }, mode, "app:appearance")), hairline(), icon_picker(v.app_icon)),
				ui.text { "Fonts", class = "mm-section-title" },
				group(font_row("interface_family", "Interface & mailboxes", v.font_size), hairline(), font_row("list_family", "Messages & Correspondence", v.font_size), hairline(), font_row("reader_family", "Message reader", v.reader_size)),
				ui.row { align = "center", shrink = false, ui.spacer {}, ui.button { label = "Restore Defaults", msg = "defaults" } }
			)
		elseif state.page == "reading" then
			content = page(
				group(
					row("Group messages into threads", "Expand a conversation using its disclosure arrow.", switch(v.threaded, { toggle = "threaded" })),
					hairline(),
					row("Mark messages as read", "When you open a message in the reader.", switch(v.mark_read, { toggle = "mark_read" }))
				),
				caption("Click From, Subject or Date to sort; click again to reverse. Expanded threads show up to 100 matching messages.")
			)
		else
			content = page(
				group(row("Local archive", mail.root or "", action("Rebuild Index", "reindex")), hairline(), row("Font overrides", mail.font_config or "", nil)),
				caption("Your original messages stay on this Mac. Font changes preserve your other stylesheet customizations.")
			)
		end
		return ui.row { class = "mm-settings-window", grow = true, gap = "none",
			ui.column { width = 224, shrink = false, class = "mm-settings-sidebar", table.unpack(nav) },
			ui.column { grow = true, gap = "none",
				ui.row { class = "mm-toolbar", height = 52, shrink = false, align = "center", drag = true, ui.text { title, class = "mm-page-title" } },
				content, if mail.error and mail.error ~= "" and not state.add_account then ui.text { mail.error, class = "error" } else nil },
		}
	end,
	styles = {
		[".mm-settings-window"] = { background = "settings-window" },
		[".mm-settings-sidebar"] = { padding = { 0, 10, 10, 10 }, gap = 2, background = "settings-sidebar", border = "right", border_color = "foreground/10" },
		[".mm-identity"] = { padding = { 2, 6, 14, 6 }, gap = 10 },
		[".mm-settings-nav"] = { padding = { 0, 6 }, gap = 8, radius = 8 },
		[".mm-settings-nav:hover"] = { background = "foreground/6" },
		[".mm-settings-nav:selected"] = { background = "foreground/10", weight = "semibold" },
		[".mm-settings-content"] = { padding = { 16, 24, 24, 24 }, gap = 10 },
		[".mm-toolbar"] = { padding = { 0, 24 }, border = "bottom", border_color = "foreground/8" },
		[".mm-page-title"] = { size = "large", weight = "bold" },
		[".mm-section-title"] = { weight = "semibold", padding = { 12, 4, 0, 4 } },
		[".mm-caption"] = { size = "small", color = "muted-foreground", padding = { 0, 4 } },
		[".mm-group"] = { background = "settings-card", radius = 12, gap = 0, clip = true },
		[".mm-hairline"] = { background = "foreground/8", margin = { 0, 0, 0, 14 } },
		[".mm-setting-row"] = { padding = { 10, 14 }, gap = 16 },
		[".mm-link-row"] = { padding = { 12, 14 }, gap = 6 },
		[".mm-link-row:hover"] = { background = "foreground/4" },
		[".mm-switch"] = { padding = 2, radius = 11, background = "foreground/16", transition = "fade 120ms" },
		[".mm-switch:selected"] = { background = "accent" },
		[".mm-switch-knob"] = { background = "#ffffff", radius = 9, shadow = "small" },
		[".mm-segments"] = { padding = 2, radius = 9, background = "foreground/7" },
		[".mm-segment"] = { radius = 7 },
		[".mm-segment:selected"] = { background = "settings-control", shadow = "small", weight = "medium" },
		[".mm-icon-picker"] = { padding = { 12, 14, 14, 14 }, gap = 10 },
		[".mm-icon-tiles"] = { gap = 26 },
		[".mm-icon-choice"] = { padding = { 6, 12, 8, 12 }, radius = 14 },
		[".mm-icon-choice:hover"] = { background = "foreground/4" },
		[".mm-icon-choice:selected"] = { background = "accent/12", color = "accent" },
		[".mm-icon-label"] = { padding = { 2, 0, 0, 0 } },
	},
}
