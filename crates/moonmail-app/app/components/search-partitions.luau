local function build(props)
	local mail = props.mail
	local id, key = mail.active_saved_search, mail.partition_key
	local page = mail.partition_page or { rows = {} }
	local items = {
		ui.text { "Automatic partitions", class = "title" },
		ui.text { mail.partition_name or "Open partitions from a saved search", weight = "semibold" },
		ui.text { "Groups cover the entire saved query, including nested sources. Counts are distinct messages, not thread rows. A message can belong to multiple groups; empty values are omitted.", tone = "muted" },
		ui.row { ui.button { label = "Saved searches", small = true, msg = "searches" }, ui.button { label = "Back to mail", small = true, msg = "mail" } },
	}
	local choices = {}
	for _, field in { { "sender", "Sender address" }, { "recipient", "To / Cc address" }, { "keyword", "Keyword" }, { "none", "No partitions" } } do
		table.insert(choices, ui.button { label = field[2], small = true, disabled = not key, variant = if (mail.partition_field or "none") == field[1] then "primary" else "ghost", msg = { action = "partition_field", id = id, key = key, field = field[1] } })
	end
	table.insert(items, ui.row { table.unpack(choices) })
	table.insert(items, ui.text { "Exact addresses preserve local-part case and fold domain case. Keyword groups use wire names. Each page shows up to 100 values.", tone = "muted" })
	for _, bucket in page.rows or {} do
		table.insert(items, ui.row { shrink = false, padding = "small", align = "center",
			ui.column { grow = true, ui.text { bucket.value, weight = "semibold", truncate = true }, ui.text { tostring(bucket.total) .. " messages · " .. tostring(bucket.unread) .. " unread", tone = "muted" } },
			ui.button { label = "Open group", small = true, msg = { action = "partition_choose", id = id, key = key, after = mail.partition_after, value = bucket.value } },
		})
	end
	table.insert(items, ui.row {
		ui.button { label = "First page", small = true, disabled = not mail.partition_after, msg = { action = "partition_first", id = id, key = key } },
		ui.button { label = "Next page", small = true, disabled = not page.next, msg = { action = "partition_next", id = id, key = key, after = mail.partition_after, value = page.next } },
	})
	return ui.column { class = "form", grow = true, scroll = true, table.unpack(items) }
end

return component { title = "Search partitions", example = { mail = {} }, build = build }
