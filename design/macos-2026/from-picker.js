const identities = [
  {
    id: "personal",
    name: "Alex Lane",
    email: "alex@example.com",
    label: "Personal",
    initials: "AL",
    tone: "",
  },
  {
    id: "studio",
    name: "Alex Lane",
    email: "alex@studio.example",
    label: "Studio",
    initials: "AL",
    tone: "studio",
  },
  {
    id: "notes",
    name: "Alex Lane",
    email: "notes@example.org",
    label: "Writing",
    initials: "AL",
    tone: "notes",
  },
];
const paths = {
  down: "m6 9 6 6 6-6",
  check: "m4 12 5 5L20 6",
  clip: "m8 13 7-7a3 3 0 0 1 4 4L9 20a5 5 0 0 1-7-7L13 2 M5 16 15 6",
  send: "m22 2-7 20-4-9L2 9z M11 13 22 2",
  search: "M16 10a6 6 0 1 1-12 0 6 6 0 0 1 12 0 m-2 4 7 7",
  trash: "M4 6h16 M9 6V3h6v3 M6 6l1 15h10l1-15 M10 10v7 M14 10v7",
};
const icon = (name, small = false) =>
  `<svg class="icon${small ? " small" : ""}" viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name]}"/></svg>`;
const state = { compact: "personal", detailed: "personal" };
function windowHTML(kind) {
  return `<div class="window" data-kind="${kind}">
    <div class="titlebar"><div class="lights" aria-hidden="true"><i></i><i></i><i></i></div><div class="title">New message<div class="saved">✓ &nbsp; Draft saved locally</div></div><span class="spacer"></span><button class="attach" data-demo="Attach files" aria-label="Attach files">${icon("clip")}</button><div class="send"><button data-demo="Send">${icon("send")}Send</button><button data-demo="Send options" aria-label="Send options">${icon("down", true)}</button></div></div>
    <div class="envelope"><div class="field"><span class="label">From</span><div class="sender-wrap"><button class="sender" id="${kind}-trigger" aria-expanded="true" aria-haspopup="listbox" aria-controls="${kind}-list"><span class="sender-content"></span>${icon("down", true)}</button>
    <div class="popover ${kind}-menu" id="${kind}-popover">${kind === "detailed" ? `<label class="filter">${icon("search", true)}<input aria-label="Find an identity" placeholder="Find an identity…" autocomplete="off"></label><div class="menu-label">SENDING IDENTITIES</div>` : ""}<div class="options" role="listbox" aria-label="Sending identity" id="${kind}-list"></div><div class="menu-bottom"><button class="manage" data-demo="Manage identities">Manage identities…<span class="dots">↗</span></button></div></div>
    </div></div><div class="field"><span class="label">To</span><span class="to-chip">Nora Reed</span><button class="cc" data-demo="Cc / Bcc">Cc / Bcc</button></div><div class="field"><span class="label">Subject</span><span class="subject">A small idea for the weekend</span></div></div>
    <div class="body" contenteditable="true" role="textbox" aria-label="Sample message for ${kind} menu" aria-multiline="true" spellcheck="false"><p>Hi Nora,</p><p>What if we took the morning train, found somewhere by the water,<br>and left the rest unplanned?</p><p>No rush — let me know what you think.</p><p>Alex</p></div><div class="bottom">${icon("clip")}<span>Plain text</span><span class="spacer"></span><button data-demo="Save and close">Save & close</button><button data-demo="Discard draft" aria-label="Discard draft">${icon("trash", true)}</button></div></div>`;
}
function renderOptions(kind, query = "") {
  const list = document.getElementById(`${kind}-list`);
  const matches = identities.filter((i) =>
    (i.name + " " + i.email + " " + i.label)
      .toLowerCase()
      .includes(query.toLowerCase()),
  );
  list.innerHTML =
    matches
      .map(
        (i) =>
          `<button class="option" role="option" aria-selected="${state[kind] === i.id}" data-identity="${i.id}">${kind === "compact" ? `<span class="check">${icon("check")}</span><span class="option-copy"><b>${i.name}</b> <span class="option-address">&lt;${i.email}&gt;</span></span>` : `<span class="initials ${i.tone}">${i.initials}</span><span class="option-copy"><span class="identity-title"><b>${i.name}</b><small>${i.label}</small></span><span class="option-address">${i.email}</span></span><span class="check">${icon("check")}</span>`}</button>`,
      )
      .join("") || '<div class="empty">No matching identities</div>';
}
function renderTrigger(kind) {
  const i = identities.find((i) => i.id === state[kind]);
  document.querySelector(`#${kind} .sender-content`).innerHTML =
    `${i.name} <span class="address">&lt;${i.email}&gt;</span>`;
}
function toggle(kind, open) {
  const trigger = document.getElementById(`${kind}-trigger`);
  document.getElementById(`${kind}-popover`).hidden = !open;
  trigger.setAttribute("aria-expanded", String(open));
  if (open) {
    const input = document.querySelector(`#${kind} .filter input`);
    if (input) input.value = "";
    renderOptions(kind);
  }
}
function fit() {
  document.querySelectorAll(".preview").forEach((preview) => {
    const scale = preview.clientWidth / 640;
    preview.style.height = `${492 * scale}px`;
    preview.querySelector(".window").style.transform = `scale(${scale})`;
  });
}
for (const kind of ["compact", "detailed"]) {
  const preview = document.getElementById(kind);
  preview.innerHTML = windowHTML(kind);
  renderTrigger(kind);
  renderOptions(kind);
  new ResizeObserver(fit).observe(preview);
  preview
    .querySelector(".sender")
    .addEventListener("click", () =>
      toggle(kind, document.getElementById(`${kind}-popover`).hidden),
    );
  preview
    .querySelector(".filter input")
    ?.addEventListener("input", (event) =>
      renderOptions(kind, event.target.value),
    );
  preview.addEventListener("click", (event) => {
    const option = event.target.closest("[data-identity]");
    if (option) {
      state[kind] = option.dataset.identity;
      renderTrigger(kind);
      renderOptions(kind);
      toggle(kind, false);
      document.getElementById(`${kind}-trigger`).focus();
    }
  });
  preview.addEventListener("keydown", (event) => {
    const menu = document.getElementById(`${kind}-popover`);
    if (event.key === "Escape" && !menu.hidden) {
      event.preventDefault();
      toggle(kind, false);
      document.getElementById(`${kind}-trigger`).focus();
    }
    if (
      (event.key === "ArrowDown" || event.key === "ArrowUp") &&
      event.target.closest(".sender-wrap")
    ) {
      event.preventDefault();
      if (menu.hidden) toggle(kind, true);
      const options = [...preview.querySelectorAll(".option")];
      if (!options.length) return;
      const index = options.indexOf(document.activeElement);
      const next =
        index < 0
          ? event.key === "ArrowDown"
            ? 0
            : options.length - 1
          : (index + (event.key === "ArrowDown" ? 1 : -1) + options.length) %
            options.length;
      options[next].focus();
    }
    if (event.key === "Enter" && event.target.matches(".filter input")) {
      event.preventDefault();
      preview.querySelector(".option")?.click();
    }
  });
}
document.addEventListener("click", (event) => {
  for (const kind of ["compact", "detailed"])
    if (
      !event.target.closest(`#${kind} .sender-wrap`) &&
      !event.target.closest("#theme")
    )
      toggle(kind, false);
});
document.getElementById("theme").addEventListener("click", (event) => {
  const dark = document.body.classList.toggle("dark");
  event.target.textContent = dark ? "Light appearance" : "Dark appearance";
});
let timer;
function notify(message) {
  const notice = document.getElementById("notice");
  notice.textContent = message;
  notice.classList.add("visible");
  clearTimeout(timer);
  timer = setTimeout(() => notice.classList.remove("visible"), 5500);
}
document
  .querySelectorAll("[data-demo]")
  .forEach((button) =>
    button.addEventListener("click", () =>
      notify(`${button.dataset.demo} is a visual control in this mockup.`),
    ),
  );
document.querySelectorAll("[data-pick]").forEach((button) =>
  button.addEventListener("click", async () => {
    const kind = button.dataset.pick;
    const summary = `Moonmail From menu: I choose ${kind === "compact" ? "01 — Compact native" : "02 — Identity picker"}.`;
    const data = {
      variant: kind,
      appearance: document.body.classList.contains("dark") ? "dark" : "light",
    };
    if (window.herdr?.send) {
      window.herdr.send(summary, data);
      notify("Choice ready in the notes panel — press Send to agent.");
    } else {
      try {
        await navigator.clipboard.writeText(summary);
        notify("Choice copied. Paste it into the conversation.");
      } catch {
        notify(summary + " Tell me your choice in the conversation.");
      }
    }
  }),
);
fit();
