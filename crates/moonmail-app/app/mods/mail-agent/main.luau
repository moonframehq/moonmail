-- The assistant panel: a conversation with an agent (Claude, Codex,
-- OpenCode, pi, Gemini...) over ACP, run by herdr-acp through the `chat`
-- provider and drawn by Moonframe's `agent-chat` component, as Sheprd runs
-- its chats. The agent works in the archive's agent folder
-- (`app.mail.agent.dir`), whose configuration gives it Moonmail's MCP tools
-- and whose guide lists them. It asks before every tool that writes; sends
-- also wait here for the user's Send on a card Moonmail draws, whatever the
-- agent does. Every tool call is in the activity log (the list button).

local agent_chat = use "agent-chat"

local SUGGESTIONS = {
	"Summarize my unread mail",
	"What needs a reply from me?",
	"Reply to this message",
	"Make a rule that tags mail like this",
}

-- Each field occupies one line in the hidden app-context frame.
local function field(label, value)
	return label .. ": " .. (string.gsub(value, "[\r\n]+", " "))
end

local function mail_view(mail)
	local checked = {}
	for _, row in mail.rows or {} do
		if row.checked then table.insert(checked, tostring(row.id)) end
	end
	table.sort(checked)
	local parts = {}
	if #checked > 0 then table.insert(parts, "checked messages " .. table.concat(checked, ", ")) end
	for _, mailbox in mail.mailboxes or {} do
		if mailbox.id == mail.mailbox then
			table.insert(parts, `folder {mailbox.name} (id {mailbox.id}, {mailbox.account})`)
		end
	end
	if mail.search and mail.search ~= "" then table.insert(parts, `search {mail.search}`) end
	return if #parts > 0 then table.concat(parts, "; ") else "none"
end

-- Read only Moonmail fields in the frame, never text from the visible prompt.
local function frame_field(text, label)
	local frame = string.match(text, "^<app%-context>\n(.-)\n</app%-context>\n\n")
	if frame then
		return string.match("\n" .. frame .. "\n", "\n(" .. label .. ": [^\n]*)\n")
	end
	return nil
end

local function needs_field(transcript, label, current)
	if not transcript then return true end
	-- A queued update can still be removed. Omit the field only if it is
	-- already established and no queued prompt could change it.
	for _, queued in transcript.queued or {} do
		local prior = frame_field(queued.text, label)
		if (prior and prior ~= current) or string.sub(queued.text, 1, 3) == "…" then return true end
	end
	local entries = transcript.entries or {}
	for index = #entries, 1, -1 do
		local entry = entries[index].kind
		if entry.type == "user" then
			local prior = frame_field(entry.text, label)
			if prior then return prior ~= current end
			-- Moonframe keeps the tail of long prompts, which may hide a frame.
			if string.sub(entry.text, 1, 3) == "…" then break end
		end
	end
	return true
end

local function context(mail, chats, id)
	local transcript
	for _, chat in chats or {} do
		if chat.id == id then
			transcript = chat.transcript
			break
		end
	end
	local selected = mail.selected and mail.selected.summary
	local message = field("Open email", if selected
		then `message {selected.id} “{selected.subject}” from {selected.sender} <{selected.address}>`
		else "none")
	local view = field("Mail view", mail_view(mail))
	local parts = {}
	if needs_field(transcript, "Open email", message) then table.insert(parts, message) end
	if needs_field(transcript, "Mail view", view) then table.insert(parts, view) end
	return if #parts > 0 then table.concat(parts, "\n") else nil
end

-- A send the agent asked for: Moonmail sends only when the user presses
-- Send, which names the digest of exactly what this card shows.
local function approval_view(ask)
	local lines = {
		ui.row { gap = "small", align = "center",
			ui.icon { name = "send", size = "small", tone = "warning" },
			ui.text { "Send this message?", weight = "semibold" },
		},
		ui.text { "From: " .. (ask.from or ""), class = "agent-hint", truncate = true },
		ui.text { "To: " .. (ask.to or ""), class = "agent-hint", truncate = true },
	}
	if ask.cc and ask.cc ~= "" then table.insert(lines, ui.text { "Cc: " .. ask.cc, class = "agent-hint", truncate = true }) end
	if ask.bcc and ask.bcc ~= "" then table.insert(lines, ui.text { "Bcc: " .. ask.bcc, class = "agent-hint", truncate = true }) end
	table.insert(lines, ui.text { if ask.subject ~= "" then ask.subject else "(no subject)", weight = "medium", wrap = true })
	-- Every attached file, so nothing leaves unseen.
	for _, file in ask.files or {} do
		local size = if file.bytes >= 1048576 then string.format("%.1f MB", file.bytes / 1048576)
			else string.format("%d KB", math.ceil(file.bytes / 1024))
		table.insert(lines, ui.row { gap = "small", align = "center",
			ui.icon { name = "paperclip", size = "small", tone = "warning" },
			ui.text { file.name, weight = "medium", truncate = true },
			ui.text { size, class = "agent-hint" },
		})
	end
	table.insert(lines, ui.text { (ask.body or "") .. (if ask.truncated then "\n…" else ""), class = "agent-approval-body", wrap = true, lines = 10 })
	table.insert(lines, ui.row { gap = "small", align = "center",
		ui.button { "Review", small = true, variant = "ghost", tooltip = "Open it in the composer", msg = { action = "draft_open", id = ask.draft }, to = "provider:mail" },
		ui.spacer {},
		ui.button { "Deny", small = true, variant = "outline", msg = { action = "agent_request", id = ask.id, decision = "deny" }, to = "provider:mail" },
		ui.button { "Send", small = true, variant = "primary", msg = { action = "agent_request", id = ask.id, decision = "approve", digest = ask.digest, key = ask.key }, to = "provider:mail" },
	})
	return ui.column { class = "agent-approval", table.unpack(lines) }
