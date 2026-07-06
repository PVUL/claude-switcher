/**
 * claude-switcher pi extension.
 *
 * Three features:
 *  1. Footer status — the active account + 5-hour usage on the footer's path row.
 *  2. /claude-switcher [account] — change the active Claude account without leaving pi
 *     (flips the symlink, re-points this process, reloads so history carries).
 *  3. Account pinning — pin this session to one account so every bridge
 *     child-exec stays on it (via CLAUDE_SWITCHER_PIN), even if the symlink is
 *     flipped elsewhere; otherwise the bridge's underlying Claude Code sessions
 *     scatter across profile dirs and resume breaks. The account is *recorded
 *     into the session* the first time it's pinned and restored by name on
 *     every later turn/resume — so a resumed conversation keeps its own account
 *     instead of following whatever the symlink points at now.
 *
 * Shows the active Claude account on the RIGHT side of the footer's path row
 * (row 1), above the model — so the status section stays 2 rows tall:
 *
 *   ~/repos/foo (main)                 paul-nhost  ██░░░░░░ 13% 3h 30m
 *   ↑12k ↓3k $0.412 (sub) 8.1%/200k    claude-opus-4-8 • high
 *
 * The account segment degrades gracefully as width tightens: it drops the
 * reset countdown, then the bar, keeping at least the account name.
 *
 * Data comes from `claude-switcher usage --json` (the active entry's 5-hour
 * window). It is the only network-touching path and is entirely best-effort:
 * if the binary is missing or usage is unavailable, the footer simply falls
 * back to showing the path across the full width.
 *
 * setFooter replaces pi's built-in footer, so row 2 (token stats + model) is
 * reproduced here to match the stock look.
 */

