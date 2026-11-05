const icons = {
  up: "m6 14 6-6 6 6",
  down: "m6 10 6 6 6-6",
  chevron: "m9 5 7 7-7 7",
  compose:
    "M13 5H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8 M16 3l5 5 M9 15l1-5L19 1l4 4-9 9z",
  reply: "m9 5-7 6 7 6 M2 11h11c6 0 8 4 8 9",
  replyAll: "m7 5-6 6 6 6 M13 5l-6 6 6 6 M7 11h8c5 0 7 4 7 9",
  forward: "m15 5 7 6-7 6 M22 11H11c-6 0-8 4-8 9",
  flag: "M5 22V3 M5 4c5-5 9 5 15 0v11c-6 5-10-5-15 0",
  mail: "M3 5h18v14H3z m0 1 9 7 9-7",
  archive: "M3 3h18v5H3z M5 8v13h14V8 M9 12h6",
  trash: "M4 6h16 M9 6V3h6v3 M6 6l1 15h10l1-15 M10 10v7 M14 10v7",
  folder: "M2 6h8l2 3h10v11H2z",
  sync: "M20 8a8 8 0 0 0-14-3L3 8 M3 3v5h5 M4 16a8 8 0 0 0 14 3l3-3 M21 21v-5h-5",
  search: "M16 10a6 6 0 1 1-12 0 6 6 0 0 1 12 0 m-2 4 7 7",
  settings:
    "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M9 3l1-2h4l1 2 3 2 3 1 1 4-2 2v3l1 3-3 3-3-1h-3l-3 1-3-3 1-3v-3L1 10l1-4 3-1z",
  inbox: "M5 3h14l3 12v6H2v-6z M2 15h6l2 3h4l2-3h6",
  send: "m22 2-7 20-4-9L2 9z M11 13 22 2",
  draft: "M5 2h9l5 5v15H5z M14 2v6h5 M8 12h8 M8 16h6",
  ban: "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0 M6 6l12 12",
  star: "m12 2 3 6 7 1-5 5 1 7-6-3-6 3 1-7-5-5 7-1z",
  clip: "m8 13 7-7a3 3 0 0 1 4 4L9 20a5 5 0 0 1-7-7L13 2 M5 16 15 6",
  check: "m4 12 5 5L20 6",
  more: "M5 12h.01 M12 12h.01 M19 12h.01",
  sidebar: "M3 3h18v18H3z M9 3v18",
  signature: "M3 17c4-3 8-15 5-15S1 22 9 16s6-6 5-2 2 4 7-1 M3 22h18",
  expand: "M8 3H3v5 M16 3h5v5 M3 16v5h5 M21 16v5h-5",
};
const icon = (name, size = "") =>
  `<svg class="icon ${size}" viewBox="0 0 24 24" aria-hidden="true"><path d="${icons[name] || icons.more}"/></svg>`;
const btn = (name, label, action = "", extra = "") =>
  `<button class="icon-button ${extra}" title="${label}" aria-label="${label}" ${action ? `data-action="${action}"` : `data-demo="${label}"`}>${icon(name)}</button>`;
const traffic =
  '<div class="window-buttons" aria-hidden="true"><i></i><i></i><i></i></div>';
const drafts = [
  {
    to: "Nora Reed",
    subject: "A small idea for the weekend",
    from: "Personal",
    time: "10:42",
    file: true,
    body: "I keep coming back to your idea of a slower weekend. What if we took the morning train, found somewhere by the water, and left the rest unplanned?",
  },
  {
    to: "Theo Martin, Maya Laurent",
    subject: "Re: A quieter place to work",
    from: "Studio",
    time: "09:18",
    file: false,
    body: "The new space looks lovely. Could we visit on Thursday afternoon? I’d love to see how the light moves through the studio.",
  },
  {
    to: "Clara Jensen",
    subject: "Notes from the coast",
    from: "Personal",
    time: "Yesterday",
    file: true,
    body: "A few notes from the coast, before the details slip away. The weather was beautiful and we found that little bookshop you mentioned.",
  },
  {
    to: "Julian Park",
    subject: "Dinner at seven?",
    from: "Personal",
    time: "Yesterday",
    file: false,
    body: "Would seven work for dinner on Friday? I can bring something for dessert.",
  },
  {
    to: "Maya Laurent",
    subject: "The November reading list",
    from: "Studio",
    time: "Monday",
    file: false,
    body: "Here are a few things I’ve been reading lately. I thought you might enjoy the essay on small, independent studios.",
  },
  {
    to: "Nora Reed",
    subject: "Photographs from Lisbon",
    from: "Personal",
    time: "Monday",
    file: true,
    body: "I finally had a chance to look through the photographs. These are a few of my favorites from our afternoon walk.",
  },
];
let view = "floating",
  selected = 0,
  ascending = true,
  query = "",
  sortKey = "date";
