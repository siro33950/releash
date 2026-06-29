import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { type ITheme, Terminal } from "@xterm/xterm";
import { type RefObject, useCallback, useEffect, useRef } from "react";
import {
	reportMountedXtermMounted,
	reportMountedXtermUnmounted,
} from "@/lib/telemetry";
import type { Theme } from "@/types/settings";

interface PtyOutput {
	pty_id: number;
	data: string;
	sequence: number;
}

interface PtyExit {
	pty_id: number;
	exit_code: number | null;
}

interface PtyEvicted {
	pty_id: number;
	session_key: string;
	reason: string;
}

interface GetOrSpawnPtyResult {
	pty_id: number;
	session_key: string;
	buffered_output: string;
	buffered_output_sequence: number;
	is_new: boolean;
	is_exited: boolean;
	exit_code: number | null;
}

interface GetPtyBufferedOutputResult {
	pty_id: number;
	session_key: string;
	buffered_output: string;
	buffered_output_sequence: number;
	is_exited: boolean;
	exit_code: number | null;
}

interface TauriCommandError {
	code?: unknown;
	message?: unknown;
}

const QUEUED_INITIAL_OUTPUT_MAX_ITEMS = 256;
const QUEUED_INITIAL_OUTPUT_MAX_BYTES = 64 * 1024;
const MAX_INITIAL_REFETCH = 5;

const TERMINAL_CAP_REACHED_CODE = "CAP_REACHED";
const INITIAL_OUTPUT_RESYNC_FAILED_MESSAGE =
	"\r\n\x1b[33m[Terminal output may be incomplete: unable to resynchronize buffered output]\x1b[0m\r\n";
const textEncoder = new TextEncoder();

const sessionKeyCache = new Map<string, string>();
let activeTerminalTokenCounter = 0;

function removeCachedSessionKey(sessionKey: string): void {
	for (const [cwd, cachedSessionKey] of sessionKeyCache.entries()) {
		if (cachedSessionKey === sessionKey) {
			sessionKeyCache.delete(cwd);
		}
	}
}

