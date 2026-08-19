# Quilt 5-year roadmap (2026-2031)

> The path from "single-file browser runtime" to "runtime for the era of personal data on personal devices, in a personal mesh."

This document is a sketch of where Quilt goes over the next five years. It is intentionally detailed: not "what's possible" but **what we are building, in what order, and why**.

---

## The thesis

The dominant computing metaphor of the next era is **the cell**, not the file, not the program, not the service. A cell is a value plus a history plus a type plus an access policy plus dependencies plus formulas. Cells compose. Cells sync. Cells outlive devices. Cells belong to people, not platforms.

The 1990s were about files. The 2010s were about services. The 2030s will be about cells. Quilt is the runtime for that era.

**Quilt already exists.** It's a reactive engine in TypeScript and Rust that runs in the browser and on the desktop. It has 98 tests passing. It has 54 working examples. It powers single-file portable apps.

**What's missing** is the larger vision: the cell model, scaled out. Cells on microcontrollers. Cells in a peer-to-peer mesh. Cells in encrypted vaults. Cells in vision pipelines. Cells in zero-knowledge proofs. Cells as visual graphs. Cells as agents.

The next five years are about building that scale-out. This document names the repos, sketches the first versions, and orders the work.

---

## End-state in 2031

A vivid description of what working looks like in 2031, so the path back is clear.

It's 2031. You wake up in a bed that has 47 Quilt cells: temperature, weight-on-mattress, last-moved, room-temperature, ambient-light, co2, presence. Your watch has 312 cells: heart-rate, spo2, steps, sleep-stage, ambient-noise, location, etc. Your phone has 1,847 cells. Your home has 12,000+ cells: lights, locks, windows, doors, appliances, sensors, meters, vacuums, plants, pets.

The cells are in rooms. Your bedroom is a room. Your house is a room. Your family is a room. Your team at work is a room. The city's public-data room has cells for traffic, weather, air quality, transit, parking. The world has many rooms, and your devices are in many of them.

The cells sync. The watch and the bed and the phone and the house all know your sleep score because the sleep-score cell is in your "body" room, and every device contributes its slice. The vault encrypts everything. The owner (you) holds the keys. You can grant temporary access to your doctor (the health-data room, just the relevant cells) or your partner (the home-data room, every cell). Revocation is a button.

The agent runs as a sheet. The agent's memory is a set of value cells. The agent's tools are API cells. The agent's reasoning is a chain of program cells. The agent is a Quilt sheet, edited in the visual editor, time-travel-debugged like a database, versioned like git.

A cell is portable. A cell is not a Google cell or an Apple cell. It's yours. It runs on your hardware, syncs to your mesh, lives in your vault. You can take it with you when you leave a platform.

The system is alive. New cells appear when a new device joins the mesh. Old cells retire when devices go away. The graph grows, prunes, and reorganizes itself. There is no "app" in the traditional sense — there are sheets, and sheets compose.

The economic model is different. The cloud doesn't see your data. The cloud sees encrypted envelopes and serves them back to you. The cloud is dumb storage, fast routing, and a CDN. The intelligence is in your cells, on your devices.

This is the end-state. The next 5 years is the path.

---

## The repos (built and to-build)

We have **6 shipped** + **6 sketched** + **15+ to ideate**. The list is not exhaustive. There are more repos we haven't thought of yet.

### Shipped (have first releases)

