// quilt.js — minimal client for the quilt-web REST + SSE API.
//
// Usage:
//   <script type="module" src="./quilt.js"></script>
//
// The page is auto-populated from GET /api/sheet. Cells with
// kind = "value" or kind = "sensor" are editable; clicking
// a value cell opens an input that PATCHes on Enter.
//
// Live updates stream in over /api/events (SSE).

const $title = document.getElementById("title");
const $cells = document.getElementById("cells");
const $live = document.getElementById("live-dot");

const state = {
  sheet: null,
  values: new Map(), // id -> { data, status, error }
};

async function fetchSheet() {
  const res = await fetch("/api/sheet");
  if (!res.ok) throw new Error("failed to load sheet");
  state.sheet = await res.json();
  document.title = `Quilt — ${state.sheet.id}`;
  $title.firstChild.nodeValue = `Quilt — ${state.sheet.id} `;
}

async function fetchCell(id) {
  const res = await fetch(`/api/cell/${encodeURIComponent(id)}`);
  if (!res.ok) {
    state.values.set(id, { data: null, status: "error", error: res.statusText });
    return;
  }
  const v = await res.json();
  state.values.set(id, v);
}

function renderCell(cell) {
  const v = state.values.get(cell.id) || { data: null, status: "idle", error: null };
  const row = document.createElement("div");
  row.className = `cell kind-${cell.kind}`;
  row.dataset.cellId = cell.id;
  row.innerHTML = `
    <div class="id"></div>
    <div class="kind">${escapeHtml(cell.kind)}</div>
    <div class="value"></div>
    <div class="status ${v.status}">${escapeHtml(v.status)}</div>
  `;
  row.querySelector(".id").textContent = cell.id;
  const $val = row.querySelector(".value");
  if (v.error) {
    $val.textContent = `<error: ${v.error}>`;
    $val.style.color = "var(--err)";
  } else if (cell.kind === "value" || cell.kind === "sensor") {
    // Editable
    $val.innerHTML = "";
    const input = document.createElement("input");
    input.className = "editable";
    input.value = JSON.stringify(v.data);
    input.addEventListener("keydown", async (e) => {
      if (e.key === "Enter") {
        let parsed;
        try {
          parsed = JSON.parse(input.value);
        } catch {
          parsed = input.value; // fall back to string
        }
        await setCell(cell.id, parsed);
      } else if (e.key === "Escape") {
        input.value = JSON.stringify(v.data);
        input.blur();
      }
    });
    $val.appendChild(input);
  } else {
    $val.textContent = JSON.stringify(v.data);
  }
  return row;
}

function renderAll() {
  $cells.innerHTML = "";
  for (const cell of state.sheet.cells) {
    $cells.appendChild(renderCell(cell));
  }
}

async function refreshAll() {
  await Promise.all(state.sheet.cells.map((c) => fetchCell(c.id)));
  renderAll();
}

async function setCell(id, value) {
  const res = await fetch(`/api/cell/${encodeURIComponent(id)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(value),
  });
  if (!res.ok) {
    console.error("set failed", id, res.statusText);
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  })[c]);
}

// SSE subscription
function connectEvents() {
  const es = new EventSource("/api/events");
  es.onopen = () => {
    $live.classList.remove("disconnected");
  };
  es.onerror = () => {
    $live.classList.add("disconnected");
  };
  es.onmessage = async (msg) => {
    if (!msg.data) return;
    try {
      const ev = JSON.parse(msg.data);
      if (ev.lagged !== undefined) return; // skip
      // Refresh the changed cell + its dependents
      await fetchCell(ev.cell_id);
      const cell = state.sheet?.cells.find((c) => c.id === ev.cell_id);
      if (cell) {
        // Replace just this row to avoid full re-render
        const old = document.querySelector(`[data-cell-id="${CSS.escape(ev.cell_id)}"]`);
        const fresh = renderCell(cell);
        if (old) old.replaceWith(fresh);
      }
    } catch (e) {
      console.error("sse parse error", e);
    }
  };
  return es;
}

(async function main() {
  try {
    await fetchSheet();
    await refreshAll();
    connectEvents();
  } catch (e) {
    $cells.textContent = `error: ${e.message}`;
  }
})();
