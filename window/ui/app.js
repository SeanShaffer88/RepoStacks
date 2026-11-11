// RepoStacks window — all data comes from core operations via invoke().
const { invoke } = window.__TAURI__.core;
const listen = window.__TAURI__.event ? window.__TAURI__.event.listen : null;

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => [...document.querySelectorAll(sel)];

const state = {
  screen: "catalogue",
  entries: [],
  volumes: 0,       // total catalogue size, for empty-state wording
  sort: localStorage.getItem("repostacks-sort") || "",
  view: { layout: "ledger", theme: "dark-ledger", density: "comfortable", group_by: "none", columns: [], show_facets: true },
  facetQuery: "",   // composed from facet checkboxes
  detailRepo: null,
};

function toast(msg, isError) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = isError ? "error" : "";
  clearTimeout(toast._timer);
  toast._timer = setTimeout(() => t.classList.add("hidden"), 4200);
}

async function call(cmd, args) {
  try {
    return await invoke(cmd, args || {});
  } catch (e) {
    toast(String(e), true);
    throw e;
  }
}

// Deterministic colour for a language "cover".
const LANG_COLORS = {
  Rust: "#dea584", Go: "#00add8", Python: "#3776ab", TypeScript: "#3178c6",
  JavaScript: "#f1e05a", C: "#a8b9cc", "C++": "#f34b7d", "C#": "#178600",
  Java: "#b07219", PHP: "#4f5d95", Ruby: "#701516", Swift: "#f05138",
  Kotlin: "#a97bff", Lua: "#000080", Shell: "#89e051", PowerShell: "#012456",
};
function coverColor(lang, name) {
  if (lang && LANG_COLORS[lang]) return LANG_COLORS[lang];
  let h = 0;
  const s = (lang || name || "?");
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
  return `hsl(${h}, 45%, 42%)`;
}

function statusBadge(e) {
  if (e.status === "Installed") return `<span class="badge installed">✔ Installed</span>`;
  if (e.status === "Update available") return `<span class="badge update">⬆ Update</span>`;
  if (e.status === "Not installed") return `<span class="badge">Not installed</span>`;
  return `<span class="badge">${e.status}</span>`;
}
function fmtStars(n) {
  return n >= 1000 ? (n / 1000).toFixed(1).replace(/\.0$/, "") + "k" : String(n);
}
function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

// ---------- Navigation ----------
function showScreen(name) {
  state.screen = name;
  $$("#main-nav button").forEach((b) => {
    const active = b.dataset.screen === name;
    b.classList.toggle("active", active);
    if (active) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  });
  ["catalogue", "detail", "shelves", "myshelf", "acquisitions", "librarian"].forEach((s) => {
    $("#screen-" + s).classList.toggle("hidden", s !== name);
  });
  if (name === "catalogue") refreshCatalogue();
  if (name === "shelves") refreshShelves();
  if (name === "myshelf") refreshMyShelf();
  if (name === "acquisitions") refreshQueue();
  if (name === "librarian") refreshLibrarian();
}
$$("#main-nav button").forEach((b) => b.addEventListener("click", () => showScreen(b.dataset.screen)));

// ---------- Catalogue ----------
function currentQuery() {
  const parts = [$("#global-search").value.trim(), state.facetQuery].filter(Boolean);
  const joined = parts.join(" ");
  // The toolbar sort applies unless the typed query carries its own sort:.
  if (state.sort && !/(^|\s)sort:/.test(joined)) {
    return (joined + " sort:" + state.sort).trim();
  }
  return joined;
}

async function refreshCatalogue() {
  const q = currentQuery();
  state.entries = await call("find_entries", { query: q });
  const ov = await call("overview");
  state.volumes = ov.volumes;
  $("#result-count").textContent = `${state.entries.length} volume(s)`;
  renderEntries();
}
$("#sort-select").addEventListener("change", (ev) => {
  state.sort = ev.target.value;
  localStorage.setItem("repostacks-sort", state.sort);
  refreshCatalogue();
});

