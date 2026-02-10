import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { type ITheme, Terminal } from "@xterm/xterm";
import { type RefObject, useCallback, useEffect, useRef } from "react";
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
	buffered_output: string;
	is_new: boolean;
	is_exited: boolean;
	exit_code: number | null;
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
) {
	const terminalRef = useRef<Terminal | null>(null);
	const fitAddonRef = useRef<FitAddon | null>(null);
	const ptyIdRef = useRef<number | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const themeRef = useRef(theme);
	themeRef.current = theme;
	const startupCommandRef = useRef(terminalStartupCommand);
	startupCommandRef.current = terminalStartupCommand;

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
			const result = await invoke<GetOrSpawnPtyResult>("get_or_spawn_pty", {
				rows,
				cols,
				cwd: worktreePath,
				worktreePath: worktreePath ?? "",
			});

			if (!isMounted) return;

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

			// 6. Send startup command for newly created PTY
			if (result.is_new && startupCommandRef.current) {
				const cmd = startupCommandRef.current.trim();
				if (cmd) {
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

		const resizeObserver = new ResizeObserver(() => {
			if (fitAddonRef.current) {
				fitAddonRef.current.fit();
				if (ptyIdRef.current !== null && terminalRef.current) {
					const { rows, cols } = terminalRef.current;
					invoke("resize_pty", { ptyId: ptyIdRef.current, rows, cols }).catch(
						(error) => {
							console.error("Failed to resize PTY:", error);
						},
					);
				}
			}
		});
		resizeObserver.observe(container);
		resizeObserverRef.current = resizeObserver;

		return () => {
			isMounted = false;
			resizeObserver.disconnect();
			unlistenOutput?.();
			unlistenExit?.();
			terminal.dispose();
		};
	}, [containerRef, cwd]);

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

	return { terminalRef, ptyIdRef, writeToTerminal };
}
