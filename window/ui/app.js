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
  selectMode: false,
  selected: new Set(),
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
  return `<span class="badge">${esc(e.status)}</span>`;
}
function fmtStars(n) {
  return n >= 1000 ? (n / 1000).toFixed(1).replace(/\.0$/, "") + "k" : String(n);
}
function fmtBytes(n) {
  if (n >= 1048576) return (n / 1048576).toFixed(1) + " MB";
  if (n >= 1024) return Math.round(n / 1024) + " KB";
  return n + " B";
}
function fmtEta(secs) {
  if (secs >= 3600) return Math.round(secs / 3600) + "h";
  if (secs >= 60) return Math.round(secs / 60) + "m";
  return secs + "s";
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
  bindEntryOpen(host);
  host.querySelectorAll("input[data-sel]").forEach((cb) =>
    cb.addEventListener("change", () => {
      if (cb.checked) state.selected.add(cb.dataset.sel);
      else state.selected.delete(cb.dataset.sel);
      updateBulkBar();
    }));
}

// The selection checkbox rendered on each entry while Select mode is
// on. Clicks on it must not open the volume — bindEntryOpen guards.
function selBox(e) {
  if (!state.selectMode) return "";
  const on = state.selected.has(e.full_name) ? " checked" : "";
  return `<label class="selbox" title="Select ${esc(e.full_name)}"><input type="checkbox" data-sel="${esc(e.full_name)}"${on}></label>`;
}

function updateBulkBar() {
  $("#bulk-bar").classList.toggle("hidden", !state.selectMode);
  $("#bulk-count").textContent = state.selectMode ? `${state.selected.size} selected` : "";
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
  const selHead = state.selectMode ? `<th class="selcol"></th>` : "";
  let h = `<table class="ledger"><thead><tr>${selHead}${cols.map((c) => `<th class="${COLUMNS[c].num ? "num" : ""}">${COLUMNS[c].head}</th>`).join("")}</tr></thead><tbody>`;
  for (const e of members) {
    const selCell = state.selectMode ? `<td class="selcol">${selBox(e)}</td>` : "";
    h += `<tr class="entry" data-repo="${esc(e.full_name)}">${selCell}${cols.map((c) => `<td class="${COLUMNS[c].num ? "num" : ""}">${COLUMNS[c].cell(e)}</td>`).join("")}</tr>`;
  }
  return h + "</tbody></table>";
}