function groupEntries(entries) {
  const g = state.view.group_by;
  if (g === "none") return [[null, entries]];
  const keyOf = (e) =>
    g === "shelf" ? (e.shelves[0] || "—") :
    g === "language" ? (e.language || "—") :
    g === "license" ? (e.license || "—") : e.status;
  const map = new Map();
  for (const e of entries) {
    const k = keyOf(e);
    if (!map.has(k)) map.set(k, []);
    map.get(k).push(e);
  }
  return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
}

function renderEntries() {
  const host = $("#entries");
  const entries = state.entries;
  if (!entries.length) {
    // Two different empties: an empty library gets a call to action, a
    // filtered-to-nothing catalogue gets an explanation.
    if (state.volumes === 0) {
      host.innerHTML = `<div class="empty"><p>Your library is empty.</p><button id="empty-add" class="primary">+ Add your first volumes</button></div>`;
      $("#empty-add").addEventListener("click", openAddModal);
    } else {
      host.innerHTML = `<div class="empty">No volumes match. Loosen the filters or the search — or add more with “+ Add volumes”.</div>`;
    }
    return;
  }
  const layout = state.view.layout;
  let html = "";
  for (const [label, members] of groupEntries(entries)) {
    if (label !== null) html += `<h3 class="group-head">${esc(label)}</h3>`;
    if (layout === "ledger") html += renderLedger(members);
    else if (layout === "cards") html += renderCards(members);
    else if (layout === "compact") html += renderCompact(members);
    else html += renderDiagram(members);
  }
  host.innerHTML = html;
  host.querySelectorAll("[data-repo]").forEach((el) =>
    el.addEventListener("click", () => openDetail(el.dataset.repo)));
}

const COLUMNS = {
  title: { head: "Title", cell: (e) => `<span class="repo-link">${esc(e.full_name)}</span>` },
  owner: { head: "Owner", cell: (e) => esc(e.owner) },
  language: { head: "Language", cell: (e) => esc(e.language || "—") },
  license: { head: "License", cell: (e) => esc(e.license || "—") },
  stars: { head: "Stars", cell: (e) => fmtStars(e.stars), num: true },
  release: { head: "Latest Release", cell: (e) => esc(e.latest_tag || "—") },
  installed: { head: "Installed", cell: (e) => esc(e.installed_tag || "—") },
  status: { head: "Status", cell: (e) => statusBadge(e) },
  topics: { head: "Topics", cell: (e) => esc(e.topics.join(", ") || "—") },
  shelf: { head: "Shelf", cell: (e) => esc(e.shelves.join(", ") || "—") },
};

function renderLedger(members) {
  const cols = (state.view.columns.length ? state.view.columns : Object.keys(COLUMNS))
    .filter((c) => COLUMNS[c]);
  let h = `<table class="ledger"><thead><tr>${cols.map((c) => `<th class="${COLUMNS[c].num ? "num" : ""}">${COLUMNS[c].head}</th>`).join("")}</tr></thead><tbody>`;
  for (const e of members) {
    h += `<tr class="entry" data-repo="${esc(e.full_name)}">${cols.map((c) => `<td class="${COLUMNS[c].num ? "num" : ""}">${COLUMNS[c].cell(e)}</td>`).join("")}</tr>`;
  }
  return h + "</tbody></table>";
}

function renderCards(members) {
  let h = `<div class="cards">`;
  for (const e of members) {
    h += `<div class="card" data-repo="${esc(e.full_name)}">
      <div class="cover" style="background:${coverColor(e.language, e.name)}"><span>${esc(e.language || "repo")}</span></div>
      <div class="name">${esc(e.name)}</div>
      <div class="owner">${esc(e.owner)}</div>
      <div class="meta">★ ${fmtStars(e.stars)} · ${esc(e.language || "—")} · ${esc(e.license || "—")}</div>
      <div class="meta">${esc(e.latest_tag || "no releases")}</div>
      <div class="tagrow">${statusBadge(e)}${e.auto_download ? '<span class="badge">standing order</span>' : ""}</div>
    </div>`;
  }
  return h + "</div>";
}

function renderCompact(members) {
  return members.map((e) =>
    `<div class="compact-row" data-repo="${esc(e.full_name)}"><strong>${esc(e.full_name)}</strong>
     <span class="muted">${esc(e.language || "—")} · ★${fmtStars(e.stars)} · ${esc(e.latest_tag || "—")} · ${esc(e.status)}</span></div>`).join("");
}