import type { AssistantMessage } from "@mariozechner/pi-ai";
import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext, Theme } from "@mariozechner/pi-coding-agent";
import { truncateToWidth, visibleWidth } from "@mariozechner/pi-tui";
import { execFile } from "node:child_process";
import { existsSync, readlinkSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

// --- claude-switcher binary resolution ------------------------------------

// pi's PATH may not include ~/.local/bin (install.sh's default), so probe the
// common install locations before falling back to a bare PATH lookup.
function resolveSwitcherBinary(): string {
	const candidates = [
		join(homedir(), ".local", "bin", "claude-switcher"),
		"/usr/local/bin/claude-switcher",
		"/opt/homebrew/bin/claude-switcher",
	];
	for (const c of candidates) {
		if (existsSync(c)) return c;
	}
	return "claude-switcher"; // rely on PATH
}

const SWITCHER_BIN = resolveSwitcherBinary();

// --- Usage snapshot --------------------------------------------------------

interface AccountSnapshot {
	name: string;
	/** 5-hour window utilization percent (0-100), or undefined if unavailable. */
	utilization?: number;
	/** ISO timestamp the 5-hour window resets, or undefined if unavailable. */
	resetsAt?: string;
}

let snapshot: AccountSnapshot | null = null;
let fetchInFlight = false;
let requestRender: (() => void) | null = null;
// The account this session is pinned to. Drives both the footer/usage display
// and the CLAUDE_SWITCHER_PIN env, so the shown account matches the one the
// bridge actually runs on. Null until the first pin resolves.
let pinnedAccountName: string | null = null;

interface RawUsageEntry {
	active?: boolean;
	name?: string;
	fiveHour?: { utilization?: number; resetsAt?: string } | null;
}

function refreshUsage(): void {
	if (fetchInFlight) return;
	fetchInFlight = true;
	execFile(SWITCHER_BIN, ["usage", "--json"], { timeout: 8000 }, (err, stdout) => {
		fetchInFlight = false;
		if (err) return; // best-effort: leave the last snapshot in place
		try {
			const entries = JSON.parse(stdout) as RawUsageEntry[];
			const list = Array.isArray(entries) ? entries : [];
			// Show the account this session is pinned to, not whatever the global
			// symlink points at — otherwise a resumed conversation would display a
			// different account than it runs on. Fall back to the active account when
			// there's no pin yet (or it isn't present on this machine).
			const entry =
				(pinnedAccountName && list.find((e) => e.name === pinnedAccountName)) || list.find((e) => e.active);
			if (entry?.name) {
				snapshot = {
					name: entry.name,
					utilization:
						typeof entry.fiveHour?.utilization === "number" ? entry.fiveHour.utilization : undefined,
					resetsAt: entry.fiveHour?.resetsAt,
				};
				requestRender?.();
			}
		} catch {
			// Malformed JSON — ignore, keep prior snapshot.
		}
	});
}

// --- Account list + switching ---------------------------------------------

interface Account {
	name: string;
	active: boolean;
	email?: string;
	authenticated?: boolean;
	/** Config directory, possibly `~`-prefixed (e.g. `~/.claude-work`). */
	path?: string;
}

let accountsCache: { at: number; accounts: Account[] } | null = null;
const ACCOUNTS_TTL_MS = 5000;

async function listAccounts(): Promise<Account[]> {
	if (accountsCache && Date.now() - accountsCache.at < ACCOUNTS_TTL_MS) return accountsCache.accounts;
	try {
		const { stdout } = await execFileAsync(SWITCHER_BIN, ["list", "--json"], { timeout: 8000 });
		const parsed = JSON.parse(stdout) as Account[];
		const accounts = Array.isArray(parsed) ? parsed : [];
		accountsCache = { at: Date.now(), accounts };
		return accounts;
	} catch {
		return accountsCache?.accounts ?? [];
	}
}

/**
 * The resolved directory the ~/.claude-switcher symlink currently points at.
 * Mirrors claude-switcher-exec: read the link target, falling back to the link
 * path itself. Used to re-point the *running* pi process at the new account so
 * the bridge reads/writes its session JSONL under the right profile dir.
 */
function resolveActiveConfigDir(): string {
	const link = join(process.env.CLAUDE_SWITCHER_HOME || homedir(), ".claude-switcher");
	try {
		return readlinkSync(link);
	} catch {
		return link;
	}
}

// Custom session entry that records which account a conversation belongs to.
// Persisted (not sent to the LLM) so it survives reloads and cross-machine
// resumes; we store the account *name* (portable) rather than an absolute dir.
const PIN_ENTRY_TYPE = "claude-switcher/pinned-account";
interface PinData {
	account: string;
}

/** The account name this session recorded for itself (latest wins), if any. */
function recordedAccount(ctx: ExtensionContext): string | undefined {
	const entries = ctx.sessionManager.getEntries();
	for (let i = entries.length - 1; i >= 0; i--) {
		const e = entries[i] as { type?: string; customType?: string; data?: PinData };
		if (e.type === "custom" && e.customType === PIN_ENTRY_TYPE && e.data?.account) return e.data.account;
	}
	return undefined;
}

function expandHome(p: string): string {
	const home = process.env.CLAUDE_SWITCHER_HOME || homedir();
	if (p === "~") return home;
	if (p.startsWith("~/")) return join(home, p.slice(2));
	return p;
}

/** Resolve an account name to its config dir on THIS machine, or undefined. */
async function dirForAccount(name: string): Promise<string | undefined> {
	const acc = (await listAccounts()).find((a) => a.name === name);
	return acc?.path ? expandHome(acc.path) : undefined;
}

/** Point this pi process — and every exec it spawns — at `dir`. */
function applyPin(dir: string): void {
	process.env.CLAUDE_SWITCHER_PIN = dir;
	process.env.CLAUDE_CONFIG_DIR = dir;
}

/**
 * Pin the session to ONE account for the whole conversation.
 *
 * A conversation records its account the first time it is pinned; every later
 * turn — and every resume, even on another machine or after the global symlink
 * was flipped elsewhere — restores THAT account by name rather than following
 * the live symlink. Following the symlink on resume was the bug: a resumed
 * conversation would drift onto whatever account happened to be active,
 * scattering the bridge's underlying Claude Code sessions across profile dirs
 * and breaking resume.
 *
 * Only a brand-new conversation (nothing recorded) captures the currently
 * active account and records it. An explicit /claude-switcher switch re-records
 * via `recordPin`.
 */
async function restoreOrCapturePin(pi: ExtensionAPI, ctx: ExtensionContext): Promise<void> {
	try {
		const recorded = recordedAccount(ctx);
		if (recorded) {
			const dir = await dirForAccount(recorded);
			if (dir) {
				pinnedAccountName = recorded;
				applyPin(dir);
				return;
			}
			// Recorded account isn't present on this machine — fall through so the
			// session still runs, but DON'T overwrite the record: a later resume on
			// the right machine must still restore the real owner.
		}
		const active = (await listAccounts()).find((a) => a.active);
		if (active?.path) {
			pinnedAccountName = active.name;
			applyPin(expandHome(active.path));
			if (!recorded) pi.appendEntry<PinData>(PIN_ENTRY_TYPE, { account: active.name });
			return;
		}
		// No account info at all — follow the live symlink as a last resort.
		applyPin(resolveActiveConfigDir());
	} catch {
		applyPin(resolveActiveConfigDir());
	}
}

/** Record + apply a specific account (an explicit /claude-switcher switch). */
async function recordPin(pi: ExtensionAPI, name: string): Promise<void> {
	pinnedAccountName = name;
	applyPin((await dirForAccount(name)) ?? resolveActiveConfigDir());
	pi.appendEntry<PinData>(PIN_ENTRY_TYPE, { account: name });
}

// Coalesces the session_start + before_agent_start calls that fire close
// together so we don't resolve (and record) the pin twice. `force` re-resolves
// even when a (possibly stale) pin is already set, which session_start needs
// because the session identity may have just changed; before_agent_start only
// fills an unset pin.
let pinning: Promise<void> | null = null;
function schedulePin(pi: ExtensionAPI, ctx: ExtensionContext, force: boolean): Promise<void> {
	if (!force && process.env.CLAUDE_SWITCHER_PIN) return Promise.resolve();
	if (!pinning) {
		pinning = restoreOrCapturePin(pi, ctx).finally(() => {
			pinning = null;
		});
	}
	return pinning;
}

// --- Formatting helpers ----------------------------------------------------

const BAR_WIDTH = 8;

/** "3h 30m" style countdown from an ISO reset timestamp. */
function formatReset(resetsAt: string): string | undefined {
	const ms = Date.parse(resetsAt) - Date.now();
	if (Number.isNaN(ms)) return undefined;
	const secs = Math.floor(ms / 1000);
	if (secs <= 0) return "resetting";
	const d = Math.floor(secs / 86_400);
	const h = Math.floor((secs % 86_400) / 3600);
	const m = Math.floor((secs % 3600) / 60);
	if (d > 0) return `${d}d ${h}h`;
	if (h > 0) return `${h}h ${m}m`;
	return `${m}m`;
}

function utilizationColor(pct: number): "success" | "warning" | "error" {
	if (pct >= 90) return "error";
	if (pct >= 70) return "warning";
	return "success";
}

/**
 * Build the account segment at progressively shorter tiers, richest first.
 * `theme` is applied so callers can measure width with visibleWidth (ANSI is
 * ignored by that helper, matching pi's own footer math).
 */
function buildAccountTiers(theme: Theme, snap: AccountSnapshot): string[] {
	// Name in the same muted color as the rest of the footer; only the bar and
	// percentage carry the threshold color.
	const nameOnly = theme.fg("dim", snap.name);

	if (typeof snap.utilization !== "number") {
		// Signed in but usage not available (expired token / offline / no data).
		return [`${nameOnly} ${theme.fg("dim", "usage n/a")}`, nameOnly];
	}

	const pct = Math.round(snap.utilization);
	const color = utilizationColor(snap.utilization);
	const filled = Math.min(BAR_WIDTH, Math.max(0, Math.round((snap.utilization / 100) * BAR_WIDTH)));
	const bar = theme.fg(color, "█".repeat(filled)) + theme.fg("dim", "░".repeat(BAR_WIDTH - filled));
	const pctStr = theme.fg(color, `${pct}%`);
	const reset = snap.resetsAt ? formatReset(snap.resetsAt) : undefined;

	const noBar = `${nameOnly} ${pctStr}`;
	const noReset = `${nameOnly}  ${bar} ${pctStr}`;
	if (!reset) return [noReset, noBar, nameOnly];

	// Just the countdown (e.g. "3h 30m") — no "resets in" wording, to stay tight.
	const withReset = `${noReset} ${theme.fg("dim", reset)}`;
	return [withReset, noReset, noBar, nameOnly];
}

function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10_000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1_000_000) return `${Math.round(count / 1000)}k`;
	if (count < 10_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
	return `${Math.round(count / 1_000_000)}M`;
}