const edits = new Map();
const canvas = document.getElementById("canvas");
const stage = document.querySelector(".stage");
function escapeHTML(value) {
  return value.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
}
function sidebar() {
  const item = (name, label, count = "", klass = "") =>
    `<button class="side-item ${klass}" ${label === "Drafts" ? 'data-action="drafts"' : `data-demo="${label}"`}>${icon(name, "small")}<span>${label}</span><span class="count">${count}</span></button>`;
  return `<aside class="sidebar"><h3>Mailboxes</h3>${item("inbox", "All Messages", "128")}${item("inbox", "Inbox", "12")}${item("draft", "Drafts", "6", "selected")}${item("send", "Sent")}${item("archive", "Archive")}${item("send", "Outbox")}${item("ban", "Junk")}${item("trash", "Trash")}<h3>Favorites</h3>${item("mail", "Unread", "12")}${item("star", "Flagged", "4")}${item("search", "Saved searches")}<h3>Personal</h3>${item("folder", "Projects")}${item("folder", "Moonmail", "", "indented")}${item("folder", "Design", "", "deeper")}${item("folder", "Reading")}${item("folder", "Travel")}<div class="account"><div class="avatar">AL</div><div>Alex Lane<small>Personal · Fastmail</small></div>${icon("down", "tiny")}</div></aside>`;
}
function toolbar() {
  return `<div class="titlebar">${traffic}<span class="toolbar-title">Drafts</span><div class="toolbar-group">${btn("up", "Previous message")}${btn("down", "Next message")}${btn("compose", "New message", "compose", "accent")}</div><div class="toolbar-group">${btn("reply", "Reply")}${btn("replyAll", "Reply all")}${btn("forward", "Forward")}</div><div class="toolbar-group">${btn("flag", "Flag")}${btn("mail", "Mark unread")}${btn("archive", "Archive")}${btn("trash", "Discard draft", "discard")}${btn("folder", "Move to folder")}${btn("sync", "Sync")}</div><span class="spacer"></span>${btn("sidebar", "Toggle sidebar")}<label class="search">${icon("search", "small")}<input type="search" id="search" placeholder="Search drafts" aria-label="Search drafts" value="${escapeHTML(query)}"></label>${btn("settings", "Settings")}</div>`;
}
function table() {
  let list = drafts
    .map((d, id) => ({ ...d, id }))
    .filter((d) =>
      (d.subject + " " + d.to).toLowerCase().includes(query.toLowerCase()),
    );
  list.sort((a, b) =>
    sortKey === "date"
      ? a.id - b.id
      : sortKey === "recipient"
        ? a.to.localeCompare(b.to)
        : a.subject.localeCompare(b.subject),
  );
  if (!ascending) list.reverse();
  if (view === "integrated") list = list.slice(0, 3);
  return `<div class="list-heading"><h2>Drafts</h2><span class="subtle">${drafts.length} messages</span><span class="spacer"></span><span class="subtle">Saved on this Mac</span>${btn("more", "Draft options")}</div><div class="table-wrap"><table aria-label="Draft messages"><thead><tr><th class="check-col"><input type="checkbox" aria-label="Select all drafts" id="select-all"></th><th class="recipient-col"><button data-sort="recipient">To</button></th><th><button data-sort="subject">Subject</button></th><th class="file-col">${icon("clip", "tiny")}</th><th class="identity-col">From</th><th class="date-col"><button data-sort="date">Modified ${ascending ? "⌄" : "⌃"}</button></th></tr></thead><tbody>${list.map((d) => `<tr tabindex="0" aria-selected="${d.id === selected}" class="${d.id === selected ? "selected" : ""}" data-draft="${d.id}"><td class="check-col"><input type="checkbox" aria-label="Select draft to ${d.to}"></td><td>${d.to}</td><td>${escapeHTML(edits.get(d.id)?.subject ?? d.subject)}</td><td>${d.file ? icon("clip", "tiny") : ""}</td><td style="color:var(--muted)">${d.from}</td><td style="color:var(--muted)">${d.time}</td></tr>`).join("")}</tbody></table></div><div class="table-status"><span>${list.length} drafts shown</span><span>Double-click to continue writing</span></div>`;
}
function reader() {
  const d = drafts[selected];
  return `<article class="reader"><div class="draft-label">DRAFT · ${d.from.toUpperCase()}</div><div class="reader-actions"><button class="secondary" data-action="compose">Continue writing ↗</button></div><h2>${escapeHTML(edits.get(selected)?.subject ?? d.subject)}</h2><div class="reader-meta"><div class="avatar">AL</div><div><strong>Alex Lane</strong> &lt;alex@example.com&gt;<br><span>To: ${d.to}</span></div></div><p>Hi ${d.to.split(" ")[0]},</p><p>${d.body}</p><p>No rush — let me know what you think.</p><p>Alex</p>${d.file ? attachment() : ""}</article>`;
}
function attachment() {
  return `<div class="attachment"><div class="file-icon">PDF</div><div>Weekend notes.pdf<small>248 KB · attached to draft</small></div>${icon("clip", "small")}</div>`;
}
function composer() {
  const d = drafts[selected],
    edit = edits.get(selected);
  return `<section class="composer ${view === "floating" ? "floating-composer" : "integrated-composer"}" aria-label="Compose message"><div class="composer-bar">${view === "floating" ? traffic : btn("draft", "Back to drafts", "drafts")}<div class="composer-title">${view === "floating" ? "New message" : "Continue writing"}<span class="saved">${icon("check", "tiny")}<span id="save-state">Draft saved locally</span></span></div><span class="spacer"></span>${btn("clip", "Attach files", "attach")}<div class="send-control"><button data-action="send">${icon("send")}Send</button><button data-action="send-menu" aria-label="More send options" aria-expanded="false">${icon("down", "tiny")}</button><div class="popover" id="send-menu" hidden><button data-action="send">Send message <small>⌘ Return</small></button><button data-action="queue">Queue only</button></div></div></div><div class="envelope"><div class="field"><label for="identity">From</label><select id="identity" class="identity-select"><option ${d.from === "Personal" ? "selected" : ""}>Alex Lane &lt;alex@example.com&gt; · Personal</option><option ${d.from === "Studio" ? "selected" : ""}>Alex Lane &lt;alex@studio.example&gt; · Studio</option></select></div><div class="field"><label for="recipient">To</label><span class="address-chip">${d.to} ${icon("down", "tiny")}</span><input id="recipient" aria-label="Add recipient" placeholder=""><button class="reveal" data-action="cc" aria-expanded="false">Cc / Bcc</button></div><div class="field cc-row" hidden><label for="cc">Cc</label><input id="cc" placeholder="Add a recipient"></div><div class="field cc-row" hidden><label for="bcc">Bcc</label><input id="bcc" placeholder="Add a hidden recipient"></div><div class="field"><label for="subject">Subject</label><input id="subject" class="subject" value="${escapeHTML(edit?.subject ?? d.subject)}"></div></div><div class="writing" contenteditable="true" role="textbox" aria-multiline="true" aria-label="Message body" spellcheck="false">${edit?.body ?? `<p>Hi ${d.to.split(" ")[0]},</p><p>${d.body}</p><p>No rush — let me know what you think.</p><p class="signature">Alex</p>`}</div><div class="attachment-shelf" ${!d.file ? "hidden" : ""}>${d.file ? attachment() : ""}</div><div class="composer-footer">${btn("clip", "Attach files", "attach")}${btn("signature", "Signature", "signature")}<span class="plain-text">Plain text</span><span class="spacer"></span><span class="hint">⌘ Return to send</span><button class="text-button" data-action="drafts">Save & close</button>${btn("trash", "Discard draft", "discard")}</div></section>`;
}
const notes = {
  floating: [
    "01 / FOCUS",
    "One message. Its own space.",
    "A floating composer with a compact envelope, a comfortable line length, and Send always in view. Close the window to return to your saved draft.",
  ],
  integrated: [
    "02 / CONTEXT",
    "Stay in the flow of your mailbox.",
    "Write directly below the Drafts table. Keep nearby drafts in view, with the same compact address rows and a persistent Send control.",
  ],
  drafts: [
    "03 / CONSISTENCY",
    "Drafts are messages, too.",
    "The same table language as the inbox: recipients, subject, attachments, identity, and modified time. Select to preview; double-click to continue writing.",
  ],
};
function remember() {
  const subject = document.getElementById("subject"),
    body = document.querySelector(".writing");
  if (subject && body)
    edits.set(selected, { subject: subject.value, body: body.innerHTML });
}
function render() {
  canvas.innerHTML = `<div class="app">${toolbar()}${sidebar()}<section class="workspace">${table()}${view === "integrated" ? composer() : reader()}</section><div class="statusbar"><span><i class="status-dot"></i>All changes saved on this Mac</span><span>⌘ N New message &nbsp;&nbsp; ⌘ R Sync &nbsp;&nbsp; ↑ ↓ Navigate</span></div></div>${view === "floating" ? `<div class="veil"></div>${composer()}` : ""}`;
  document.querySelectorAll("[data-view]").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === view);
    b.setAttribute("aria-pressed", b.dataset.view === view);
  });
  ["note-number", "note-title", "note-body"].forEach(
    (id, i) => (document.getElementById(id).textContent = notes[view][i]),
  );
  document.getElementById("pick").innerHTML =
    `Choose ${view === "drafts" ? "this Drafts layout" : "this direction"} <span>↗</span>`;
  fit();
}
function setView(next) {
  remember();
  view = next;
  render();
}
function fit() {
  const scale = stage.clientWidth / 1280;
  canvas.style.transform = `scale(${scale})`;
  stage.style.height = `${790 * scale}px`;
}
let toastTimer;
function toast(message) {
  const feedback = document.getElementById("feedback");
  feedback.textContent = message;
  feedback.classList.add("visible");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => feedback.classList.remove("visible"), 4500);
}
document
  .querySelectorAll("[data-view]")
  .forEach((b) => b.addEventListener("click", () => setView(b.dataset.view)));