- **[quilt](https://github.com/SuperInstance/quilt)** — the canonical TypeScript runtime
- **[quilt-rust](https://github.com/SuperInstance/quilt-rust)** — the desktop Rust runtime
- **[quilt-live](https://github.com/SuperInstance/quilt-live)** — single-file browser runtime, full Quilt in one HTML

### Sketched (first cut pushed)

- **[quilt-esp32](https://github.com/SuperInstance/quilt-esp32)** — `no_std` Rust port to ESP32-class microcontrollers. 2 unit tests pass.
- **[quilt-mesh](https://github.com/SuperInstance/quilt-mesh)** — broker-less CRDT mesh. 3 unit tests pass.
- **[quilt-agent](https://github.com/SuperInstance/quilt-agent)** — LLM agents as sheets. Working example.
- **[quilt-time](https://github.com/SuperInstance/quilt-time)** — time-travel for cells. **17 tests pass.** Every cell has history; you can fork, rewind, replay, merge.
- **[quilt-vault](https://github.com/SuperInstance/quilt-vault)** — encrypted cells. **10 tests pass.** Real ECDH P-256 + AES-GCM. Per-cell access control.
- **[quilt-vision](https://github.com/SuperInstance/quilt-vision)** — images as cells. 8 vision cell kinds.
- **[quilt-zk](https://github.com/SuperInstance/quilt-zk)** — zero-knowledge over cells. **7 tests pass.** 6 pre-built circuits.
- **[quilt-flow](https://github.com/SuperInstance/quilt-flow)** — visual editor. **8 tests pass.** Drag-and-drop cell wiring.

### To ideate (not yet sketched)

- **quilt-packs** — distribution format. Cell kinds as packages.
- **quilt-rooms** — semantic grouping. Access/sync boundary.
- **quilt-os** — minimal OS where every "file" is a sheet.
- **quilt-music** — audio as cells. Notes, chords, beats, tracks, songs.
- **quilt-print** — paper as a cell. PDF, QR codes, ZPL labels.
- **quilt-fabric** — distributed cells across many devices.
- **quilt-civic** — public cells. The cell model, for the public sector.
- **quilt-build** — compile cells to WASM, native, embedded.
- **quilt-fs** — files as cells. Replaces the file system metaphor.
- **quilt-money** — financial cells. Personal finance as a sheet.
- **quilt-health** — health data cells. HIPAA-compliant by default.
- **quilt-climate** — environmental cells. Sensors for the planet.
- **quilt-garden** — garden cells. Soil, light, water, growth.
- **quilt-pets** — pet cells. GPS collar, feeder, health.
- **quilt-sport** — sports cells. Strava as cells.
- **quilt-book** — books as cells. Reading list, notes, quotes.
- **quilt-language** — language learning cells. Vocabulary, grammar, conversation.
- **quilt-meditation** — mindfulness cells. Sessions, moods, streaks.
- **quilt-dream** — sleep & dream cells.
- **quilt-mood** — emotional state cells.
- **quilt-kitchen** — kitchen cells. Recipes, inventory, grocery.
- **quilt-dna** — DNA as a cell. Genealogy, ancestry.

We don't need to build all of these. We need to build the ones with the highest leverage. The current top three by leverage: **quilt-time** (done), **quilt-vault** (done), **quilt-mesh** (sketched). Next: **quilt-fs**, **quilt-money**, **quilt-build**.

---

## The three paradigm shifts

The 5-year arc is not "more features." It's three structural shifts in what the cell is.

### Files → cells

A file is a static blob. A cell is a live, addressable, reactive value with a type, a history, and an access policy. Files live on a disk. Cells live in a mesh.

This shift is already happening. Notion's blocks are file-ish. Airtable's records are file-ish. Linear's issues are file-ish. But they're file-ish *on a server*. Quilt's cells are file-ish *in a mesh*. The mesh is the differentiator.

### Programs → graphs

A program is a sequence of statements. A graph is a set of nodes and edges. Programs run once. Graphs are always running.

This shift is what makes the cell model composable. You can take a 10-cell "mortgage" sub-graph and drop it into a 100-cell "personal finance" graph. The composition is automatic. The dependencies are explicit. The reactive engine handles the propagation.

### Services → rooms

A service is a public API. A room is a private shared space. Services are on a server. Rooms are on a mesh.

This shift is the privacy story. A room is a logical group with an access policy. A "service" is just a room that anyone can read. A "personal API" is a room with a few members. The cell model is the same in both cases; only the access policy differs.

---

## 12-month roadmap

A build order. Each item is a first release. We ship in this sequence.

**Q1 2026 — Foundations** ✅

- ✅ **`quilt-time`** (1 month) — time-travel for cells. **Shipped.** 17 tests pass.
- ✅ **`quilt-vault`** (1 month) — encrypted cells. **Shipped.** 10 tests pass.
- ✅ **`quilt-rooms`** (sketched) — semantic grouping. Simple but required for `quilt-mesh`.

**Q2 2026 — Distribution**

- ✅ **`quilt-mesh`** (1 month) — peer-to-peer cell sync. **Sketched.** 3 tests pass.
- ✅ **`quilt-esp32`** (1 month) — `no_std` port to microcontrollers. **Sketched.** 2 tests pass.
- **`quilt-packs`** (1 month) — distribution. Cell kinds as packages. Registry on GitHub Pages.

**Q3 2026 — Intelligence**

- ✅ **`quilt-agent`** (1 month) — LLM agents as sheets. **Sketched.** Working example.
- ✅ **`quilt-vision`** (1 month) — images as cells. **Sketched.** 8 vision cell kinds.
- ✅ **`quilt-zk`** (2 months) — zero-knowledge proofs over cells. **Sketched.** 7 tests pass.
- **`quilt-build`** (1 month) — compile cells to WASM, native, embedded.

**Q4 2026 — Polish**

- ✅ **`quilt-flow`** (1 month) — visual editor. **Sketched.** 8 tests pass.
- **`quilt-print`** (1 month) — paper as a cell. PDF, QR, ZPL.
- **`quilt-music`** (1 month) — audio as cells. The cell model, for music.
- **`quilt-fabric`** (1 month) — distributed cells across devices. Wraps `quilt-mesh`.
- **`quilt-civic`** (1 month) — public data cells. The cell model, for the public sector.
- **`quilt-fs`** (1 month) — files as cells. Replaces the file system metaphor.

By end of 2026: 18 first releases, 3 paradigm shifts demonstrated, a working mesh from microcontroller to cloud.

**Beyond 2026 — the long arc**

- **quilt-os** (2027) — the running system. Linux-based OS image where every "file" is a sheet.
- **quilt-fabric** v2 (2027) — a true distributed graph, not just sync.
- **A public cell mesh** (2028) — anyone can publish a public cell. Anyone can read it. No central authority.
- **A cell model in the browser** (2028) — every browser has a built-in Quilt runtime. Cells are part of the web platform.
- **A cell model in the OS** (2029) — every OS has a built-in Quilt runtime. Cells are part of the OS.
- **A cell model in silicon** (2030) — Quilt is in firmware. The cell graph runs on bare metal.

By 2031: cells are everywhere. Quilt is the runtime.

---

## How this document is used

- **Roadmap**: the build order.
- **Ideation**: the 6 shipped + 6 sketched + 15+ to-ideate list. Add more as we think of them.
- **Communication**: the end-state narrative. What we're building toward.
- **Hiring**: when we add people, this is what we show them.

The list is not exhaustive. There are 10+ repos we haven't thought of yet. The point of writing this down is to make it easier to think of them. The point of the 12-month roadmap is to ship.