function renderCards(members) {
  let h = `<div class="cards">`;
  for (const e of members) {
    h += `<div class="card" data-repo="${esc(e.full_name)}">${selBox(e)}
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
    `<div class="compact-row" data-repo="${esc(e.full_name)}">${selBox(e)}<strong>${esc(e.full_name)}</strong>
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
      h += `<div class="mini" data-repo="${esc(e.full_name)}">${selBox(e)}
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
  $$("#facet-languages input[type=checkbox], #facet-topics input[type=checkbox], #facet-licenses input[type=checkbox], #facet-status input[type=checkbox]").forEach((cb) =>
    cb.addEventListener("change", composeFacetQuery));
}
function composeFacetQuery() {
  const terms = [];
  $$("#facet-languages input[type=checkbox]:checked, #facet-topics input[type=checkbox]:checked, #facet-licenses input[type=checkbox]:checked, #facet-status input[type=checkbox]:checked").forEach((cb) => {
    if (cb.dataset.facet) terms.push(cb.dataset.facet);
    if (cb.dataset.status) terms.push(cb.dataset.status);
  });
  state.facetQuery = terms.join(" ");
  refreshCatalogue();
}
$("#clear-filters").addEventListener("click", () => {
  $$("#facet-languages input[type=checkbox], #facet-topics input[type=checkbox], #facet-licenses input[type=checkbox], #facet-status input[type=checkbox]").forEach((cb) => (cb.checked = false));
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
  const failed = res.filter((r) => r.error);
  // Failures are said out loud, not left in the log: name the count and
  // the first reason so a dead feed is visible at a glance.
  toast(
    `Checked ${res.length} volume(s) — ${changed} new release(s). No API quota used.` +
      (failed.length ? ` ${failed.length} failed: ${failed[0].repo} (${failed[0].error})` : ""),
    failed.length > 0
  );
  refreshCatalogue();
});
// ---------- Bulk selection ----------
$("#select-mode").addEventListener("click", () => {
  state.selectMode = !state.selectMode;
  const btn = $("#select-mode");
  btn.setAttribute("aria-pressed", String(state.selectMode));
  btn.textContent = state.selectMode ? "Selecting…" : "Select";
  if (!state.selectMode) state.selected.clear();
  updateBulkBar();
  renderEntries();
});
$("#bulk-clear").addEventListener("click", () => {
  state.selected.clear();
  updateBulkBar();
  renderEntries();
});
$("#bulk-to-shelf").addEventListener("click", async () => {
  const shelf = $("#bulk-shelf").value.trim();
  if (!shelf) { toast("Name the shelf first.", true); return; }
  if (!state.selected.size) { toast("Select some volumes first.", true); return; }
  const placed = await call("bulk_shelf_add", { shelf, repos: [...state.selected] });
  toast(`Put ${placed} volume(s) on shelf “${shelf}”.`);
  state.selected.clear();
  updateBulkBar();
  refreshCatalogue();
});
$("#bulk-check").addEventListener("click", async () => {
  if (!state.selected.size) { toast("Select some volumes first.", true); return; }
  toast(`Checking ${state.selected.size} volume(s) via Atom feeds…`);
  const res = await call("check_repos", { repos: [...state.selected] });
  const changed = res.filter((r) => r.changed).length;
  const failed = res.filter((r) => r.error);
  toast(
    `Checked ${res.length} volume(s) — ${changed} new release(s).` +
      (failed.length ? ` ${failed.length} failed: ${failed[0].repo} (${failed[0].error})` : ""),
    failed.length > 0
  );
  refreshCatalogue();
});
$("#bulk-remove").addEventListener("click", async (ev) => {
  if (!state.selected.size) { toast("Select some volumes first.", true); return; }
  // Same arm-to-confirm pattern as the volume page's Remove.
  const btn = ev.currentTarget;
  if (btn.dataset.armed !== "1") {
    btn.dataset.armed = "1";
    btn.textContent = `Remove ${state.selected.size} — click again to confirm`;
    setTimeout(() => {
      if (btn.isConnected) {
        btn.dataset.armed = "";
        btn.textContent = "Remove selected";
      }
    }, 3500);
    return;
  }
  btn.dataset.armed = "";
  btn.textContent = "Remove selected";
  const n = await call("bulk_remove", { repos: [...state.selected] });
  state.selected.clear();
  updateBulkBar();
  toast(`Removed ${n} volume(s).`);
  refreshFacets();
  refreshCatalogue();
});

$("#refresh-meta").addEventListener("click", async () => {
  toast("Refreshing metadata…");
  const r = await call("refresh_all", { force: false });
  toast(
    `Metadata: ${r.updated} updated, ${r.unchanged} unchanged, ${r.skipped} cached — ${r.spent}/${r.limit} API calls this hour.` +
      (r.errors.length ? ` ${r.errors.length} failed: ${r.errors[0]}` : ""),
    r.errors.length > 0
  );
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

// Ledger columns picker: state.view.columns empty means "all columns".
function buildColumnPicker() {
  const host = $("#view-columns");
  const active = state.view.columns.length ? state.view.columns : Object.keys(COLUMNS);
  host.innerHTML = Object.keys(COLUMNS).map((k) =>
    `<label><input type="checkbox" data-col="${k}" ${active.includes(k) ? "checked" : ""}> ${COLUMNS[k].head}</label>`).join("");
  host.querySelectorAll("input[data-col]").forEach((cb) =>
    cb.addEventListener("change", () => {
      const picked = [...host.querySelectorAll("input[data-col]:checked")].map((x) => x.dataset.col);
      if (!picked.length) {
        cb.checked = true;
        toast("Keep at least one column.", true);
        return;
      }
      state.view.columns = picked.length === Object.keys(COLUMNS).length ? [] : picked;
      applyView();
    }));
}

// ---------- Saved searches ----------
function fillSavedSelect(map) {
  const sel = $("#saved-select");
  const names = Object.keys(map || {});
  sel.innerHTML = `<option value="">Choose a saved search…</option>` +
    names.map((n) => `<option value="${esc(n)}">${esc(n)}</option>`).join("");
}
async function refreshSaved() {
  fillSavedSelect(await call("saved_list"));
}
$("#saved-select").addEventListener("change", async (ev) => {
  const name = ev.target.value;
  if (!name) return;
  const map = await call("saved_list");
  const q = map[name];
  if (q === undefined) return;
  $$("#facet-languages input[type=checkbox], #facet-topics input[type=checkbox], #facet-licenses input[type=checkbox], #facet-status input[type=checkbox]")
    .forEach((cb) => (cb.checked = false));
  state.facetQuery = "";
  $("#global-search").value = q;
  refreshCatalogue();
});
$("#saved-save").addEventListener("click", async () => {
  const name = $("#saved-name").value.trim();
  if (!name) { toast("Name the search first.", true); return; }
  const map = await call("saved_save", { name, query: currentQuery() });
  fillSavedSelect(map);
  $("#saved-select").value = name;
  $("#saved-name").value = "";
  toast(`Saved search "${name}".`);
});
$("#saved-delete").addEventListener("click", async () => {
  const name = $("#saved-select").value;
  if (!name) { toast("Choose a saved search to delete.", true); return; }
  fillSavedSelect(await call("saved_delete", { name }));
  toast(`Deleted saved search "${name}".`);
});

// ---------- Detail ----------
async function openDetail(fullName, skipAutoCheck) {
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
        <div class="sub">${esc(e.full_name)} · ★ ${fmtStars(e.stars)} · ⑂ ${e.forks ? fmtStars(e.forks) : "—"}</div>
        <p>${esc(e.description || "No description fetched yet — Refresh metadata fills this in.")}</p>
        <div class="sub">${esc(e.language || "—")} · ${esc(e.license || "—")}${e.topics.length ? " · " + esc(e.topics.join(", ")) : ""}</div>
        <div class="chips">${statusBadge(e)} ${e.installed_tag ? `<span class="chip">installed ${esc(e.installed_tag)}</span>` : ""} ${e.latest_tag ? `<span class="chip">latest ${esc(e.latest_tag)}</span>` : ""}</div>
        <div class="row">
          <button id="detail-download" class="primary">⬇ Download ${esc(e.latest_tag || "")}</button>
          <button id="detail-check">Check for updates</button>
          <button id="detail-github">Open on GitHub</button>
          ${e.installed_tag ? '<button id="detail-folder">Open download folder</button>' : ""}
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
        <button id="detail-rules-suggest">Suggest from assets</button>
        <span class="muted" id="detail-rules-note">No rules = the Windows-first default pick.</span></div>
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
  state.detailSuggested = [];
  $("#detail-rules-suggest").addEventListener("click", () => {
    if (state.detailSuggested.length) {
      $("#detail-rules").value = state.detailSuggested.join("\n");
      $("#detail-rules-note").textContent =
        "Suggested from these file names — Save rules to keep them. The first matching line is the one that downloads.";
    } else {
      toast("Suggestions come from a release's file names — check for a release first.", true);
    }
  });
  $("#detail-download").addEventListener("click", async () => {
    toast(`Downloading ${e.full_name} ${e.latest_tag}…`);
    const path = await call("download_repo", { repo: e.full_name, tag: e.latest_tag || null });
    toast(`Downloaded → ${path}`);
    openDetail(e.full_name, true);
  });
  $("#detail-check").addEventListener("click", async () => {
    const res = await call("check_repos", { repos: [e.full_name] });
    const mine = res.find((r) => r.repo.toLowerCase() === e.full_name.toLowerCase());
    if (mine && mine.error) toast(`${e.full_name}: ${mine.error}`, true);
    openDetail(e.full_name, true);
  });
  $("#detail-github").addEventListener("click", () =>
    call("open_repo_page", { repo: e.full_name }));
  const folderBtn = $("#detail-folder");
  if (folderBtn) {
    folderBtn.addEventListener("click", () =>
      call("open_download_folder", { repo: e.full_name }));
  }
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
  // A volume page never shows stale release data: opening it checks
  // just this repo (one free Atom call) and re-renders once if the
  // newest tag moved.
  if (!skipAutoCheck) {
    const res = await call("check_repos", { repos: [e.full_name] }).catch(() => null);
    const mine = res && res.find((r) => r.repo.toLowerCase() === e.full_name.toLowerCase());
    if (mine && !mine.error && mine.latest && mine.latest !== e.latest_tag) {
      return openDetail(e.full_name, true);
    }
  }
  if (e.latest_tag) {
    const hist = await call("release_history", { repo: e.full_name });
    $("#detail-history").innerHTML = hist.slice(0, 12).map((n, i) =>
      `<div class="release-row"><span class="tag">${esc(n.tag)}</span><span class="muted">${esc(n.updated.slice(0, 10))}</span>${i === 0 ? '<span class="badge installed">latest</span>' : ""}<span>${esc(n.title !== n.tag ? n.title : "")}</span>${n.body ? `<span class="spacer"></span><button class="small" data-notes="${i}">Notes</button>` : ""}</div>${n.body ? `<div class="release-notes hidden" id="notes-${i}">${esc(n.body)}</div>` : ""}`).join("") || '<span class="muted">No releases in the feed.</span>';
    $$("#detail-history [data-notes]").forEach((b) => b.addEventListener("click", () => {
      const box = $("#notes-" + b.dataset.notes);
      if (box) box.classList.toggle("hidden");
    }));
    const res = await call("release_assets", { repo: e.full_name, tag: e.latest_tag });
    $("#detail-assets").innerHTML = res.assets.map((a) =>
      `<div class="asset-row"><span>${esc(a.name)}</span><span class="muted">${(a.size / 1048576).toFixed(1)} MB</span>${a.name === res.pick ? '<span class="badge installed">auto-pick</span>' : ""}${a.digest ? '<span class="muted">sha256 ✓</span>' : ""}</div>`).join("") + `<div class="muted">Listing cost: ${res.quota} API call.${res.reason ? " Auto-pick: " + esc(res.reason) + "." : ""}</div>`;
    // Auto-fill the rules box from the assets' filename scheme when the
    // volume has no rules of its own yet — a suggestion, not a save:
    // nothing is stored until "Save rules" is clicked.
    state.detailSuggested = res.suggested || [];
    if (!e.asset_patterns.length && state.detailSuggested.length) {
      $("#detail-rules").value = state.detailSuggested.join("\n");
      $("#detail-rules-note").textContent =
        "Suggested from these file names — Save rules to keep them. The first matching line is the one that downloads.";
    }
  }
}

// Every [data-repo] element opens its volume — by click or by keyboard
// (Enter/Space), so the catalogue is fully usable without a mouse.
function bindEntryOpen(host) {
  host.querySelectorAll("[data-repo]").forEach((el) => {
    el.addEventListener("click", (ev) => {
      if (ev.target.closest(".selbox")) return; // selecting, not opening
      openDetail(el.dataset.repo);
    });
    el.tabIndex = 0;
    el.setAttribute("role", "link");
    el.setAttribute("aria-label", `Open ${el.dataset.repo}`);
    el.addEventListener("keydown", (ev) => {
      if (ev.target.closest(".selbox")) return;
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        openDetail(el.dataset.repo);
      }
    });
  });
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
      <h3>${esc(s.name)} <span class="muted">(${s.members.length})</span> <button class="small" data-shelf-update="${esc(s.name)}">Update shelf</button></h3>
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
  bindEntryOpen(host);
  host.querySelectorAll("[data-shelf-update]").forEach((b) => b.addEventListener("click", async () => {
    const name = b.dataset.shelfUpdate;
    const shelf = data.find((s) => s.name === name);
    if (!shelf || !shelf.members.length) { toast(`Shelf “${name}” is empty.`); return; }
    b.disabled = true;
    toast(`Checking shelf “${name}” via Atom feeds…`);
    await call("check_repos", { repos: shelf.members });
    const fresh = await call("find_entries", { query: "" });
    const due = shelf.members
      .map((m) => fresh.find((x) => x.full_name.toLowerCase() === m.toLowerCase()))
      .filter((e) => e && e.status === "Update available");
    if (!due.length) {
      toast(`Shelf “${name}” is current.`);
      b.disabled = false;
      refreshShelves();
      return;
    }
    toast(`Updating ${due.length} volume(s) on “${name}”…`);
    for (const e of due) await call("download_repo", { repo: e.full_name, tag: null });
    toast(`Shelf “${name}” updated.`);
    refreshShelves();
  }));
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
      <span class="muted">${j.percent.toFixed(0)}% · ${(j.bytes_done / 1048576).toFixed(1)}/${(j.bytes_total / 1048576).toFixed(1)} MB${j.speed_bps > 0 ? ` · ${fmtBytes(j.speed_bps)}/s${j.eta_secs > 0 ? ` · ~${fmtEta(j.eta_secs)} left` : ""}` : ""}</span>
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
$("#queue-retry-all").addEventListener("click", async () => {
  const jobs = await call("queue_list");
  const failed = jobs.filter((j) => j.status === "failed");
  if (!failed.length) { toast("No failed jobs to retry."); return; }
  for (const j of failed) await call("queue_retry", { id: j.id });
  toast(`Retrying ${failed.length} failed job(s)…`);
  await call("queue_run");
  refreshQueue();
});

// ---------- Librarian ----------
async function refreshLibrarian() {
  const cfg = await call("get_config");
  $("#cfg-atom").value = Math.round(cfg.atom_poll_secs / 60);
  $("#cfg-meta").value = Math.round(cfg.metadata_poll_secs / 60);
  $("#cfg-conc").value = cfg.concurrency;
  $("#cfg-dir").value = cfg.download_dir;
  $("#cfg-onstart").checked = !!cfg.check_on_start;
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
      check_on_start: $("#cfg-onstart").checked,
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
$("#backup-export").addEventListener("click", async () => {
  const path = await call("export_catalog");
  toast(`Catalogue exported → ${path}`);
});
$("#backup-import").addEventListener("click", async (ev) => {
  // Replacing the whole catalogue arms first, like a removal.
  const btn = ev.currentTarget;
  if (btn.dataset.armed !== "1") {
    btn.dataset.armed = "1";
    btn.textContent = "Replace catalogue — click again to confirm";
    setTimeout(() => {
      if (btn.isConnected) {
        btn.dataset.armed = "";
        btn.textContent = "Import catalogue";
      }
    }, 3500);
    return;
  }
  btn.dataset.armed = "";
  btn.textContent = "Import catalogue";
  const n = await call("import_catalog");
  toast(`Imported a catalogue backup: ${n} volume(s).`);
  refreshFacets();
  refreshCatalogue();
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
// Ctrl+K or Ctrl+F jumps to the search box from anywhere; "/" does the
// same when not already typing; Esc closes the Add modal; Tab stays
// inside the open modal (a focus trap, so keyboard users can't tab
// into the page behind it).
document.addEventListener("keydown", (ev) => {
  const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName || "");
  const modalOpen = !$("#add-modal").classList.contains("hidden");
  if ((ev.ctrlKey || ev.metaKey) && ["k", "f"].includes(ev.key.toLowerCase())) {
    ev.preventDefault();
    $("#global-search").focus();
    $("#global-search").select();
  } else if (ev.key === "/" && !typing) {
    ev.preventDefault();
    $("#global-search").focus();
  } else if (ev.key === "Escape" && modalOpen) {
    closeAddModal();
  } else if (ev.key === "Tab" && modalOpen) {
    const focusables = $$("#add-modal button, #add-modal input, #add-modal textarea, #add-modal select, #add-modal [tabindex]")
      .filter((el) => !el.disabled && el.offsetParent !== null);
    if (!focusables.length) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (ev.shiftKey && document.activeElement === first) {
      ev.preventDefault();
      last.focus();
    } else if (!ev.shiftKey && document.activeElement === last) {
      ev.preventDefault();
      first.focus();
    }
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

// ---------- Import from a GitHub user ----------
function iuInput(dryRun) {
  return {
    username: $("#iu-name").value.trim(),
    org: $("#iu-acct").value === "org",
    ownership: $("#iu-ownership").value,
    forks: $("#iu-forks").value,
    archived: $("#iu-archived").value,
    visibility: $("#iu-visibility").value,
    dry_run: dryRun,
  };
}
$("#iu-acct").addEventListener("change", (ev) => {
  const org = ev.target.value === "org";
  // An org listing spans many owners, so the Whose filter doesn't apply.
  $("#iu-ownership").disabled = org;
  $("#iu-name-label").firstChild.textContent = org ? "GitHub organization " : "GitHub username ";
  $("#iu-name").placeholder = org ? "e.g. rust-lang" : "e.g. SeanShaffer88";
  $("#iu-go").disabled = true;
  $("#iu-go").textContent = "Import";
});
$("#iu-preview").addEventListener("click", async () => {
  const name = $("#iu-name").value.trim();
  if (!name) {
    $("#iu-report").textContent = "Type a GitHub username or organization first.";
    return;
  }
  $("#iu-report").textContent = "Listing projects…";
  $("#iu-go").disabled = true;
  try {
    const r = await call("import_user", { input: iuInput(true) });
    const matched = r.added.length + r.duplicates.length;
    $("#iu-report").innerHTML =
      `${esc(name)}: <strong>${r.listed} listed</strong>, ${matched} match the filters, ` +
      `<strong>${r.added.length} would be added</strong>, ${r.duplicates.length} already catalogued. ` +
      `Listing cost: ${r.quota} API call(s).`;
    $("#iu-go").disabled = r.added.length === 0;
    $("#iu-go").textContent = r.added.length ? `Import ${r.added.length} project(s)` : "Import";
  } catch {
    $("#iu-report").textContent = "";
  }
});
$("#iu-go").addEventListener("click", async () => {
  try {
    const r = await call("import_user", { input: iuInput(false) });
    $("#iu-report").innerHTML =
      `Imported <strong>${r.added.length} project(s)</strong>; ${r.duplicates.length} were already catalogued.`;
    $("#iu-go").disabled = true;
    $("#iu-go").textContent = "Import";
    refreshFacets();
    refreshCatalogue();
    toast(`Imported ${r.added.length} project(s).`);
  } catch { /* the error was toasted by call() */ }
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
  buildColumnPicker();
  refreshSaved();
  await refreshFacets();
  await refreshCatalogue();
  const ov = await call("overview");
  if (ov.updates > 0) toast(`${ov.updates} update(s) waiting on My Shelf.`);
})();