function sanitizeStatusText(text: string): string {
	return text.replace(/[\r\n\t]/g, " ").replace(/ +/g, " ").trim();
}

// --- Reload ----------------------------------------------------------------

const RELOAD_WIDGET_KEY = "claude-switcher-reload";

/**
 * Reload immediately after a switch, showing a brief "reloading…" widget so the
 * screen doesn't look frozen while the session rebuilds.
 *
 * There is deliberately no artificial pre-reload countdown: the reload itself
 * already takes a visible beat, and stacking a fixed delay on top of it just
 * doubles the wait. The widget is a static hint that the reload owns; it is
 * torn down with the old session runtime when reload swaps in a fresh one.
 *
 * In non-interactive contexts (no widgets) it degrades to a plain reload.
 */
async function switchThenReload(ctx: ExtensionCommandContext, name: string): Promise<void> {
	if (ctx.hasUI) {
		ctx.ui.setWidget(RELOAD_WIDGET_KEY, [`Switched to ${name} — reloading…`]);
	}
	await ctx.reload();
}

// --- Footer factory --------------------------------------------------------

function installFooter(pi: ExtensionAPI, ctx: ExtensionContext): void {
	if (!ctx.hasUI) return;

	ctx.ui.setFooter((tui, theme, footerData) => {
		requestRender = () => tui.requestRender();
		const unsub = footerData.onBranchChange(() => tui.requestRender());
		refreshUsage();

		return {
			dispose() {
				unsub();
				requestRender = null;
			},
			invalidate() {},
			render(width: number): string[] {
				// ---- Row 1: path (left) + account (right) ----
				let pwd = ctx.sessionManager.getCwd();
				const home = process.env.HOME || process.env.USERPROFILE;
				if (home && pwd.startsWith(home)) pwd = `~${pwd.slice(home.length)}`;
				const branch = footerData.getGitBranch();
				if (branch) pwd = `${pwd} (${branch})`;
				const sessionName = ctx.sessionManager.getSessionName();
				if (sessionName) pwd = `${pwd} • ${sessionName}`;

				let pwdLine: string;
				if (snapshot) {
					const tiers = buildAccountTiers(theme, snapshot);
					const GAP = 2;
					const pwdW = visibleWidth(pwd);
					// Pick the richest account tier that fits beside at least a
					// minimally-truncated path (8 cols), so the account never
					// swallows the whole line.
					let account = tiers[tiers.length - 1];
					for (const tier of tiers) {
						if (visibleWidth(tier) + GAP + Math.min(pwdW, 8) <= width) {
							account = tier;
							break;
						}
					}
					const accountW = visibleWidth(account);
					if (accountW >= width) {
						// Terminal too narrow for both — show the account alone.
						pwdLine = truncateToWidth(account, width);
					} else {
						const pwdAvail = width - accountW - GAP;
						// Skip the "..." ellipsis when there isn't room for it (it would
						// otherwise overflow the line and corrupt the TUI row).
						const ellipsis = pwdAvail >= 4 ? theme.fg("dim", "...") : "";
						const pwdShown = truncateToWidth(theme.fg("dim", pwd), pwdAvail, ellipsis);
						const pad = Math.max(GAP, width - visibleWidth(pwdShown) - accountW);
						pwdLine = truncateToWidth(pwdShown + " ".repeat(pad) + account, width);
					}
				} else {
					pwdLine = truncateToWidth(theme.fg("dim", pwd), width, theme.fg("dim", "..."));
				}

				// ---- Row 2: token stats (left) + model (right) ----
				let totalInput = 0,
					totalOutput = 0,
					totalCacheRead = 0,
					totalCacheWrite = 0,
					totalCost = 0;
				for (const entry of ctx.sessionManager.getEntries()) {
					if (entry.type === "message" && entry.message.role === "assistant") {
						const m = entry.message as AssistantMessage;
						totalInput += m.usage.input;
						totalOutput += m.usage.output;
						totalCacheRead += m.usage.cacheRead;
						totalCacheWrite += m.usage.cacheWrite;
						totalCost += m.usage.cost.total;
					}
				}

				const contextUsage = ctx.getContextUsage();
				const contextWindow = contextUsage?.contextWindow ?? ctx.model?.contextWindow ?? 0;
				const contextPercentValue = contextUsage?.percent ?? 0;
				const contextPercent = contextUsage?.percent != null ? contextPercentValue.toFixed(1) : "?";

				const statsParts: string[] = [];
				if (totalInput) statsParts.push(`↑${formatTokens(totalInput)}`);
				if (totalOutput) statsParts.push(`↓${formatTokens(totalOutput)}`);
				if (totalCacheRead) statsParts.push(`R${formatTokens(totalCacheRead)}`);
				if (totalCacheWrite) statsParts.push(`W${formatTokens(totalCacheWrite)}`);
				const usingSub = ctx.model ? ctx.modelRegistry.isUsingOAuth(ctx.model) : false;
				if (totalCost || usingSub) statsParts.push(`$${totalCost.toFixed(3)}${usingSub ? " (sub)" : ""}`);
				const ctxDisplay =
					contextPercent === "?"
						? `?/${formatTokens(contextWindow)}`
						: `${contextPercent}%/${formatTokens(contextWindow)}`;
				statsParts.push(
					contextPercentValue > 90
						? theme.fg("error", ctxDisplay)
						: contextPercentValue > 70
							? theme.fg("warning", ctxDisplay)
							: ctxDisplay,
				);

				let statsLeft = statsParts.join(" ");
				let statsLeftWidth = visibleWidth(statsLeft);
				if (statsLeftWidth > width) {
					statsLeft = truncateToWidth(statsLeft, width, "...");
					statsLeftWidth = visibleWidth(statsLeft);
				}

				const modelName = ctx.model?.id || "no-model";
				let right = modelName;
				if (ctx.model?.reasoning) {
					const level = pi.getThinkingLevel?.() || "off";
					right = level === "off" ? `${modelName} • thinking off` : `${modelName} • ${level}`;
				}
				const rightWidth = visibleWidth(right);
				const minPad = 2;
				let statsLine: string;
				if (statsLeftWidth + minPad + rightWidth <= width) {
					statsLine = statsLeft + " ".repeat(width - statsLeftWidth - rightWidth) + right;
				} else {
					const availRight = width - statsLeftWidth - minPad;
					if (availRight > 0) {
						const tr = truncateToWidth(right, availRight, "");
						statsLine = statsLeft + " ".repeat(Math.max(0, width - statsLeftWidth - visibleWidth(tr))) + tr;
					} else {
						statsLine = statsLeft;
					}
				}
				// Dim the whole stats line; statsLeft may embed a colored context %
				// (which resets), so dim the two halves independently like pi does.
				const dimStatsLine = theme.fg("dim", statsLeft) + theme.fg("dim", statsLine.slice(statsLeft.length));

				const lines = [pwdLine, dimStatsLine];

				// ---- Optional row 3+: other extensions' statuses ----
				const statuses = footerData.getExtensionStatuses();
				if (statuses.size > 0) {
					const line = Array.from(statuses.entries())
						.sort(([a], [b]) => a.localeCompare(b))
						.map(([, t]) => sanitizeStatusText(t))
						.join(" ");
					lines.push(truncateToWidth(line, width, theme.fg("dim", "...")));
				}
				return lines;
			},
		};
	});
}

