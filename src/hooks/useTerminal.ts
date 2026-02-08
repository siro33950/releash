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

const terminalDarkTheme: ITheme = {
	background: "#1a1a1a",
	foreground: "#e0e0e0",
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
	background: "#f8f8f8",
	foreground: "#1a1a1a",
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

function getTerminalTheme(theme?: Theme): ITheme {
	return theme === "light" ? terminalLightTheme : terminalDarkTheme;
}

export function useTerminal(
	containerRef: RefObject<HTMLDivElement | null>,
	cwd?: string | null,
	theme?: Theme,
) {
	const terminalRef = useRef<Terminal | null>(null);
	const fitAddonRef = useRef<FitAddon | null>(null);
	const ptyIdRef = useRef<number | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const themeRef = useRef(theme);
	themeRef.current = theme;

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let isMounted = true;

		const terminal = new Terminal({
			cursorBlink: true,
			fontFamily: 'Menlo, Monaco, "Courier New", monospace',
			fontSize: 14,
			theme: getTerminalTheme(themeRef.current),
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

			const { rows, cols } = terminal;
			const ptyId = await invoke<number>("spawn_pty", {
				rows,
				cols,
				cwd: cwd ?? null,
			});

			if (!isMounted) {
				invoke("kill_pty", { ptyId }).catch(() => {});
				return;
			}

			ptyIdRef.current = ptyId;
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
			if (ptyIdRef.current !== null) {
				invoke("kill_pty", { ptyId: ptyIdRef.current }).catch(() => {});
			}
			terminal.dispose();
		};
	}, [containerRef, cwd]);

	useEffect(() => {
		const terminal = terminalRef.current;
		if (!terminal) return;
		terminal.options.theme = getTerminalTheme(theme);
	}, [theme]);

	const writeToTerminal = useCallback((data: string) => {
		if (ptyIdRef.current !== null) {
			invoke("write_pty", { ptyId: ptyIdRef.current, data }).catch((error) => {
				console.error("Failed to write to PTY:", error);
			});
		}
	}, []);

	return { terminalRef, ptyIdRef, writeToTerminal };
}
