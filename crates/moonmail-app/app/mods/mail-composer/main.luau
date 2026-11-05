local composer = use "mail-compose-view"

local function same_choices(before, after)
	if #before ~= #after then return false end
	for index, choice in before do
		local other = after[index]
		if choice.account ~= other.account or choice.identity ~= other.identity or choice.label ~= other.label or choice.current ~= other.current then return false end
	end
	return true
end

return mod {
	title = "Message · Moonmail",
	window = { size = { width = 760, height = 680 }, min_size = { width = 620, height = 520 }, titlebar = "hidden", traffic_lights = { x = 20, y = 23 } },
	subscribe = { "provider/mail" },
	init = function() return { cc = false, from_open = false, from_choices = {}, send_options = false } end,
	update = function(state, msg)
		if msg == "open" then
			state.from_open = false; state.send_options = false
			return state, { { to = "app:window", msg = { open = "mail-composer" } } }
		elseif msg == "close" then return state, { { to = "app:window", msg = { close = "mail-composer" } } }
		elseif msg == "toggle-cc" then state.cc = not state.cc
		elseif msg == "toggle-from" then
			state.from_open = not state.from_open; state.from_index = 1
			for index, choice in state.from_choices do if choice.current then state.from_index = index end end
		elseif msg == "close-from" then state.from_open = false
		elseif msg == "manage-accounts" then
			state.from_open = false
			return state, { { to = "settings", msg = "open-accounts" } }
		elseif msg == "toggle-send" then state.send_options = not state.send_options
		elseif msg == "save-close" and state.draft_id then
			return state, { { to = "provider:mail", msg = { action = "draft_close", id = state.draft_id } } }
		elseif msg == "send" and state.draft_id and state.can_send then
			return state, { { to = "provider:mail", msg = { action = "queue", id = state.draft_id } } }
		elseif type(msg) == "table" then
			if msg.topic == "provider/mail" then
				local mail = msg.value or {}
				local draft = mail.draft
				if not draft or draft.id ~= state.draft_id then
					state.cc = draft ~= nil and (draft.cc ~= "" or draft.bcc ~= ""); state.from_open = false; state.send_options = false
				end
				state.draft_id = if draft then draft.id else nil
				if not same_choices(state.from_choices, mail.from_accounts or {}) then state.from_open = false; state.from_index = nil end
				state.from_choices = mail.from_accounts or {}
				state.can_send = not mail.busy and not mail.demo
			elseif msg.from_step and state.from_open and msg.id == state.draft_id then
				state.from_index = ((state.from_index or 1) - 1 + msg.from_step) % (#state.from_choices + 1) + 1
			elseif msg.from_choose and state.from_open and msg.id == state.draft_id then
				state.from_open = false
				local choice = state.from_choices[state.from_index or 1]
				if choice then return state, { { to = "provider:mail", msg = { action = "draft_account", id = state.draft_id, account = choice.account, identity = choice.identity } } } end
				return state, { { to = "settings", msg = "open-accounts" } }
			elseif msg.attach_draft then
				return state, { { to = "app:pick", msg = { kind = "open", multiple = true, prompt = "Attach files", reply = { action = "draft_attach", id = msg.attach_draft } } } }
			elseif msg.action and not msg.cancelled then
				if msg.action == "draft_identity" or msg.action == "draft_account" then state.from_open = false end
				return state, { { to = "provider:mail", msg = msg } }
			end
		end
	end,
	view = function(state, app)
		return composer { state = state, mail = app.mail or {} }
	end,
}
