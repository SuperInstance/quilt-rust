# Examples

Working Quilt sheets you can run, study, and modify.

| Example | What it shows |
|---|---|
| [boat-autopilot](./boat-autopilot/) | Sensors + PID + voice intent + model router. The killer demo. |
| [agent-dashboard](./agent-dashboard/) | Tasks, status, shared human + agent workspace. The "agent mission control" pattern. |
| [model-router](./model-router/) | Caller-aware model selection. Row/column/identity-based routing. |
| [sensor-anomaly](./sensor-anomaly/) | Self-tuning anomaly detector. Local loop + model escalation. |

## Running an example

```bash
# Show the sheet structure
$ quilt inspect examples/boat-autopilot/sheet.yaml

# Run it (loads cells, prints values)
$ quilt run examples/boat-autopilot/sheet.yaml

# Run the live demo (simulated sensors, live updates)
$ npx tsx examples/boat-autopilot/demo.ts

# Expose as MCP server
$ quilt serve examples/agent-dashboard/sheet.yaml --mcp
```

## Each example

### boat-autopilot
The flagship demo. A reactive control system with:
- **Sensors** (compass reading from NMEA)
- **Pure computations** (heading error formula)
- **Programs** (PID-style rudder controller)
- **Listeners** (off-course alert)
- **Routers** (model router based on caller row)
- **IO cells** (rudder actuator)

Edit `desired.heading` and watch all three boats update.

### agent-dashboard
A shared workspace for humans and agents:
- Each task is a row (task.1.*, task.2.*, task.3.*)
- Aggregates compute pending/running/done counts
- A dispatcher routes each task to a different model
- The agent reads/writes cells via MCP
- The human watches in tmux

### model-router
The pure "caller-aware routing" pattern:
- Three model cells (fast, precise, premium)
- Three prompt cells
- A router that picks models by `caller.row`
- A router that picks prompts by `caller.identity.tags`

Change one cell — every caller reroutes.

### sensor-anomaly
The "self-tuning" pattern:
- A simulated temperature sensor
- A rolling mean cell (classical, fast, cheap)
- A surprise z-score (formula)
- A model cell that only fires on surprise
- A listener that escalates anomalies

Over time, the rolling mean adapts and the model fires less often.

## Writing your own

Start with a template:

```bash
cp templates/predictive-maintenance.yaml my-monitor.quilt.yaml
$EDITOR my-monitor.quilt.yaml
quilt run my-monitor.quilt.yaml
```

Or copy an example and modify:

```bash
cp examples/boat-autopilot/sheet.yaml my-project.quilt.yaml
$EDITOR my-project.quilt.yaml
quilt run my-project.quilt.yaml
```

The sheet is just a YAML file. The runtime is the same. The MCP server exposes it to agents. The CLI runs it. The TUI watches it.