function renderDiagram(members) {
  // Diagram groups by shelf regardless of the group-by setting.
  const map = new Map();
  for (const e of members) {
    const keys = e.shelves.length ? e.shelves : ["Unsorted"];
    for (const k of keys) {
      if (!map.has(k)) map.set(k, []);
      map.get(k).push(e);
    }
  }
  let h = "";
  for (const [zone, list] of [...map.entries()].sort()) {
    h += `<div class="zone"><h3>${esc(zone)}</h3><div class="minis">`;
    for (const e of list) {
      h += `<div class="mini" data-repo="${esc(e.full_name)}">
        <div class="mini-head" style="background:${coverColor(e.language, e.name)}">${esc(e.full_name)}</div>
        <div class="mini-body">language ${esc(e.language || "—")} · license ${esc(e.license || "—")}<br>
        stars ${fmtStars(e.stars)} · release ${esc(e.latest_tag || "—")}<br>status ${esc(e.status)}</div></div>`;
    }
    h += "</div></div>";
  }
  return h;
}

// Facets
async function refreshFacets() {
  const f = await call("facets");
  const fill = (host, items, kind) => {
    host.innerHTML = items.map((x) =>
      `<label><input type="checkbox" data-facet="${kind}:${esc(x.name)}"> ${esc(x.name)} <span class="count">${x.count}</span></label>`).join("");
  };
  fill($("#facet-languages"), f.languages, "lang");
  fill($("#facet-topics"), f.topics, "topic");
  fill($("#facet-licenses"), f.licenses, "license");
  $$("#facet-panel input[type=checkbox]").forEach((cb) =>
    cb.addEventListener("change", composeFacetQuery));
}
function composeFacetQuery() {
  const terms = [];
  $$("#facet-panel input[type=checkbox]:checked").forEach((cb) => {
    if (cb.dataset.facet) terms.push(cb.dataset.facet);
    if (cb.dataset.status) terms.push(cb.dataset.status);
  });
  state.facetQuery = terms.join(" ");
  refreshCatalogue();
}
$("#clear-filters").addEventListener("click", () => {
  $$("#facet-panel input[type=checkbox]").forEach((cb) => (cb.checked = false));
  $("#global-search").value = "";
  state.facetQuery = "";
  refreshCatalogue();
});
let searchTimer;
$("#global-search").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(refreshCatalogue, 180);
});
$("#check-now").addEventListener("click", async () => {
  toast("Checking every volume via Atom feeds…");
  const res = await call("check_all");
  const changed = res.filter((r) => r.changed).length;
  toast(`Checked ${res.length} volume(s) — ${changed} new release(s). No API quota used.`);
  refreshCatalogue();
});
$("#refresh-meta").addEventListener("click", async () => {
  toast("Refreshing metadata…");
  const r = await call("refresh_all", { force: false });
  toast(`Metadata: ${r.updated} updated, ${r.unchanged} unchanged, ${r.skipped} cached — ${r.spent}/${r.limit} API calls this hour.`);
  refreshFacets();
  refreshCatalogue();
});

// View controls
async function applyView() {
  const v = state.view;
  document.documentElement.dataset.theme = v.theme;
  document.documentElement.dataset.density = v.density;
  $("#view-layout").value = v.layout;
  $("#view-theme").value = v.theme;
  $("#view-density").value = v.density;
  $("#view-group").value = v.group_by;
  await call("set_view", { view: v });
  renderEntries();
}
for (const [id, key] of [["#view-layout", "layout"], ["#view-theme", "theme"], ["#view-density", "density"], ["#view-group", "group_by"]]) {
  $(id).addEventListener("change", (ev) => { state.view[key] = ev.target.value; applyView(); });
}