end

local function activity_view(activity)
	local rows = {}
	for index = #activity, 1, -1 do
		local entry = activity[index]
		local icon = if not entry.ok then "circle-x" elseif entry.writes then "pencil" else "eye"
		local tone = if not entry.ok then "danger" elseif entry.writes then "accent" else "muted"
		table.insert(rows, ui.row { gap = "small", align = "start", class = "agent-log-row",
			ui.icon { name = icon, size = "small", tone = tone },
			ui.column { gap = "none", grow = true,
				ui.text { entry.summary or "", size = "small", wrap = true, lines = 3 },
				ui.text { `{entry.tool} · {entry.source}`, class = "agent-hint" },
			},
		})
	end
	if #rows == 0 then
		return ui.text { "No agent has used Moonmail's tools yet.", class = "agent-hint", wrap = true }
	end
	return ui.column { gap = "small", table.unpack(rows) }
end

return mod {
	title = "Assistant",
	styles = {
		[".agent-panel"] = { background = "background" },
		[".agent-bar"] = { padding = { 0, 6, 0, 12 }, border = "bottom", border_color = "border" },
		[".agent-approval"] = { gap = 6, padding = 12, radius = 10, border = true, border_color = "warning/60", background = "warning/10" },
		[".agent-approval-body"] = { size = "small", padding = { 6, 8 }, radius = 6, background = "background" },
		[".agent-log-row"] = { padding = { 4, 0 }, border = "bottom", border_color = "border" },
		[".agent-hint"] = { size = "small", color = "muted-foreground" },
	},

	init = function(previous)
		local state = previous or {}
		if type(state.generation) ~= "number" then state.generation = 1 end
		return state
	end,

	update = function(state, msg)
		if msg == "new" then
			-- A new component instance starts a new chat on its first prompt.
			state.chat = nil
			state.generation += 1
			state.log = false
		elseif msg == "log" then
			state.log = not state.log
		elseif type(msg) == "table" and msg.msg == "chat-started" then
			state.chat = msg.chat
		elseif type(msg) == "table" and msg.msg == "link" and type(msg.url) == "string" then
			return state, { { to = "app:open", msg = msg.url } }
		end
		return state
	end,

	view = function(state, app)
		local mail = app.mail or {}
		local approvals = {}
		for _, ask in (mail.agent and mail.agent.approvals) or {} do
			table.insert(approvals, approval_view(ask))
		end
		local body
		if state.log then
			body = ui.column { grow = true, scroll = true, gap = "small", padding = "large",
				ui.text { "Every Moonmail tool an agent called, newest first.", class = "agent-hint" },
				activity_view((mail.agent and mail.agent.activity) or {}),
			}
		elseif not (mail.agent and mail.agent.dir) then
			body = ui.column { grow = true, padding = "large",
				ui.text { "The assistant's folder is not ready; see the status bar.", class = "agent-hint", wrap = true },
			}
		else
			body = agent_chat {
				id = `mail-assistant-{state.generation}`,
				chat = state.chat,
				cwd = mail.agent.dir,
				context = context(mail, app.chat and app.chat.chats, state.chat),
				title = "Ask about your mail",
				placeholder = "Ask about your mail…",
				suggestions = SUGGESTIONS,
				started = "chat-started",
				link = "link",
			}
		end
		return ui.column { grow = true, gap = "none", class = "agent-panel",
			ui.row { class = "agent-bar", height = 36, shrink = false, align = "center", gap = "small",
				ui.icon { name = "bot", size = "small" },
				ui.text { if state.log then "Agent activity" else "Assistant", weight = "semibold" },
				ui.spacer {},
				ui.button { icon = "list", small = true, variant = if state.log then "secondary" else "ghost", tooltip = "Agent activity log", msg = "log" },
				ui.button { icon = "square-pen", small = true, variant = "ghost", tooltip = "New conversation", msg = "new" },
			},
			#approvals > 0 and ui.column { gap = "small", padding = "normal", shrink = false, table.unpack(approvals) },
			ui.column { grow = true, gap = "none", body },
		}
	end,
}