// --- Extension entry point -------------------------------------------------

export default function (pi: ExtensionAPI) {
	let pollTimer: ReturnType<typeof setInterval> | undefined;
	let footerInstalled = false;

	// Idempotent: installs the footer + poll timer once per (re)load. Called from
	// several lifecycle events so the footer survives a /reload (which re-imports
	// this extension but may not re-fire session_start).
	const ensureFooter = (ctx: ExtensionContext) => {
		if (footerInstalled || !ctx.hasUI) return;
		footerInstalled = true;
		installFooter(pi, ctx);
		refreshUsage();
		if (!pollTimer) {
			// Re-poll periodically; this also re-renders so the countdown ticks.
			pollTimer = setInterval(refreshUsage, 60_000);
			pollTimer.unref?.();
		}
	};

	// Re-resolve the pin on every session_start (startup/new/resume/reload/fork):
	// restore the account this session recorded for itself, or capture+record the
	// active one if it's brand new. `force` because the session identity may have
	// just changed, making any inherited pin stale.
	pi.on("session_start", async (_event, ctx) => {
		ensureFooter(ctx);
		await schedulePin(pi, ctx, true);
	});
	// before_agent_start fires per turn; only fill an unset pin here as a fallback
	// for reload paths that don't re-fire session_start — never re-resolve when a
	// pin already exists, or a mid-conversation symlink flip would scatter the
	// bridge sessions.
	pi.on("before_agent_start", async (_event, ctx) => {
		ensureFooter(ctx);
		await schedulePin(pi, ctx, false);
	});

	pi.on("session_shutdown", () => {
		footerInstalled = false;
		if (pollTimer) {
			clearInterval(pollTimer);
			pollTimer = undefined;
		}
	});

	// /claude-switcher [account] — change the active Claude account without leaving pi.
	// Flips the claude-switcher symlink, re-points this process's
	// CLAUDE_CONFIG_DIR at the new profile, then reloads so the bridge rebuilds
	// the conversation under the new account (history is preserved pi-side).
	pi.registerCommand("claude-switcher", {
		description: "Switch the active Claude account (claude-switcher)",
		getArgumentCompletions: async (prefix) => {
			const accounts = await listAccounts();
			const items = accounts
				.filter((a) => a.name.startsWith(prefix))
				.map((a) => ({
					value: a.name,
					label: a.active ? `${a.name} (active)` : a.name,
					description: a.email,
				}));
			return items.length > 0 ? items : null;
		},
		handler: async (args, ctx) => {
			const accounts = await listAccounts();
			if (accounts.length === 0) {
				ctx.ui.notify("No claude-switcher accounts found (is claude-switcher installed?)", "error");
				return;
			}

			let name = args.trim();
			if (!name) {
				const options = accounts.map((a) => (a.active ? `${a.name}  (active)` : a.name));
				const picked = await ctx.ui.select("Switch Claude account", options);
				if (!picked) return; // cancelled
				name = picked.replace(/\s+\(active\)\s*$/, "").trim();
			}

			const target = accounts.find((a) => a.name === name);
			if (!target) {
				ctx.ui.notify(`Unknown account: ${name}`, "error");
				return;
			}
			if (target.active) {
				ctx.ui.notify(`Already on ${name}`, "info");
				return;
			}

			try {
				await execFileAsync(SWITCHER_BIN, ["switch", name], { timeout: 8000 });
			} catch (err) {
				ctx.ui.notify(`Switch failed: ${err instanceof Error ? err.message : String(err)}`, "error");
				return;
			}

			// Record the new account on THIS session and re-pin the running process so
			// the bridge's session JSONL lands in the right dir on the next turn. The
			// reload below re-fires session_start, which restores this record.
			accountsCache = null;
			await recordPin(pi, name);
			snapshot = null; // force the footer to refetch usage for the new account

			try {
				await switchThenReload(ctx, name);
			} catch (err) {
				ctx.ui.notify(
					`Account switched, but reload failed (${err instanceof Error ? err.message : String(err)}). ` +
						`Your next message may need a /reload to pick up history.`,
					"warning",
				);
			}
		},
	});
}