// ---------- Detail ----------
async function openDetail(fullName) {
  state.detailRepo = fullName;
  const all = await call("find_entries", { query: "" });
  const e = all.find((x) => x.full_name.toLowerCase() === fullName.toLowerCase());
  if (!e) return;
  const host = $("#screen-detail");
  host.innerHTML = `
    <button id="detail-back">← Catalogue</button>
    <div class="detail-head">
      <div class="detail-cover" style="background:${coverColor(e.language, e.name)}"><span>${esc(e.language || "repo")}</span></div>
      <div>
        <h2>${esc(e.name)}</h2>
        <div class="sub">${esc(e.full_name)} · ★ ${fmtStars(e.stars)} · ⑂ —</div>
        <p>${esc(e.description || "No description fetched yet — Refresh metadata fills this in.")}</p>
        <div class="sub">${esc(e.language || "—")} · ${esc(e.license || "—")}${e.topics.length ? " · " + esc(e.topics.join(", ")) : ""}</div>
        <div class="chips">${statusBadge(e)} ${e.installed_tag ? `<span class="chip">installed ${esc(e.installed_tag)}</span>` : ""} ${e.latest_tag ? `<span class="chip">latest ${esc(e.latest_tag)}</span>` : ""}</div>
        <div class="row">
          <button id="detail-download" class="primary">⬇ Download ${esc(e.latest_tag || "")}</button>
          <button id="detail-check">Check for updates</button>
          <button id="detail-remove">Remove volume</button>
        </div>
      </div>
    </div>
    <div class="detail-grid">
      <div class="panel">
        <h3>Shelves</h3>
        <div class="chips" id="detail-shelves"></div>
        <div class="row"><input id="detail-shelf-input" placeholder="Shelf name"> <button id="detail-shelf-add">Add to shelf</button></div>
        <h3>Standing order</h3>
        <label class="check"><input type="checkbox" id="detail-auto" ${e.auto_download ? "checked" : ""}> Auto-download new releases (the Librarian)</label>
      </div>
      <div class="panel">
        <h3>Asset rules (auto-download pick)</h3>
        <textarea id="detail-rules" rows="4" placeholder="One glob per line, e.g. *-x86_64-pc-windows-msvc.zip">${esc(e.asset_patterns.join("\n"))}</textarea>
        <div class="row"><button id="detail-rules-save">Save rules</button>
        <span class="muted">No rules = the Windows-first default pick.</span></div>
      </div>
      <div class="panel">
        <h3>Release history</h3>
        <div id="detail-history"><span class="muted">Loading…</span></div>
      </div>
      <div class="panel">
        <h3>Assets — ${esc(e.latest_tag || "no release known")}</h3>
        <div id="detail-assets"><span class="muted">${e.latest_tag ? "Loading…" : "Check for releases first."}</span></div>
      </div>
    </div>`;
  showScreen("detail");
  $("#detail-back").addEventListener("click", () => showScreen("catalogue"));
  const renderShelves = () => {
    $("#detail-shelves").innerHTML = e.shelves.map((s) =>
      `<span class="chip">${esc(s)}<button data-shelf="${esc(s)}" title="Remove from shelf">✕</button></span>`).join("") || '<span class="muted">Not on any shelf.</span>';
    $$("#detail-shelves button").forEach((b) => b.addEventListener("click", async () => {
      await call("shelf_remove", { shelf: b.dataset.shelf, repo: e.full_name });
      e.shelves = e.shelves.filter((s) => s !== b.dataset.shelf);
      renderShelves();
    }));
  };
  renderShelves();
  $("#detail-shelf-add").addEventListener("click", async () => {
    const shelf = $("#detail-shelf-input").value.trim();
    if (!shelf) return;
    await call("shelf_add", { shelf, repo: e.full_name });
    if (!e.shelves.includes(shelf)) e.shelves.push(shelf);
    renderShelves();
    toast(`On shelf “${shelf}”.`);
  });
  $("#detail-auto").addEventListener("change", async (ev) => {
    await call("set_auto", { repo: e.full_name, on: ev.target.checked });
    toast(ev.target.checked ? "Standing order on." : "Standing order off.");
  });
  $("#detail-rules-save").addEventListener("click", async () => {
    const patterns = $("#detail-rules").value.split("\n").map((s) => s.trim()).filter(Boolean);
    await call("set_rules", { repo: e.full_name, patterns });
    toast("Asset rules saved.");
  });
  $("#detail-download").addEventListener("click", async () => {
    toast(`Downloading ${e.full_name} ${e.latest_tag}…`);
    const path = await call("download_repo", { repo: e.full_name, tag: e.latest_tag || null });
    toast(`Downloaded → ${path}`);
    openDetail(e.full_name);
  });
  $("#detail-check").addEventListener("click", async () => {
    await call("check_all");
    openDetail(e.full_name);
  });
  $("#detail-remove").addEventListener("click", async (ev) => {
    // Removing a volume is destructive, so the button arms first:
    // one click asks, the second confirms. No dialog to dismiss.
    const btn = ev.currentTarget;
    if (btn.dataset.armed !== "1") {
      btn.dataset.armed = "1";
      btn.textContent = "Remove — click again to confirm";
      setTimeout(() => {
        if (btn.isConnected) {
          btn.dataset.armed = "";
          btn.textContent = "Remove volume";
        }
      }, 3500);
      return;
    }
    await call("remove_repo", { repo: e.full_name });
    toast("Volume removed.");
    showScreen("catalogue");
  });
  if (e.latest_tag) {
    const hist = await call("release_history", { repo: e.full_name });
    $("#detail-history").innerHTML = hist.slice(0, 12).map((n, i) =>
      `<div class="release-row"><span class="tag">${esc(n.tag)}</span><span class="muted">${esc(n.updated.slice(0, 10))}</span>${i === 0 ? '<span class="badge installed">latest</span>' : ""}<span>${esc(n.title !== n.tag ? n.title : "")}</span></div>`).join("") || '<span class="muted">No releases in the feed.</span>';
    const res = await call("release_assets", { repo: e.full_name, tag: e.latest_tag });
    $("#detail-assets").innerHTML = res.assets.map((a) =>
      `<div class="asset-row"><span>${esc(a.name)}</span><span class="muted">${(a.size / 1048576).toFixed(1)} MB</span>${a.name === res.pick ? '<span class="badge installed">auto-pick</span>' : ""}${a.digest ? '<span class="muted">sha256 ✓</span>' : ""}</div>`).join("") + `<div class="muted">Listing cost: ${res.quota} API call.${res.reason ? " Auto-pick: " + esc(res.reason) + "." : ""}</div>`;
  }
}

