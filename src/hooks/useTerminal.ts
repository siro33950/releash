import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { type ITheme, Terminal } from "@xterm/xterm";
import { type RefObject, useCallback, useEffect, useMemo, useRef } from "react";
import {
	reportMountedXtermMounted,
	reportMountedXtermUnmounted,
} from "@/lib/telemetry";
import type { Theme } from "@/types/settings";

interface TerminalSurfaceSnapshot {
	replay: string;
	sequence: number;
	cols: number;
	rows: number;
}

interface GetOrSpawnTerminalResult {
	session_key: string;
	is_exited: boolean;
	exit_code: number | null;
}

type TerminalSurfaceStreamItem =
	| {
			type: "snapshot";
			surface: {
				session_key: string;
				terminal_surface: TerminalSurfaceSnapshot;
				is_exited: boolean;
				exit_code: number | null;
			};
	  }
	| {
			type: "output";
			session_key: string;
			data: string;
			sequence: number;
	  }
	| {
			type: "resize";
			session_key: string;
			cols: number;
			rows: number;
			sequence: number;
	  }
	| {
			type: "exit";
			session_key: string;
			exit_code: number | null;
			sequence: number;
	  };

interface TauriCommandError {
	code?: unknown;
	message?: unknown;
}

export type TerminalSurfaceOwner =
	| { kind: "workspace"; workspacePath: string }
	| { kind: "session"; workspacePath: string; sessionId: string };

