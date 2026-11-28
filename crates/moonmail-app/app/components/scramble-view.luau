-- Build a presentation-only tree. Copy layout properties explicitly so new
-- widgets, native surfaces, hover text and edit buffers are hidden by default.
local layout = {
	"class", "style", "grow", "shrink", "width", "height", "min", "max", "size",
	"fixed", "visible", "divider", "align", "justify", "gap", "padding", "scroll",
	"drag", "selected", "tone", "weight", "truncate",
}
local containers = { row = true, column = true, hsplit = true, vsplit = true, pane = true, spacer = true }
local labels = {}
for _, text in {
	"Moonmail", "All Messages", "Inbox", "INBOX", "Archive", "Sent", "Drafts", "Junk", "Trash", "Outbox",
	"Mailboxes", "Accounts", "Sources", "Favorites", "Views", "Unread", "Flagged", "People", "Saved Searches",
	"Mail Operations", "Move & Folders", "Correspondence", "Reply Thread", "From", "Subject", "To", "Date",
	"From:", "Subject:", "To:", "Date:", "Received:", "Sort date:", "Cc", "Bcc", "HTML", "Plain text",
	"First", "Previous", "Next", "Compose", "Settings", "Appearance", "Reading", "Fonts", "Sync",
	"Search", "Search messages", "New message", "Reply", "Send", "Attachments…", "Links…", "Embedded images",
	"Save .eml…", "●", "⚑", "✓", "□", "☑",
} do labels[text] = true end

local function mask(value)
	if type(value) ~= "string" or value == "" then return "" end
	if labels[value] then return value end
	-- A bounded, approximate width also covers non-Latin text, URLs and digits.
	return string.rep("•", math.clamp(utf8.len(value) or #value, 3, 28))
end

local function scrub(node)
	local kind = node.type
	local copy = {}
	for _, key in layout do copy[key] = node[key] end
	if containers[kind] then
		copy.type = kind
		if kind == "hsplit" or kind == "vsplit" then copy.id = node.id end
		for index, child in node do
			if type(index) == "number" and type(child) == "table" then copy[index] = scrub(child) end
		end
	elseif kind == "input" then
		return ui.column { width = node.width, grow = node.grow, justify = "center",
			ui.text { mask(if node.value and node.value ~= "" then node.value else node.placeholder), truncate = not node.multiline },
		}
	elseif kind == "markdown" then
		return ui.text { mask(node.text) }
	elseif kind == "text" then
		copy.type = "text"
		copy.text = mask(node.text)
	elseif kind == "icon" then
		copy.type = "icon"
		copy.name = node.name
	elseif kind == "button" then
		copy.type = "button"
		copy.label = mask(node.label)
		copy.icon = node.icon
		copy.small = node.small
		copy.variant = node.variant
		copy.disabled = node.disabled
	elseif kind == "list" then
		copy.type = "list"
		copy.id = "scrambled-" .. node.id
		copy.count = node.count
		copy.row_height = node.row_height
		copy.row = function(index) return scrub(node.row(index)) end
	elseif kind == "popover" then
		return scrub(if node.open then node.content else node[1])
	elseif kind == "mount" and node.mod == "mail-agent" then
		return ui.column { grow = true, padding = "normal", ui.text { "Assistant", weight = "semibold" }, ui.text { "Conversation hidden", tone = "muted" } }
	elseif kind == "browser" or kind == "view" then
		return ui.column { grow = true, padding = "large", ui.text { "Content hidden", tone = "muted" } }
	else
		-- Includes mounted extensions, images and stateful component instances.
		return ui.spacer {}
	end
	return copy
end

return component {
	title = "Scramble mode", example = { tree = ui.text { "Example" } },
	build = function(props)
		if not props.enabled then return props.tree end
		return ui.column { grow = true, gap = "none", scrub(props.tree),
			ui.row { height = 22, shrink = false, align = "center", justify = "center", class = "status",
				ui.text { "Scramble Mode", size = "small" },
			},
		}
	end,
}
