/**
 * Boat Autopilot Demo
 *
 * Loads the boat-autopilot sheet, simulates a heading sensor, and
 * watches the rudder command update in real time.
 *
 * Run:  npx tsx examples/boat-autopilot/demo.ts
 */

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { QuiltEngine, parseSheet } from '@quilt/core';

async function main() {
  const source = await readFile(resolve('examples/boat-autopilot/sheet.yaml'), 'utf8');
  const sheet = parseSheet(source);
  const engine = new QuiltEngine(sheet.id);
  engine.loadSheet(sheet);

  console.log('⛵ Boat Autopilot — Live Demo');
  console.log('─'.repeat(60));
  console.log('Watch the rudder command respond to compass changes.\n');
  console.log('  boat     compass → desired  error      rudder');
  console.log('  ────     ─────── → ───────  ─────      ──────');

  // Simulate 3 boats. Each is a "row" — same sheet, different caller context.
  const boats = [
    { id: 'boat-1', heading: 180 },
    { id: 'boat-2', heading: 175 },
    { id: 'boat-3', heading: 195 },
  ];

  const desired = 180;

  const interval = setInterval(async () => {
    // Drift each boat's heading slightly
    for (const boat of boats) {
      boat.heading = (boat.heading + (Math.random() - 0.5) * 4 + 360) % 360;
    }

    // Update lines
    const lines: string[] = [];
    for (const boat of boats) {
      // Push the new heading
      await engine.push('compass.heading', boat.heading);
      // Get the error (formula) and rudder command (program) in this boat's context
      const err = await engine.get('heading.error', { row: boat.id, timestamp: Date.now() });
      const rudder = await engine.call('rudder.command', undefined, { row: boat.id, timestamp: Date.now() });

      const h = String(Math.round(boat.heading)).padStart(3);
      const d = String(desired).padStart(3);
      const e = err.data !== undefined ? String(((err.data as number) || 0).toFixed(1)).padStart(6) : '   n/a';
      const r = rudder.data && typeof rudder.data === 'object' && 'angle' in rudder.data
        ? ((rudder.data as { angle: number }).angle || 0).toFixed(1).padStart(6)
        : '   n/a';

      lines.push(`  ${boat.id.padEnd(8)} ${h}° → ${d}°    ${e}°    ${r}°`);
    }

    process.stdout.write('\r' + lines.join('\n'));
  }, 200);

  // Stop after 8 seconds
  setTimeout(() => {
    clearInterval(interval);
    console.log('\n\n✓ Demo complete.');
    console.log('  Edit desired.heading or rudder.gain and watch all boats update.\n');
    process.exit(0);
  }, 8000);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