// ---------- Shelves screen ----------
async function refreshShelves() {
  const data = await call("list_shelves");
  const all = await call("find_entries", { query: "" });
  const byName = new Map(all.map((e) => [e.full_name.toLowerCase(), e]));
  const host = $("#shelf-list");
  if (!data.length) {
    host.innerHTML = `<div class="empty">No shelves yet — open a volume and add it to one.</div>`;
    return;
  }
  host.innerHTML = data.map((s) => `
    <div class="shelf-section">
      <h3>${esc(s.name)} <span class="muted">(${s.members.length})</span></h3>
      <div class="shelf-plank"></div>
      <div class="spines">${s.members.map((m) => {
        const e = byName.get(m.toLowerCase());
        return `<div class="spine" data-repo="${esc(m)}">
          <div class="spine-cover" style="background:${coverColor(e ? e.language : "", m)}"></div>
          <div class="spine-name">${esc(m.split("/")[1] || m)}</div>
          <div class="muted">${e ? esc(e.latest_tag || "") + " " + esc(e.status) : ""}</div>
        </div>`;
      }).join("")}</div>
    </div>`).join("");
  host.querySelectorAll("[data-repo]").forEach((el) =>
    el.addEventListener("click", () => openDetail(el.dataset.repo)));
}

// ---------- My Shelf ----------
async function refreshMyShelf() {
  const all = await call("find_entries", { query: "" });
  const updates = all.filter((e) => e.status === "Update available");
  const current = all.filter((e) => e.installed_tag && e.status !== "Update available");
  const row = (e, action) => `<div class="release-row"><span class="tag">${esc(e.name)}</span>
    <span class="muted">${esc(e.owner)}</span>
    <span>${e.installed_tag ? esc(e.installed_tag) : ""}${action ? ` → ${esc(e.latest_tag)}` : ""}</span>
    <span class="spacer"></span>${action ? `<button data-update="${esc(e.full_name)}">Update</button>` : statusBadge(e)}</div>`;
  $("#myshelf-body").innerHTML = `
    <h3 class="group-head">⚠ ${updates.length} update(s) available</h3>
    ${updates.map((e) => row(e, true)).join("") || '<div class="empty">Everything is current.</div>'}
    <h3 class="group-head">✔ Up to date (${current.length})</h3>
    ${current.map((e) => row(e, false)).join("") || '<div class="empty">Nothing installed yet.</div>'}`;
  $$("#myshelf-body [data-update]").forEach((b) => b.addEventListener("click", async () => {
    toast(`Updating ${b.dataset.update}…`);
    await call("download_repo", { repo: b.dataset.update, tag: null });
    toast("Updated.");
    refreshMyShelf();
  }));
}
$("#update-all").addEventListener("click", async () => {
  const all = await call("find_entries", { query: "has:update" });
  if (!all.length) { toast("Nothing to update."); return; }
  toast(`Updating ${all.length} volume(s)…`);
  for (const e of all) {
    await call("download_repo", { repo: e.full_name, tag: null });
  }
  toast("All updates downloaded.");
  refreshMyShelf();
});

