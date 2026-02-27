import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { type ITheme, Terminal } from "@xterm/xterm";
import { type RefObject, useCallback, useEffect, useRef } from "react";
import { trackEvent } from "@/lib/telemetry";
import type { Theme } from "@/types/settings";

interface PtyOutput {
	pty_id: number;
	data: string;
}

interface PtyExit {
	pty_id: number;
	exit_code: number | null;
}

interface GetOrSpawnPtyResult {
	pty_id: number;
	session_key: string;
	buffered_output: string;
	is_new: boolean;
	is_exited: boolean;
	exit_code: number | null;
}

const sessionKeyCache = new Map<string, string>();

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
	agentType?: string,
	label?: string,
	onPtyReady?: (ptyId: number, sessionKey: string) => void,
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
	const agentTypeRef = useRef(agentType);
	agentTypeRef.current = agentType;
	const onPtyReadyRef = useRef(onPtyReady);
	onPtyReadyRef.current = onPtyReady;

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

		const initPty = async () => {
			// 1. Register listeners first (ptyIdRef is still null so they won't fire yet)
			unlistenOutput = await listen<PtyOutput>("pty-output", (event) => {
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

			if (!isMounted) return;

			// 2. Get or spawn PTY for this worktree
			const { rows, cols } = terminal;
			const worktreePath = cwd ?? null;
			// sessionKey がない standalone の場合、cwd キャッシュから復元
			const effectiveSessionKey =
				sessionKey ?? (cwd ? sessionKeyCache.get(cwd) : undefined) ?? null;
			const result = await invoke<GetOrSpawnPtyResult>("get_or_spawn_pty", {
				rows,
				cols,
				cwd: worktreePath,
				sessionKey: effectiveSessionKey,
				worktreePath: worktreePath ?? "",
				label: label ?? null,
			});

			// standalone 用: cwd → UUID キャッシュ更新
			if (!sessionKey && cwd) {
				sessionKeyCache.set(cwd, result.session_key);
			}

			if (!isMounted) {
				if (killOnUnmountRef.current && !result.is_exited) {
					invoke("kill_pty", { ptyId: result.pty_id }).catch(() => {});
				}
				return;
			}

			// 3. Replay buffered output
			if (result.buffered_output) {
				terminal.write(result.buffered_output);
			}

			// 4. Handle already-exited session
			if (result.is_exited) {
				terminal.write(
					`\r\n\x1b[90m[Process exited with code ${result.exit_code ?? "unknown"}]\x1b[0m\r\n`,
				);
				return;
			}

			// 5. Set ptyId (from here, real-time output starts flowing)
			ptyIdRef.current = result.pty_id;
			onPtyReadyRef.current?.(result.pty_id, result.session_key);

			// 初回fit()が不正確だった場合のセーフティネット:
			// PTYスポーン後に最新のサイズで再同期する
			requestAnimationFrame(() => {
				if (!isMounted || !fitAddonRef.current || !terminalRef.current) return;
				fitAddonRef.current.fit();
				const { rows, cols } = terminalRef.current;
				if (rows > 0 && cols > 0) {
					invoke("resize_pty", {
						ptyId: result.pty_id,
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
					trackEvent("agent_started", {
						agent_type: agentTypeRef.current ?? "unknown",
					});
					invoke("write_pty", {
						ptyId: result.pty_id,
						data: `${cmd}\n`,
					}).catch((error) => {
						console.error("Failed to send startup command:", error);
					});
				}
			}
		};

		initPty().catch((error) => {
			console.error("Failed to initialize PTY:", error);
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
			unlistenOutput?.();
			unlistenExit?.();
			if (killOnUnmountRef.current && ptyIdRef.current !== null) {
				invoke("kill_pty", { ptyId: ptyIdRef.current }).catch(() => {});
			}
			ptyIdRef.current = null;
			terminal.dispose();
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
