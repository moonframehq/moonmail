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

-- Generate pronounceable stand-ins instead of permuting the source letters.
-- Only length, position and capitalization influence the generated letters.
local starts = { "b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z" }
local ends = { "a", "e", "i", "o", "u", "an", "el", "in", "or", "us", "en" }

local function word(token, position)
	local length = math.clamp(utf8.len(token) or #token, 2, 14)
	local capitalized = string.match(token, "^%u") ~= nil
	local seed = (length * 8009 + position * 3701 + (if capitalized then 7919 else 0)) % 65521
	local result = ""
	while #result < length do
		result ..= starts[seed % #starts + 1] .. ends[math.floor(seed / #starts) % #ends + 1]
		seed = (seed * 251 + 17) % 65521
	end
	result = string.sub(result, 1, length)
	if result == string.lower(token) then result ..= "a" end
	return if capitalized then string.upper(string.sub(result, 1, 1)) .. string.sub(result, 2) else result
end

local function replace_token(token, position)
	if string.find(token, "@", 1, true) then
		local address = word(token, position) .. "@example.invalid"
		return if string.sub(token, 1, 1) == "<" then "<" .. address .. ">" else address
	end
	if string.match(token, "^https?://") then return "https://example.invalid/" .. word(token, position) end
	if string.match(token, "^[%d:/.,%-]+$") then
		return (string.gsub(token, "%d+", function(digits)
			if #digits == 4 then return "2024" end
			return string.sub(string.rep("123", 5), 1, #digits)
		end))
	end
	local punctuation = string.match(token, "[.,!?;:]$") or ""
	return word(token, position) .. punctuation
end

local function mask(value)
	if type(value) ~= "string" or value == "" then return "" end
	if labels[value] then return value end
	-- Bound the work even when a draft or diagnostic is very long. Whitespace
	-- keeps the shape of multi-line fields without copying any private tokens.
	local position = 0
	return (string.gsub(string.sub(value, 1, 512), "%S+", function(token)
		position += 1
		return replace_token(token, position)
	end))
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
	elseif (kind == "browser" and node.id == "message-reader") or (kind == "view" and node.name == "mail-reader") then
		-- Both reader backends show a synthetic sample without loading the mail.
		return ui.column { grow = true, padding = "large", gap = "normal",
			ui.text { "Nela soren," },
			ui.text { "Valen tira monel saviro. Pelen nori valeta, saven lorin taro mira." },
			ui.text { "Ferin dalo reniva solan. Lora delan vena torin." },
		}
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