// ---------- Acquisitions ----------
async function refreshQueue() {
  const jobs = await call("queue_list");
  const host = $("#queue-list");
  if (!jobs.length) {
    host.innerHTML = `<div class="empty">The queue is empty. Downloads you start land here.</div>`;
    return;
  }
  host.innerHTML = jobs.map((j) => `
    <div class="queue-row">
      <span class="tag">#${j.id}</span>
      <span><strong>${esc(j.repo)}</strong> ${esc(j.tag)} · ${esc(j.asset_name)}</span>
      <span class="bar" role="progressbar" aria-label="Download progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${j.percent.toFixed(0)}"><div style="width:${j.percent.toFixed(0)}%"></div></span>
      <span class="muted">${j.percent.toFixed(0)}% · ${(j.bytes_done / 1048576).toFixed(1)}/${(j.bytes_total / 1048576).toFixed(1)} MB</span>
      <span class="badge">${esc(j.status)}</span>
      ${j.error ? `<span class="muted">${esc(j.error)}</span>` : ""}
      ${j.status === "downloading" || j.status === "queued" ? `<button data-act="pause" data-id="${j.id}" title="Pause" aria-label="Pause job ${j.id}">⏸</button>` : ""}
      ${j.status === "paused" ? `<button data-act="resume" data-id="${j.id}" title="Resume" aria-label="Resume job ${j.id}">▶</button>` : ""}
      ${j.status === "failed" ? `<button data-act="retry" data-id="${j.id}" title="Retry" aria-label="Retry job ${j.id}">↻</button>` : ""}
      ${["queued", "paused", "downloading"].includes(j.status) ? `<button data-act="cancel" data-id="${j.id}" title="Cancel" aria-label="Cancel job ${j.id}">✕</button>` : ""}
    </div>`).join("");
  host.querySelectorAll("[data-act]").forEach((b) => b.addEventListener("click", async () => {
    await call("queue_" + b.dataset.act, { id: Number(b.dataset.id) });
    refreshQueue();
  }));
}
$("#queue-run").addEventListener("click", async () => {
  toast("Running the queue…");
  const lines = await call("queue_run");
  toast(lines.length ? lines.join(" · ") : "Nothing queued.");
  refreshQueue();
});
$("#queue-clear").addEventListener("click", async () => {
  const n = await call("queue_clear");
  toast(`Cleared ${n} finished job(s).`);
  refreshQueue();
});