function createActiveTerminalToken(): string {
	activeTerminalTokenCounter += 1;
	return `terminal-${activeTerminalTokenCounter}`;
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function getCommandError(error: unknown): TauriCommandError | null {
	if (!isObject(error)) return null;
	return error;
}

function getErrorMessage(error: unknown): string {
	const commandError = getCommandError(error);
	if (typeof commandError?.message === "string") {
		return commandError.message;
	}
	if (error instanceof Error) {
		return error.message;
	}
	return String(error);
}

function getErrorCode(error: unknown): string | null {
	const commandError = getCommandError(error);
	if (typeof commandError?.code === "string") {
		return commandError.code;
	}
	return null;
}

function createBoundedPtyOutputQueue(
	maxItems = QUEUED_INITIAL_OUTPUT_MAX_ITEMS,
	maxBytes = QUEUED_INITIAL_OUTPUT_MAX_BYTES,
) {
	let entries: Array<{ output: PtyOutput; bytes: number }> = [];
	let totalBytes = 0;
	let dropped = false;

	return {
		enqueue(output: PtyOutput) {
			const bytes = textEncoder.encode(output.data).byteLength;
			if (bytes > maxBytes) {
				entries = [];
				totalBytes = 0;
				dropped = true;
				return;
			}

			entries.push({ output, bytes });
			totalBytes += bytes;

			while (entries.length > maxItems || totalBytes > maxBytes) {
				const droppedEntry = entries.shift();
				if (!droppedEntry) break;
				totalBytes -= droppedEntry.bytes;
				dropped = true;
			}
		},
		values(): PtyOutput[] {
			return entries.map((entry) => entry.output);
		},
		clear() {
			entries = [];
			totalBytes = 0;
			dropped = false;
		},
		size(): number {
			return entries.length;
		},
		bytes(): number {
			return totalBytes;
		},
		hasDropped(): boolean {
			return dropped;
		},
		resetDropped() {
			dropped = false;
		},
	};
}

function registerActiveTerminal(
	worktreePath: string,
	sessionKey: string,
	activeToken: string,
): void {
	invoke("register_active_terminal", {
		worktreePath,
		sessionKey,
		activeToken,
	}).catch((error) => {
		console.error("Failed to register active terminal:", error);
	});
}

function unregisterActiveTerminal(
	worktreePath: string | null,
	sessionKey: string | null,
	activeToken: string,
): void {
	if (worktreePath === null || sessionKey === null) return;
	invoke("unregister_active_terminal", {
		worktreePath,
		sessionKey,
		activeToken,
	}).catch((error) => {
		console.error("Failed to unregister active terminal:", error);
	});
}

function formatTerminalInitError(error: unknown): string {
	const message = getErrorMessage(error);
	if (getErrorCode(error) === TERMINAL_CAP_REACHED_CODE) {
		return `Terminal limit reached: ${message}`;
	}
	return `Failed to initialize terminal: ${message}`;
}

const terminalDarkTheme: ITheme = {
	foreground: "#e0e0e0",
	selectionBackground: "#264F78",
	selectionInactiveBackground: "#3A3D41",
	cursor: "#e0e0e0",
	cursorAccent: "#1a1a1a",
	black: "#1a1a1a",
	red: "#ff5f56",
	green: "#27c93f",
	yellow: "#ffbd2e",
	blue: "#2ea6ff",
	magenta: "#d75fff",
	cyan: "#5fd7ff",
	white: "#e0e0e0",
	brightBlack: "#7f7f7f",
	brightRed: "#ff6e67",
	brightGreen: "#5af78e",
	brightYellow: "#f9f1a5",
	brightBlue: "#57c7ff",
	brightMagenta: "#ff6ac1",
	brightCyan: "#9aedfe",
	brightWhite: "#ffffff",
};

const terminalLightTheme: ITheme = {
	foreground: "#1a1a1a",
	selectionBackground: "#ADD6FF",
	selectionInactiveBackground: "#E5EBF1",
	cursor: "#1a1a1a",
	cursorAccent: "#f8f8f8",
	black: "#1a1a1a",
	red: "#d73a49",
	green: "#22863a",
	yellow: "#e36209",
	blue: "#005cc5",
	magenta: "#6f42c1",
	cyan: "#1b7c83",
	white: "#e0e0e0",
	brightBlack: "#6a737d",
	brightRed: "#cb2431",
	brightGreen: "#28a745",
	brightYellow: "#f9c513",
	brightBlue: "#2188ff",
	brightMagenta: "#8a63d2",
	brightCyan: "#3192aa",
	brightWhite: "#fafbfc",
};

function resolveTerminalBg(container: HTMLElement): string {
	const bg = getComputedStyle(container).backgroundColor;
	if (!bg || bg === "rgba(0, 0, 0, 0)" || bg === "transparent") {
		return "#1a1a1a";
	}
	return bg;
}

function getTerminalTheme(
	theme: Theme | undefined,
	container: HTMLElement,
): ITheme {
	const base = theme === "light" ? terminalLightTheme : terminalDarkTheme;
	return { ...base, background: resolveTerminalBg(container) };
}

export function useTerminal(
	containerRef: RefObject<HTMLDivElement | null>,
	cwd?: string | null,
	theme?: Theme,
	terminalStartupCommand?: string,
	sessionKey?: string,
	label?: string,
	onPtyReady?: (ptyId: number, sessionKey: string) => void,
	onPtyError?: (message: string) => void,
	shouldKillPendingPty?: () => boolean,
) {
	const terminalRef = useRef<Terminal | null>(null);
	const fitAddonRef = useRef<FitAddon | null>(null);
	const ptyIdRef = useRef<number | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const killOnUnmountRef = useRef(false);
	const themeRef = useRef(theme);
	themeRef.current = theme;
	const startupCommandRef = useRef(terminalStartupCommand);
	startupCommandRef.current = terminalStartupCommand;
	const onPtyReadyRef = useRef(onPtyReady);
	onPtyReadyRef.current = onPtyReady;
	const onPtyErrorRef = useRef(onPtyError);
	onPtyErrorRef.current = onPtyError;
	const shouldKillPendingPtyRef = useRef(shouldKillPendingPty);
	shouldKillPendingPtyRef.current = shouldKillPendingPty;

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let isMounted = true;

		const terminal = new Terminal({
			cursorBlink: true,
			fontFamily: 'Menlo, Monaco, "Courier New", monospace',
			fontSize: 14,
			theme: getTerminalTheme(themeRef.current, container),
		});
		reportMountedXtermMounted();

		const fitAddon = new FitAddon();
		terminal.loadAddon(fitAddon);

		terminal.open(container);
		fitAddon.fit();

		// レンダラー初期化後に再度fitして正確なサイズを確定
		requestAnimationFrame(() => {
			if (isMounted) {
				fitAddon.fit();
			}
		});

		// ペイン操作に使うキーをxtermに処理させない
		terminal.attachCustomKeyEventHandler((event) => {
			const mod = event.metaKey || event.ctrlKey;
			// Cmd+D (垂直分割) / Cmd+Shift+D (水平分割)
			if (mod && event.key === "d") return false;
			if (mod && event.key === "D") return false;
			// Cmd+Option+矢印 (フォーカス移動)
			if (mod && event.altKey && event.key.startsWith("Arrow")) return false;
			return true;
		});

		terminalRef.current = terminal;
		fitAddonRef.current = fitAddon;

		let unlistenOutput: UnlistenFn | null = null;
		let unlistenExit: UnlistenFn | null = null;
		let unlistenEvicted: UnlistenFn | null = null;
		let registeredWorktreePath: string | null = null;
		let registeredSessionKey: string | null = null;
		const activeToken = createActiveTerminalToken();
		let isInitializingPty = true;
		let initializingPtyId: number | null = null;
		const queuedInitialOutput = createBoundedPtyOutputQueue();

		const cleanupPtyListeners = () => {
			unlistenOutput?.();
			unlistenOutput = null;
			unlistenExit?.();
			unlistenExit = null;
			unlistenEvicted?.();
			unlistenEvicted = null;
		};

		const initPty = async () => {
			// 1. Register listeners first and queue output until the PTY id is known.
			unlistenOutput = await listen<PtyOutput>("pty-output", (event) => {
				if (isInitializingPty) {
					if (
						initializingPtyId === null ||
						event.payload.pty_id === initializingPtyId
					) {
						queuedInitialOutput.enqueue(event.payload);
					}
					return;
				}
				if (event.payload.pty_id === ptyIdRef.current) {
					terminal.write(event.payload.data);
				}
			});

			unlistenExit = await listen<PtyExit>("pty-exit", (event) => {
				if (event.payload.pty_id === ptyIdRef.current) {
					terminal.write(
						`\r\n\x1b[90m[Process exited with code ${event.payload.exit_code ?? "unknown"}]\x1b[0m\r\n`,
					);
					ptyIdRef.current = null;
				}
			});

			unlistenEvicted = await listen<PtyEvicted>("pty-evicted", (event) => {
				removeCachedSessionKey(event.payload.session_key);
				if (event.payload.pty_id === ptyIdRef.current) {
					unregisterActiveTerminal(
						registeredWorktreePath,
						event.payload.session_key,
						activeToken,
					);
					registeredWorktreePath = null;
					registeredSessionKey = null;
					terminal.write("\r\n\x1b[90m[Terminal evicted]\x1b[0m\r\n");
					ptyIdRef.current = null;
				}
			});

			if (!isMounted) {
				cleanupPtyListeners();
				isInitializingPty = false;
				queuedInitialOutput.clear();
				return;
			}

			// 2. Get or spawn PTY for this worktree
			const { rows, cols } = terminal;
			const worktreePath = cwd ?? null;
			// sessionKey がない standalone の場合のみ、cwd キャッシュから復元
			// onPtyReady がある場合はペイン管理側がセッションキーを保持するため
			// キャッシュを使わず新規PTYをスポーンさせる
			const effectiveSessionKey =
				sessionKey ??
				(!onPtyReadyRef.current && cwd
					? sessionKeyCache.get(cwd)
					: undefined) ??
				null;
			const result = await invoke<GetOrSpawnPtyResult>("get_or_spawn_pty", {
				rows,
				cols,
				cwd: worktreePath,
				sessionKey: effectiveSessionKey,
				worktreePath: worktreePath ?? "",
				label: label ?? null,
			});

			// standalone 用: cwd → UUID キャッシュ更新（管理ペインでは不要）
			if (!sessionKey && !onPtyReadyRef.current && cwd) {
				sessionKeyCache.set(cwd, result.session_key);
			}

			if (!isMounted) {
				const shouldKillDetachedPty =
					killOnUnmountRef.current ||
					(shouldKillPendingPtyRef.current?.() ?? false);
				if (shouldKillDetachedPty && !result.is_exited) {
					invoke("kill_pty", { ptyId: result.pty_id }).catch(() => {});
				} else if (!shouldKillDetachedPty) {
					onPtyReadyRef.current?.(result.pty_id, result.session_key);
				}
				unregisterActiveTerminal(
					worktreePath ?? "",
					result.session_key,
					activeToken,
				);
				cleanupPtyListeners();
				isInitializingPty = false;
				queuedInitialOutput.clear();
				return;
			}

			let ptyId = result.pty_id;
			initializingPtyId = ptyId;
			let resolvedSessionKey = result.session_key;
			let bufferedOutput = result.buffered_output;
			let bufferedOutputSequence = result.buffered_output_sequence;
			let isExited = result.is_exited;
			let exitCode = result.exit_code;
			let attempts = 0;

			while (
				!isExited &&
				queuedInitialOutput.hasDropped() &&
				attempts < MAX_INITIAL_REFETCH
			) {
				queuedInitialOutput.resetDropped();
				const refreshed = await invoke<GetPtyBufferedOutputResult>(
					"get_pty_buffered_output",
					{
						sessionKey: resolvedSessionKey,
						worktreePath: worktreePath ?? "",
					},
				);
				ptyId = refreshed.pty_id;
				initializingPtyId = ptyId;
				resolvedSessionKey = refreshed.session_key;
				bufferedOutput = refreshed.buffered_output;
				bufferedOutputSequence = refreshed.buffered_output_sequence;
				isExited = refreshed.is_exited;
				exitCode = refreshed.exit_code;
				attempts += 1;
			}

			if (attempts >= MAX_INITIAL_REFETCH && queuedInitialOutput.hasDropped()) {
				terminal.write(INITIAL_OUTPUT_RESYNC_FAILED_MESSAGE);
			}

			// 3. Replay buffered output
			if (bufferedOutput) {
				terminal.write(bufferedOutput);
			}

			// 4. Handle already-exited session
			if (isExited) {
				isInitializingPty = false;
				queuedInitialOutput.clear();
				terminal.write(
					`\r\n\x1b[90m[Process exited with code ${exitCode ?? "unknown"}]\x1b[0m\r\n`,
				);
				return;
			}

			// 5. Set ptyId and flush output that arrived after the backend snapshot.
			ptyIdRef.current = ptyId;
			for (const output of queuedInitialOutput.values()) {
				if (
					output.pty_id === ptyId &&
					output.sequence > bufferedOutputSequence
				) {
					terminal.write(output.data);
				}
			}
			queuedInitialOutput.clear();
			isInitializingPty = false;
			registeredWorktreePath = worktreePath ?? "";
			registeredSessionKey = resolvedSessionKey;
			registerActiveTerminal(
				registeredWorktreePath,
				registeredSessionKey,
				activeToken,
			);
			onPtyReadyRef.current?.(ptyId, resolvedSessionKey);

			// 初回fit()が不正確だった場合のセーフティネット:
			// PTYスポーン後に最新のサイズで再同期する
			requestAnimationFrame(() => {
				if (!isMounted || !fitAddonRef.current || !terminalRef.current) return;
				fitAddonRef.current.fit();
				const { rows, cols } = terminalRef.current;
				if (rows > 0 && cols > 0) {
					invoke("resize_pty", {
						ptyId: ptyId,
						rows,
						cols,
					}).catch((error) => {
						console.error("Failed to resize PTY:", error);
					});
				}
			});

			// 6. Send startup command for newly created PTY
			if (result.is_new && startupCommandRef.current) {
				const cmd = startupCommandRef.current.trim();
				if (cmd) {
					invoke("write_pty", {
						ptyId: ptyId,
						data: `${cmd}\n`,
					}).catch((error) => {
						console.error("Failed to send startup command:", error);
					});
				}
			}
		};

		initPty().catch((error) => {
			console.error("Failed to initialize PTY:", error);
			isInitializingPty = false;
			queuedInitialOutput.clear();
			if (!isMounted) return;
			const message = formatTerminalInitError(error);
			terminal.write(`\r\n\x1b[31m${message}\x1b[0m\r\n`);
			onPtyErrorRef.current?.(message);
		});

		terminal.onData((data) => {
			if (ptyIdRef.current !== null) {
				invoke("write_pty", { ptyId: ptyIdRef.current, data }).catch(
					(error) => {
						console.error("Failed to write to PTY:", error);
					},
				);
			}
		});

		const RESIZE_DEBOUNCE_MS = 100;
		let resizeTimer: ReturnType<typeof setTimeout> | null = null;
		let wasHidden = false;

		const performResize = () => {
			const el = containerRef.current;
			if (!el || !fitAddonRef.current || !terminalRef.current) return;
			if (el.clientWidth === 0 || el.clientHeight === 0) return;

			fitAddonRef.current.fit();

			if (ptyIdRef.current !== null) {
				const { rows, cols } = terminalRef.current;
				if (rows > 0 && cols > 0) {
					invoke("resize_pty", {
						ptyId: ptyIdRef.current,
						rows,
						cols,
					}).catch((error) => {
						console.error("Failed to resize PTY:", error);
					});
				}
			}
		};

		const resizeObserver = new ResizeObserver(() => {
			const el = containerRef.current;
			if (!el || !fitAddonRef.current) return;

			const isHidden = el.clientWidth === 0 || el.clientHeight === 0;
			if (isHidden) {
				wasHidden = true;
				return;
			}

			if (wasHidden) {
				wasHidden = false;
				performResize();
				requestAnimationFrame(() => {
					terminalRef.current?.refresh(0, (terminalRef.current?.rows ?? 1) - 1);
				});
				return;
			}

			if (resizeTimer !== null) {
				clearTimeout(resizeTimer);
			}
			resizeTimer = setTimeout(() => {
				resizeTimer = null;
				performResize();
			}, RESIZE_DEBOUNCE_MS);
		});
		resizeObserver.observe(container);
		resizeObserverRef.current = resizeObserver;

		return () => {
			isMounted = false;
			if (resizeTimer !== null) {
				clearTimeout(resizeTimer);
			}
			resizeObserver.disconnect();
			cleanupPtyListeners();
			if (killOnUnmountRef.current && ptyIdRef.current !== null) {
				invoke("kill_pty", { ptyId: ptyIdRef.current }).catch(() => {});
			}
			unregisterActiveTerminal(
				registeredWorktreePath,
				registeredSessionKey,
				activeToken,
			);
			ptyIdRef.current = null;
			terminal.dispose();
			reportMountedXtermUnmounted();
		};
	}, [containerRef, cwd, sessionKey, label]);

	useEffect(() => {
		const terminal = terminalRef.current;
		const container = containerRef.current;
		if (!terminal || !container) return;
		terminal.options.theme = getTerminalTheme(theme, container);
	}, [theme, containerRef]);

	const writeToTerminal = useCallback((data: string) => {
		if (ptyIdRef.current !== null) {
			invoke("write_pty", { ptyId: ptyIdRef.current, data }).catch((error) => {
				console.error("Failed to write to PTY:", error);
			});
		}
	}, []);

	const requestKill = useCallback(() => {
		killOnUnmountRef.current = true;
	}, []);

	return { terminalRef, ptyIdRef, writeToTerminal, requestKill };
}