document.getElementById("theme").addEventListener("click", (event) => {
  const dark = document.body.classList.toggle("dark");
  event.target.textContent = dark ? "Light appearance" : "Dark appearance";
  event.target.setAttribute(
    "aria-label",
    `Switch to ${dark ? "light" : "dark"} appearance`,
  );
});
document
  .getElementById("density")
  .addEventListener("change", (event) =>
    document.body.classList.toggle("compact", event.target.value === "compact"),
  );
document.getElementById("pick").addEventListener("click", async () => {
  const summary = `Moonmail mockup choice: ${notes[view][1]}`;
  const data = {
    view,
    appearance: document.body.classList.contains("dark") ? "dark" : "light",
    density: document.getElementById("density").value,
  };
  if (window.herdr?.send) {
    window.herdr.send(summary, data);
    toast("Choice ready in the notes panel — press Send to agent.");
  } else {
    try {
      await navigator.clipboard.writeText(
        summary + "\n" + JSON.stringify(data),
      );
      toast("Choice copied — paste it into the conversation.");
    } catch {
      toast(summary + " Tell me your choice in the conversation.");
    }
  }
});
canvas.addEventListener("click", (event) => {
  const target = event.target.closest("button"),
    row = event.target.closest("[data-draft]");
  if (event.target.id === "select-all") {
    canvas
      .querySelectorAll("tbody input[type=checkbox]")
      .forEach((box) => (box.checked = event.target.checked));
    return;
  }
  if (row && event.target.type !== "checkbox") {
    if (selected !== Number(row.dataset.draft)) {
      remember();
      selected = Number(row.dataset.draft);
      render();
    }
    return;
  }
  if (!target) return;
  if (target.dataset.sort) {
    remember();
    ascending = sortKey === target.dataset.sort ? !ascending : true;
    sortKey = target.dataset.sort;
    render();
    return;
  }
  if (target.dataset.demo) {
    toast(`${target.dataset.demo} · visual control in this mockup`);
    return;
  }
  switch (target.dataset.action) {
    case "compose":
      setView(view === "integrated" ? "integrated" : "floating");
      break;
    case "drafts":
      setView("drafts");
      break;
    case "cc": {
      const show = canvas.querySelector(".cc-row").hidden;
      canvas.querySelectorAll(".cc-row").forEach((r) => (r.hidden = !show));
      target.setAttribute("aria-expanded", show);
      break;
    }
    case "send-menu": {
      const menu = document.getElementById("send-menu");
      menu.hidden = !menu.hidden;
      target.setAttribute("aria-expanded", !menu.hidden);
      break;
    }
    case "send":
      toast("Send preview · no message was sent.");
      break;
    case "queue":
      toast("Queue only · would save to Outbox for manual sending.");
      document.getElementById("send-menu").hidden = true;
      break;
    case "attach":
      toast("Attachment control · opens the native file picker in the app.");
      break;
    case "signature":
      toast("Signature control · chooses a signature for this draft.");
      break;
    case "discard":
      toast("Discard control · the prototype keeps all sample drafts.");
      break;
  }
});
canvas.addEventListener("dblclick", (event) => {
  if (event.target.closest("[data-draft]")) setView("floating");
});
canvas.addEventListener("keydown", (event) => {
  const row = event.target.closest("[data-draft]");
  if (row && event.key === "Enter") {
    selected = Number(row.dataset.draft);
    setView("floating");
  }
  if (
    (event.metaKey || event.ctrlKey) &&
    event.key === "Enter" &&
    event.target.closest(".composer")
  ) {
    event.preventDefault();
    toast("Send preview · no message was sent.");
  }
});
canvas.addEventListener("input", (event) => {
  if (event.target.id === "search") {
    const start = event.target.selectionStart;
    remember();
    query = event.target.value;
    render();
    const search = document.getElementById("search");
    search.focus();
    if (start !== null && search.type !== "search")
      search.setSelectionRange(start, start);
  } else if (event.target.closest(".composer")) {
    remember();
    document.getElementById("save-state").textContent =
      "Saved for this preview";
  }
});
new ResizeObserver(fit).observe(stage);
render();