// ---------- Librarian ----------
async function refreshLibrarian() {
  const cfg = await call("get_config");
  $("#cfg-atom").value = Math.round(cfg.atom_poll_secs / 60);
  $("#cfg-meta").value = Math.round(cfg.metadata_poll_secs / 60);
  $("#cfg-conc").value = cfg.concurrency;
  $("#cfg-dir").value = cfg.download_dir;
  $("#token-state").textContent = cfg.has_token
    ? "A token is stored (DPAPI-protected) — API budget 5,000/hour."
    : "No token stored — anonymous mode, API budget 60/hour. Release checks are free either way.";
  const lines = await call("get_activity");
  $("#activity-log").innerHTML = lines.slice().reverse().map((l) => `<div>${esc(l)}</div>`).join("");
}
$("#cfg-save").addEventListener("click", async () => {
  await call("set_config", {
    config: {
      download_dir: $("#cfg-dir").value,
      concurrency: Number($("#cfg-conc").value) || 3,
      atom_poll_secs: (Number($("#cfg-atom").value) || 360) * 60,
      metadata_poll_secs: (Number($("#cfg-meta").value) || 1440) * 60,
      has_token: false,
    },
  });
  toast("Librarian settings saved.");
});
$("#token-set").addEventListener("click", async () => {
  const tok = $("#cfg-token").value.trim();
  if (!tok) { toast("Paste a token first.", true); return; }
  await call("set_token", { token: tok });
  $("#cfg-token").value = "";
  toast("Token stored, DPAPI-protected.");
  refreshLibrarian();
});
$("#token-clear").addEventListener("click", async () => {
  await call("clear_token");
  toast("Token removed.");
  refreshLibrarian();
});
$("#librarian-now").addEventListener("click", async () => {
  toast("The Librarian is making a pass…");
  const lines = await call("librarian_now");
  toast(lines.length ? `Pass done: ${lines.length} thing(s) happened.` : "Pass done — nothing new.");
  refreshLibrarian();
  refreshCatalogue();
});
if (listen) {
  listen("librarian-activity", (ev) => {
    toast("Librarian: " + ev.payload.join(" · "));
    if (state.screen === "librarian") refreshLibrarian();
    if (state.screen === "acquisitions") refreshQueue();
  });
}

// ---------- Add modal ----------
let lastFocus = null;
function openAddModal() {
  lastFocus = document.activeElement;
  $("#add-modal").classList.remove("hidden");
  $("#add-report").innerHTML = "";
  $("#add-single").focus();
}
function closeAddModal() {
  $("#add-modal").classList.add("hidden");
  if (lastFocus && lastFocus.isConnected) lastFocus.focus();
}
$("#add-btn").addEventListener("click", openAddModal);
$("#add-close").addEventListener("click", closeAddModal);
$("#add-cancel").addEventListener("click", closeAddModal);

// ---------- Keyboard ----------
// Ctrl+K jumps to the search box from anywhere; "/" does the same when
// not already typing; Esc closes the Add modal.
document.addEventListener("keydown", (ev) => {
  const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName || "");
  if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === "k") {
    ev.preventDefault();
    $("#global-search").focus();
    $("#global-search").select();
  } else if (ev.key === "/" && !typing) {
    ev.preventDefault();
    $("#global-search").focus();
  } else if (ev.key === "Escape" && !$("#add-modal").classList.contains("hidden")) {
    closeAddModal();
  }
});
$("#add-go").addEventListener("click", async () => {
  const single = $("#add-single").value.trim();
  const bulk = $("#add-bulk").value;
  const shelf = $("#add-shelf").value.trim() || null;
  if (single && !bulk.trim()) {
    const dto = await call("add_repo", { input: single });
    $("#add-report").innerHTML = `Added <strong>${esc(dto.full_name)}</strong>.`;
  } else {
    const text = [single, bulk].filter(Boolean).join("\n");
    const r = await call("import_apply", { text, shelf });
    $("#add-report").innerHTML =
      `Preview: <strong>${r.added.length} added</strong>, ${r.duplicates.length} duplicate(s), ${r.invalid.length} invalid.` +
      (r.invalid.length ? "<br>" + r.invalid.map((p) => `✗ ${esc(p[0])} — ${esc(p[1])}`).join("<br>") : "");
  }
  $("#add-single").value = "";
  refreshFacets();
  refreshCatalogue();
});

// ---------- Boot ----------
(async () => {
  try {
    state.view = await invoke("get_view");
  } catch { /* defaults stay */ }
  document.documentElement.dataset.theme = state.view.theme;
  document.documentElement.dataset.density = state.view.density;
  $("#sort-select").value = state.sort;
  $("#view-layout").value = state.view.layout;
  $("#view-theme").value = state.view.theme;
  $("#view-density").value = state.view.density;
  $("#view-group").value = state.view.group_by;
  await refreshFacets();
  await refreshCatalogue();
  const ov = await call("overview");
  if (ov.updates > 0) toast(`${ov.updates} update(s) waiting on My Shelf.`);
})();
