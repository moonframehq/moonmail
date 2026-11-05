local function build(props)
	local state, mail = props.state, props.mail
	local folders = {}
	for _, folder in mail.mailboxes or {} do folders[folder.id] = folder end
	local items = {
		ui.text { "Folder-entry keyword rules", class = "title" },
		ui.text { "Run once when a message enters a physical folder. Saving or enabling a rule leaves existing messages alone. Rules run in creation order; later rules see earlier changes.", tone = "muted" },
		ui.text { "Conditions support account:, in:, is:, keyword:, tag:, sender-address:, recipient-address:, after:, before: and Boolean operators. Text/body conditions and smart-mailbox triggers are not available yet.", tone = "muted" },
		ui.row { ui.button { label = "Back to mail", small = true, msg = "mail" }, ui.button { label = "Tags & recovery", small = true, msg = "tags" }, ui.button { label = "New rule", small = true, msg = { rule_edit = { id = 0, revision = 0, name = "", source = "", keyword = "", value = true, enabled = false, mailbox = mail.mailbox } } } },
	}
	local edit = state.rule_edit
	if edit then
		table.insert(items, ui.text { if edit.id == 0 then "New rule · choose a source folder" else "Edit " .. edit.name, weight = "semibold" })
		local choices = {}
		for _, folder in mail.mailboxes or {} do
			if folder.selectable then table.insert(choices, ui.button { label = folder.account .. " · " .. folder.name, small = true, variant = if edit.mailbox == folder.id then "primary" else "ghost", msg = { rule_folder = folder.id } }) end
		end
		table.insert(items, ui.column { height = 96, scroll = true, gap = "small", table.unpack(choices) })
		for _, field in { { "name", "Rule name" }, { "source", "Metadata condition (empty matches every entry)" }, { "keyword", "Wire keyword to add or remove" } } do
			table.insert(items, ui.input { id = "rule-" .. tostring(edit.id) .. "-" .. field[1], value = edit[field[1]] or "", placeholder = field[2], change = { rule_field = field[1] } })
		end
		table.insert(items, ui.row {
			ui.button { label = if edit.value then "Action: Add keyword" else "Action: Remove keyword", small = true, msg = { rule_toggle = "value" } },
			ui.button { label = if edit.enabled then "Automatic: Enabled" else "Automatic: Disabled", small = true, msg = { rule_toggle = "enabled" } },
		})
		local folder = folders[edit.mailbox]
		if folder then table.insert(items, ui.button { label = "Save rule", small = true, msg = { action = "rule_save", id = edit.id, revision = edit.revision, mailbox = folder.id, account = folder.account, folder = folder.name, epoch = folder.epoch, name = edit.name, source = edit.source, keyword = edit.keyword, value = edit.value, enabled = edit.enabled } }) end
	end
	for _, rule in mail.keyword_rules or {} do
		local folder = folders[rule.mailbox]
		table.insert(items, ui.column { gap = "small", padding = "small", shrink = false,
			ui.text { rule.name .. " · " .. (if rule.enabled then "enabled" else "disabled"), weight = "semibold" },
			ui.text { (if folder then folder.account .. " · " .. folder.name else "Unavailable source folder") .. " · " .. (if rule.value then "add " else "remove ") .. rule.keyword, tone = "muted" },
			ui.text { if rule.source ~= "" then rule.source else "All new folder entries", tone = "muted" },
			ui.row {
				ui.button { label = "Edit", small = true, msg = { rule_edit = rule } },
				ui.button { label = "Apply to selected", small = true, disabled = #(mail.move_selection or {}) == 0 or not folder, msg = { action = "rule_apply", id = rule.id, revision = rule.revision, messages = mail.move_selection, scope = mail.mailbox } },
				ui.button { label = "Delete rule", small = true, msg = { action = "rule_delete", id = rule.id, revision = rule.revision } },
			},
		})
	end
	table.insert(items, ui.text { "Matching rule history · newest first · 100 per page", weight = "semibold" })
	table.insert(items, ui.text { "Blocked actions are not retried automatically. Resolve pending transfers or keyword limits, then explicitly apply the current rule to selected messages.", tone = "muted" })
	for _, run in mail.rule_runs or {} do
		table.insert(items, ui.column { gap = "small", shrink = false,
			ui.text { run.name .. " · revision " .. tostring(run.revision) .. " · message " .. tostring(run.message) .. " · " .. run.mode .. " · " .. run.state },
			ui.text { (if run.value then "Add " else "Remove ") .. run.keyword .. " · " .. run.detail, tone = "muted" },
		})
	end
	table.insert(items, ui.row { ui.button { label = "First page", small = true, disabled = not mail.rule_before, msg = { action = "rule_first" } }, ui.button { label = "Next page", small = true, disabled = not mail.rule_next, msg = { action = "rule_next", id = mail.rule_next } } })
	return ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
end

return component { title = "Entry rules", example = { state = {}, mail = {} }, build = build }
