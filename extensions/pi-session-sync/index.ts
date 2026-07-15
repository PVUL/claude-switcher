/**
 * pi-session-sync — keep pi's session transcripts in sync across machines.
 *
 * pi stores the authoritative conversation transcripts under
 * ~/.pi/agent/sessions/. This extension drives a small git engine
 * (session-sync.sh) on pi's own lifecycle events so your Mac, laptop, and VPS
 * converge on one private git remote — no daemon, no timer required for
 * interactive use:
 *
 *   session_start (not a reload)  ->  pull   (bounded, so you start on fresh data)
 *   agent_end                     ->  push --detach  (debounced; never blocks a turn)
 *   session_shutdown              ->  push   (final flush before exit)
 *
 * All calls are best-effort: a missing engine, an unconfigured repo, or a
 * network hiccup is swallowed so it never disrupts a pi session. Run
 * `session-sync.sh init <git-remote>` once per machine to set it up, and
 * `install-timer` on any machine that also runs headless (a VPS) to cover
 * non-interactive use.
 */

import type { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

// Prefer the engine shipped beside this extension; fall back to one on PATH.
function resolveEngine(): string {
	const local = join(HERE, "session-sync.sh");
	if (existsSync(local)) return local;
	return "session-sync.sh";
}
const ENGINE = resolveEngine();

/** Run the engine best-effort; resolve regardless of exit status. */
function run(args: string[], timeoutMs: number): Promise<void> {
	return new Promise((resolve) => {
		try {
			execFile("/bin/sh", [ENGINE, ...args], { timeout: timeoutMs }, () => resolve());
		} catch {
			resolve();
		}
	});
}

export default function (pi: ExtensionAPI) {
	// Start on fresh data. Skip reloads: a /reload keeps the same working tree,
	// and account switches already reload — a redundant pull just adds latency.
	pi.on("session_start", async (event) => {
		if (event.reason === "reload") return;
		await run(["pull"], 8000);
	});

	// The agent finished a request and control returned to the user — the
	// natural "done" boundary. Detached + debounced in the engine, so it never
	// blocks the next turn.
	pi.on("agent_end", () => {
		void run(["push", "--detach"], 5000);
	});

	// Final flush on exit (awaited so it lands before the process goes away).
	pi.on("session_shutdown", async () => {
		await run(["push"], 8000);
	});

	// Manual: /session-sync forces an immediate pull + push.
	pi.registerCommand("session-sync", {
		description: "Sync pi session history now (pull + push)",
		handler: async (_args, ctx) => {
			await run(["sync"], 20000);
			ctx.ui.notify("pi session history synced", "info");
		},
	});
}
