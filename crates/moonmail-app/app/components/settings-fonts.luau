local TARGETS = { { "interface_family", "Interface" }, { "list_family", "Message Lists" }, { "reader_family", "Reader" } }
local function label(name) return if name == ".AppleSystemUIFont" then "System Font" else name end

return component {
	title = "Font Browser",
	example = { state = { font_target = "list_family", font_search = "", values = { list_family = "Menlo", font_size = "12" } }, mail = { font_families = { "Menlo", "Monaco" } } },
	build = function(props)
		local state, mail = props.state, props.mail
		local selected = state.values[state.font_target]
		local size = if state.font_target == "reader_family" then state.values.reader_size else state.values.font_size
		local families = {}
		for _, name in mail.font_families or {} do
			if string.find(string.lower(label(name)), string.lower(state.font_search), 1, true) then table.insert(families, name) end
		end
		local targets = {}
		for _, target in TARGETS do
			table.insert(targets, ui.row { class = "mm-font-target", selected = target[1] == state.font_target, msg = { font_target = target[1] }, to = "settings", align = "center", justify = "center", grow = true, height = 32, ui.text { target[2], weight = "medium" } })
		end
		return ui.column { grow = true, gap = "none",
			ui.column { class = "mm-font-header", shrink = false,
				ui.text { "Choose a font for each part of Moonmail.", tone = "muted" },
				ui.row { class = "mm-font-targets", gap = "none", table.unpack(targets) },
			},
			ui.row { grow = true, gap = "none",
				ui.column { width = 240, shrink = false, class = "mm-font-families", gap = "none",
					ui.column { style = { padding = 12 }, shrink = false, ui.input { id = "font-search", value = state.font_search, placeholder = "Search fonts", change = "font-search", to = "settings" } },
					if state.font_search == "" then ui.row { class = "mm-font-family", selected = true, height = 36, shrink = false, align = "center",
						ui.row { width = 28, height = 26, align = "center", justify = "center", class = "mm-font-aa", ui.text { "Aa", weight = "medium" } },
						ui.column { grow = true, ui.text { label(selected), truncate = true } }, ui.icon { name = "check", size = "small", tone = "accent" },
					} else nil,
					ui.text { tostring(#families) .. " font families", class = "mm-font-count" },
					ui.list { id = "font-families-" .. state.font_search, count = #families, row_height = 36, grow = true, row = function(index)
						local name = families[index]
						return ui.row { height = 36, class = "mm-font-family", selected = name == selected, align = "center", msg = { font_family = name }, to = "settings",
							ui.row { width = 28, height = 26, align = "center", justify = "center", class = "mm-font-aa", ui.text { "Aa", weight = "medium" } },
							ui.column { grow = true, ui.text { label(name), truncate = true } },
							if name == selected then ui.icon { name = "check", size = "small", tone = "accent" } else nil,
						}
					end },
					if #families == 0 then ui.text { "No matching fonts", class = "mm-font-count" } else nil,
				},
				ui.column { grow = true, gap = "none",
					ui.column { class = "mm-font-preview-header", shrink = false,
						ui.text { label(selected), size = "large", weight = "semibold", truncate = true },
						ui.row { align = "center", ui.text { "Regular", tone = "muted" }, ui.spacer {},
							ui.button { label = "−", msg = { font_step = -1 }, to = "settings", disabled = tonumber(size) <= (if state.font_target == "reader_family" then 8 else 10) },
							ui.text { size .. " pt", weight = "medium" },
							ui.button { label = "+", msg = { font_step = 1 }, to = "settings", disabled = tonumber(size) >= (if state.font_target == "reader_family" then 32 else 16) },
						},
					},
					ui.column { grow = true,
						if mail.font_preview_url and mail.font_preview_url ~= "" then ui.browser { id = "font-preview", url = mail.font_preview_url } else ui.text { "Select a font to preview it.", tone = "muted" },
					},
				},
			},
			ui.row { class = "mm-font-bottom", align = "center", shrink = false,
				ui.column { grow = true, ui.text { if state.font_target == "reader_family" then "Reader size · 8–32 pt" else "Interface & list size · 10–16 pt", tone = "muted", size = "small" }, ui.text { "Apply to save your selection.", tone = "muted", size = "small" } },
				ui.button { label = "Reset Font", msg = "reset-font", to = "settings" },
				ui.button { label = "Apply", variant = "primary", disabled = not state.dirty, msg = "apply", to = "settings" },
			},
		}
	end,
	styles = {
		[".mm-font-header"] = { padding = { 16, 24, 14, 24 }, gap = 10 },
		[".mm-font-targets"] = { padding = 3, radius = 8, background = "foreground/5" },
		[".mm-font-target"] = { radius = 6 },
		[".mm-font-target:selected"] = { background = "settings-control", shadow = "small" },
		[".mm-font-families"] = { background = "settings-window", border = "right", border_color = "foreground/8" },
		[".mm-font-count"] = { padding = { 4, 14, 12, 14 }, size = "small", color = "muted-foreground" },
		[".mm-font-family"] = { padding = { 0, 10 }, gap = 9, radius = 6, margin = { 0, 6 } },
		[".mm-font-family:selected"] = { background = "accent/14", color = "accent" },
		[".mm-font-family:hover"] = { background = "foreground/5" },
		[".mm-font-aa"] = { background = "background", radius = 5 },
		[".mm-font-preview-header"] = { padding = 18, gap = 12, border = "bottom", border_color = "foreground/8" },
		[".mm-font-bottom"] = { padding = { 12, 20 }, border = "top", border_color = "foreground/8", gap = 12 },
	},
}