const TERMINAL_CAP_REACHED_CODE = "CAP_REACHED";

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
	owner?: TerminalSurfaceOwner,
	label?: string,
	onTerminalReady?: (sessionKey: string) => void,
	onTerminalError?: (message: string) => void,
	shouldKillPendingTerminal?: () => boolean,
) {
	const ownerKind = owner?.kind ?? "workspace";
	const ownerWorkspacePath = owner?.workspacePath ?? cwd ?? "";
	const ownerSessionId = owner?.kind === "session" ? owner.sessionId : null;
	const terminalOwner = useMemo<TerminalSurfaceOwner>(() => {
		if (ownerKind === "session") {
			return {
				kind: "session",
				workspacePath: ownerWorkspacePath,
				sessionId: ownerSessionId ?? "",
			};
		}
		return { kind: "workspace", workspacePath: ownerWorkspacePath };
	}, [ownerKind, ownerWorkspacePath, ownerSessionId]);
	const terminalRef = useRef<Terminal | null>(null);
	const fitAddonRef = useRef<FitAddon | null>(null);
	const sessionKeyRef = useRef<string | null>(null);
	const isRunningRef = useRef(false);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const killOnUnmountRef = useRef(false);
	const themeRef = useRef(theme);
	themeRef.current = theme;
	const onTerminalReadyRef = useRef(onTerminalReady);
	onTerminalReadyRef.current = onTerminalReady;
	const onTerminalErrorRef = useRef(onTerminalError);
	onTerminalErrorRef.current = onTerminalError;
	const shouldKillPendingTerminalRef = useRef(shouldKillPendingTerminal);
	shouldKillPendingTerminalRef.current = shouldKillPendingTerminal;

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

		let attachmentChannel: Channel<TerminalSurfaceStreamItem> | null = null;
		let attachmentId: string | null = null;
		let resolveUnmount!: () => void;
		const unmounted = new Promise<void>((resolve) => {
			resolveUnmount = resolve;
		});
		const writeToTerminal = (data: string) =>
			new Promise<void>((resolve) => {
				terminal.write(data, resolve);
			});

		const initTerminal = async () => {
			if (!isMounted) {
				return;
			}

			// 1. Get or spawn the PTY owned by the selected product surface.
			const { rows, cols } = terminal;
			const worktreePath = cwd ?? null;
			const result = await invoke<GetOrSpawnTerminalResult>(
				"get_or_spawn_pty",
				{
					rows,
					cols,
					cwd: worktreePath,
					owner: terminalOwner,
					label: label ?? null,
					startupCommand: terminalStartupCommand?.trim() || null,
				},
			);

			if (!isMounted) {
				const shouldKillDetachedPty =
					killOnUnmountRef.current ||
					(shouldKillPendingTerminalRef.current?.() ?? false);
				if (shouldKillDetachedPty && !result.is_exited) {
					invoke("kill_pty", { owner: terminalOwner }).catch(() => {});
				} else if (!shouldKillDetachedPty) {
					onTerminalReadyRef.current?.(result.session_key);
				}
				return;
			}

			// 2. Attach to one backend-owned snapshot + sequenced stream.
			let resolveInitialSnapshot!: () => void;
			const initialSnapshot = new Promise<void>((resolve) => {
				resolveInitialSnapshot = resolve;
			});
			let hasSnapshot = false;
			let streamSessionKey = result.session_key;
			let streamProcessing = Promise.resolve();
			attachmentId = crypto.randomUUID();
			attachmentChannel = new Channel<TerminalSurfaceStreamItem>();
			attachmentChannel.onmessage = (item) => {
				streamProcessing = streamProcessing.then(async () => {
					if (!isMounted) return;
					if (item.type === "snapshot") {
						streamSessionKey = item.surface.session_key;
						sessionKeyRef.current = streamSessionKey;
						terminal.resize(
							item.surface.terminal_surface.cols,
							item.surface.terminal_surface.rows,
						);
						if (item.surface.terminal_surface.replay) {
							await writeToTerminal(item.surface.terminal_surface.replay);
						}
						isRunningRef.current = !item.surface.is_exited;
						if (item.surface.is_exited) {
							await writeToTerminal(
								`\r\n\x1b[90m[Process exited with code ${item.surface.exit_code ?? "unknown"}]\x1b[0m\r\n`,
							);
						}
						if (!hasSnapshot) {
							hasSnapshot = true;
							resolveInitialSnapshot();
						}
						return;
					}
					if (item.type === "output") {
						await writeToTerminal(item.data);
						return;
					}
					if (item.type === "resize") {
						terminal.resize(item.cols, item.rows);
						return;
					}
					if (item.type === "exit") {
						await writeToTerminal(
							`\r\n\x1b[90m[Process exited with code ${item.exit_code ?? "unknown"}]\x1b[0m\r\n`,
						);
						isRunningRef.current = false;
					}
				});
			};
			await invoke("attach_pty", {
				owner: terminalOwner,
				attachmentId,
				onEvent: attachmentChannel,
			});
			if (!isMounted) {
				await invoke("detach_pty", { attachmentId });
				return;
			}
			const initialized = await Promise.race([
				initialSnapshot.then(() => true),
				unmounted.then(() => false),
			]);
			if (!initialized) return;
			if (!isMounted || !isRunningRef.current) return;
			onTerminalReadyRef.current?.(streamSessionKey);

			// 初回fit()が不正確だった場合のセーフティネット:
			// PTYスポーン後に最新のサイズで再同期する
			requestAnimationFrame(() => {
				if (!isMounted || !fitAddonRef.current || !terminalRef.current) return;
				fitAddonRef.current.fit();
				const { rows, cols } = terminalRef.current;
				if (rows > 0 && cols > 0) {
					invoke("resize_pty", {
						owner: terminalOwner,
						rows,
						cols,
					}).catch((error) => {
						console.error("Failed to resize PTY:", error);
					});
				}
			});
		};

		initTerminal().catch((error) => {
			console.error("Failed to initialize PTY:", error);
			if (!isMounted) return;
			const message = formatTerminalInitError(error);
			terminal.write(`\r\n\x1b[31m${message}\x1b[0m\r\n`);
			onTerminalErrorRef.current?.(message);
		});

		terminal.onData((data) => {
			if (isRunningRef.current) {
				invoke("write_pty", { owner: terminalOwner, data }).catch((error) => {
					console.error("Failed to write to PTY:", error);
				});
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

			if (isRunningRef.current) {
				const { rows, cols } = terminalRef.current;
				if (rows > 0 && cols > 0) {
					invoke("resize_pty", {
						owner: terminalOwner,
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
			resolveUnmount();
			if (resizeTimer !== null) {
				clearTimeout(resizeTimer);
			}
			resizeObserver.disconnect();
			if (attachmentId) {
				invoke("detach_pty", { attachmentId }).catch(() => {});
			}
			attachmentChannel = null;
			if (killOnUnmountRef.current && isRunningRef.current) {
				invoke("kill_pty", { owner: terminalOwner }).catch(() => {});
			}
			isRunningRef.current = false;
			sessionKeyRef.current = null;
			terminal.dispose();
			reportMountedXtermUnmounted();
		};
	}, [containerRef, cwd, label, terminalOwner, terminalStartupCommand]);

	useEffect(() => {
		const terminal = terminalRef.current;
		const container = containerRef.current;
		if (!terminal || !container) return;
		terminal.options.theme = getTerminalTheme(theme, container);
	}, [theme, containerRef]);

	const writeToTerminal = useCallback(
		(data: string) => {
			if (isRunningRef.current) {
				invoke("write_pty", { owner: terminalOwner, data }).catch((error) => {
					console.error("Failed to write to PTY:", error);
				});
			}
		},
		[terminalOwner],
	);

	const requestKill = useCallback(() => {
		killOnUnmountRef.current = true;
	}, []);

	return {
		terminalRef,
		terminalOwner,
		sessionKeyRef,
		isRunningRef,
		writeToTerminal,
		requestKill,
	};
}
