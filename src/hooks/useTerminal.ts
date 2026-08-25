import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { type ITheme, Terminal } from "@xterm/xterm";
import { type RefObject, useCallback, useEffect, useMemo, useRef } from "react";
import { getErrorMessage } from "@/lib/errorMessage";
import {
	reportMountedXtermMounted,
	reportMountedXtermUnmounted,
} from "@/lib/telemetry";
import { TerminalOutputScheduler } from "@/lib/terminalOutputScheduler";
import {
	isTerminalPerformanceProbeActive,
	readTerminalLogicalBuffer,
	registerTerminalBufferReader,
	reportTerminalInputPerformancePoint,
	reportTerminalPerformancePhase,
	reportTerminalRendererMetrics,
	shouldReportTerminalSnapshotLaunchPhase,
	takeTerminalLaunchPerformanceOrigin,
} from "@/lib/terminalPerformanceProbe";
import { getTerminalPerformanceSwitches } from "@/lib/terminalPerformanceSwitches";
import { StartupInputBuffer } from "@/lib/terminalStartupInputBuffer";
import { getTerminalStreamEndpoint } from "@/lib/terminalStreamEndpoint";
import { openTerminalStreamSocket } from "@/lib/terminalStreamSocket";
import {
	applyTerminalStreamItem,
	type TerminalStreamApplyContext,
	type TerminalSurfaceOwner,
	type TerminalSurfaceStreamItem,
} from "@/lib/terminalSurfaceStream";
import type { Theme } from "@/types/settings";

export type { TerminalSurfaceOwner } from "@/lib/terminalSurfaceStream";

interface GetOrSpawnTerminalResult {
	session_key: string;
	is_exited: boolean;
	exit_code: number | null;
}

class TerminalBackendCommandError extends Error {}

async function invokeTerminalBackendCommand<T>(
	command: string,
	args: Record<string, unknown>,
): Promise<T> {
	try {
		return await invoke<T>(command, args);
	} catch (error) {
		throw new TerminalBackendCommandError(getErrorMessage(error));
	}
}

const terminalDarkTheme: ITheme = {
	foreground: "#e0e0e0",
	selectionBackground: "#264F78",
	selectionInactiveBackground: "#3A3D41",
	cursor: "#e0e0e0",
	cursorAccent: "#000000",
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
		return "#000000";
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

export type TerminalInitializationMode = "get-or-spawn" | "attach-existing";

export interface UseTerminalOptions {
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	owner?: TerminalSurfaceOwner;
	label?: string;
	onTerminalReady?: (sessionKey: string) => void;
	onTerminalError?: (message: string | null) => void;
	shouldKillPendingTerminal?: () => boolean;
	initialization?: TerminalInitializationMode;
	autoFocus?: boolean;
}

export function useTerminal(
	containerRef: RefObject<HTMLDivElement | null>,
	options: UseTerminalOptions = {},
) {
	const {
		cwd,
		theme,
		terminalStartupCommand,
		owner,
		label,
		onTerminalReady,
		onTerminalError,
		shouldKillPendingTerminal,
		initialization = "get-or-spawn",
		autoFocus = false,
	} = options;
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
	const isRunningRef = useRef(false);
	const killOnUnmountRef = useRef(false);
	const inputDispatchRef = useRef<(data: string) => void>(() => {});
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

		// IME変換中のEnterで xterm が送るEnter由来バイトを捨てるためのフラグ
		let suppressImeEnterData = false;

		// キー変換とペイン操作キーの入力抑止
		terminal.attachCustomKeyEventHandler((event) => {
			if (
				event.key === "Enter" &&
				(event.isComposing || event.keyCode === 229)
			) {
				// keyCode 229 では xterm がEnterをエンコードしないため抑止しない。
				// keyCode 13 (WebKit系) では composition確定後にEnterがエンコードされるので抑止する。
				// xtermの送出は同じtask内で同期的に終わるため、残ったフラグはmicrotaskで下ろす。
				if (event.type === "keydown" && event.keyCode === 13) {
					suppressImeEnterData = true;
					queueMicrotask(() => {
						suppressImeEnterData = false;
					});
				}
				return true;
			}
			if (event.key === "Enter") {
				const isShiftOnly =
					event.shiftKey && !event.metaKey && !event.ctrlKey && !event.altKey;
				const isMetaOnly =
					event.metaKey && !event.shiftKey && !event.ctrlKey && !event.altKey;
				if (isShiftOnly || isMetaOnly) {
					if (event.type === "keydown") {
						event.preventDefault();
						terminal.input("\x1b\r", true);
					}
					return false;
				}
			}
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

		let attachmentId: string | null = null;
		let inputSequence = 0;
		let pendingPerformanceInputSequences: number[] = [];
		const startupInput = new StartupInputBuffer((dropped) => {
			console.warn(
				`Discarding ${dropped.length} chars of terminal input typed before startup: buffer limit reached`,
			);
		});
		let deliverInput: (data: string) => void = () => {};
		let activeSocket: WebSocket | null = null;
		let wsTransportFailed = false;
		const socketsClosedByUs = new WeakSet<WebSocket>();
		const releaseCurrentAttachment = () => {
			if (activeSocket) {
				socketsClosedByUs.add(activeSocket);
				activeSocket.close();
				activeSocket = null;
			} else if (attachmentId) {
				const releasedAttachmentId = attachmentId;
				invoke("detach_terminal_surface", {
					attachmentId: releasedAttachmentId,
				}).catch((error) => {
					console.error(
						`Failed to detach terminal attachment ${releasedAttachmentId}:`,
						error,
					);
				});
			}
		};
		let resolveUnmount!: () => void;
		const unmounted = new Promise<void>((resolve) => {
			resolveUnmount = resolve;
		});
		const writeToTerminal = (data: string) =>
			new Promise<void>((resolve) => {
				terminal.write(data, resolve);
			});
		const syncPtySize = (rows: number, cols: number) => {
			if (rows <= 0 || cols <= 0) return;
			invoke("resize_terminal_surface", {
				owner: terminalOwner,
				rows,
				cols,
			}).catch((error) => {
				console.error("Failed to resize PTY:", error);
			});
		};
		let recoverAttachment: ((failedEpoch?: number) => void) | null = null;
		let attachmentEpoch = 0;
		let performanceRequestStartedAt: number | null = null;
		let firstXtermParsed = false;
		const performanceProbeActive = isTerminalPerformanceProbeActive();
		const unregisterBufferReader = registerTerminalBufferReader(() =>
			readTerminalLogicalBuffer(terminal),
		);
		const recordRendererLaunchPhase = (
			phase: "first_xterm_parsed" | "first_paint",
			durationMs: number,
		) => {
			reportTerminalPerformancePhase(phase, durationMs);
			void invoke("record_terminal_launch_renderer_phase", {
				phase,
				durationMs,
			});
		};
		const reportFirstXtermParsed = () => {
			if (
				!performanceProbeActive ||
				firstXtermParsed ||
				performanceRequestStartedAt === null
			)
				return;
			firstXtermParsed = true;
			recordRendererLaunchPhase(
				"first_xterm_parsed",
				performance.now() - performanceRequestStartedAt,
			);
			requestAnimationFrame(() => {
				if (performanceRequestStartedAt === null) return;
				recordRendererLaunchPhase(
					"first_paint",
					performance.now() - performanceRequestStartedAt,
				);
			});
		};
		const liveOutputScheduler = new TerminalOutputScheduler({
			write: (data, parsed) => terminal.write(data, parsed),
			onOverflow: () => recoverAttachment?.(),
			onMetrics: performanceProbeActive
				? reportTerminalRendererMetrics
				: undefined,
			onParsed: performanceProbeActive ? reportFirstXtermParsed : undefined,
		});
		const drainLiveOutput = () => liveOutputScheduler.drain();

		const initTerminal = async () => {
			if (!isMounted) {
				return;
			}

			// 1. Get or spawn the PTY owned by the selected product surface.
			const performanceSwitchesPromise = getTerminalPerformanceSwitches();
			const streamEndpointPromise = getTerminalStreamEndpoint();
			const { rows, cols } = terminal;
			const worktreePath = cwd ?? null;
			const requestStartedAt = performance.now();
			const launchOrigin =
				terminalOwner.kind === "session"
					? takeTerminalLaunchPerformanceOrigin(terminalOwner.sessionId)
					: null;
			performanceRequestStartedAt = launchOrigin ?? requestStartedAt;
			const result =
				initialization === "attach-existing"
					? await invokeTerminalBackendCommand<GetOrSpawnTerminalResult>(
							"get_terminal_surface",
							{
								owner: terminalOwner,
							},
						)
					: await invokeTerminalBackendCommand<GetOrSpawnTerminalResult>(
							"get_or_spawn_terminal_surface",
							{
								rows,
								cols,
								cwd: worktreePath,
								owner: terminalOwner,
								label: label ?? null,
								startupCommand: terminalStartupCommand?.trim() || null,
							},
						);
			reportTerminalPerformancePhase(
				"frontend_request_to_command_response",
				performance.now() - requestStartedAt,
			);

			if (!isMounted) {
				const shouldKillDetachedPty =
					killOnUnmountRef.current ||
					(shouldKillPendingTerminalRef.current?.() ?? false);
				if (shouldKillDetachedPty && !result.is_exited) {
					invoke("kill_terminal_surface", { owner: terminalOwner }).catch(
						(error) => {
							console.error(
								"Failed to kill detached pending terminal PTY:",
								error,
							);
						},
					);
				} else if (!shouldKillDetachedPty) {
					onTerminalReadyRef.current?.(result.session_key);
				}
				return;
			}

			// 2. Attach to one backend-owned snapshot + sequenced stream.
			const performanceSwitches = await performanceSwitchesPromise;
			const streamEndpoint = await streamEndpointPromise;
			const suppressOutputAcks = performanceSwitches.disableOutputFlowControl;
			liveOutputScheduler.setMaxWritesInFlight(
				performanceSwitches.disableRendererWriteSerialization ? 8 : 1,
			);
			if (!performanceSwitches.disableWebglRenderer) {
				try {
					const { WebglAddon } = await import("@xterm/addon-webgl");
					const webglAddon = new WebglAddon();
					webglAddon.onContextLoss(() => {
						webglAddon.dispose();
					});
					terminal.loadAddon(webglAddon);
				} catch (error) {
					console.error(
						"Failed to enable WebGL renderer, falling back to DOM:",
						error,
					);
				}
			}
			let resolveInitialSnapshot!: () => void;
			const initialSnapshot = new Promise<void>((resolve) => {
				resolveInitialSnapshot = resolve;
			});
			let hasSnapshot = false;
			let streamSessionKey = result.session_key;
			let streamProcessing = Promise.resolve();
			let recoveringSinceEpoch: number | null = null;
			let firstChannelReceived = false;
			const attachStream = async (recovery: boolean) => {
				const previousAttachmentId = attachmentId;
				const previousSocket = activeSocket;
				const epoch = ++attachmentEpoch;
				const nextAttachmentId = crypto.randomUUID();
				const acknowledgeOutput = (sequence: number) => {
					if (suppressOutputAcks) return;
					if (
						activeSocket &&
						activeSocket.readyState === WebSocket.OPEN &&
						epoch === attachmentEpoch
					) {
						activeSocket.send(
							JSON.stringify({
								type: "ack",
								attachment_id: nextAttachmentId,
								sequence,
							}),
						);
						return;
					}
					void invoke("ack_terminal_surface_output", {
						attachmentId: nextAttachmentId,
						sequence,
					}).catch((error) => {
						if (!isMounted || epoch !== attachmentEpoch) return;
						const message = getErrorMessage(error);
						const contextualMessage = `Failed to acknowledge terminal output: ${message}`;
						console.error(contextualMessage);
						onTerminalErrorRef.current?.(contextualMessage);
						recoverAttachment?.();
					});
				};
				const applyContext: TerminalStreamApplyContext = {
					isCurrent: () => isMounted && epoch === attachmentEpoch,
					drainLiveOutput,
					resizeTerminal: (cols, rows) => terminal.resize(cols, rows),
					writeToTerminal,
					applySnapshotIdentity: (sessionKey) => {
						streamSessionKey = sessionKey;
					},
					syncPtySizeAfterEmptySnapshot: () => {
						const { rows, cols } = terminal;
						syncPtySize(rows, cols);
					},
					reportSnapshotReplayParsed: (sequence) => {
						if (shouldReportTerminalSnapshotLaunchPhase(sequence)) {
							reportFirstXtermParsed();
						}
					},
					completeRecovery: () => {
						if (!recovery) return;
						liveOutputScheduler.resumeAfterSnapshot();
						if (recoveringSinceEpoch === epoch) {
							recoveringSinceEpoch = null;
						}
					},
					setRunning: (running) => {
						isRunningRef.current = running;
					},
					completeInitialSnapshot: () => {
						if (hasSnapshot) return;
						hasSnapshot = true;
						resolveInitialSnapshot();
					},
					flushStartupInput: () => {
						if (startupInput.isDone) return;
						const buffered = startupInput.markDone();
						if (isRunningRef.current) {
							for (const chunk of buffered) deliverInput(chunk);
						}
					},
					takeOutputTraceSequence: () =>
						performanceProbeActive
							? pendingPerformanceInputSequences.shift()
							: undefined,
					reportOutputTracePoint: reportTerminalInputPerformancePoint,
					enqueueOutput: (data, onParsed) =>
						liveOutputScheduler.enqueue(data, onParsed),
					acknowledgeOutput,
					reportInputUnavailable: (message) => {
						console.error(message);
						onTerminalErrorRef.current?.(message);
						recoverAttachment?.();
					},
				};
				const handleStreamItem = (item: TerminalSurfaceStreamItem) => {
					if (!firstChannelReceived) {
						firstChannelReceived = true;
						reportTerminalPerformancePhase(
							"channel_receive",
							performance.now() - requestStartedAt,
						);
					}
					streamProcessing = streamProcessing.then(() =>
						applyTerminalStreamItem(item, applyContext).catch((error) => {
							if (!isMounted || epoch !== attachmentEpoch) return;
							const message = `Failed to apply terminal stream item: ${getErrorMessage(error)}`;
							console.error(message);
							onTerminalErrorRef.current?.(message);
							recoverAttachment?.();
						}),
					);
				};
				// hot path（stream配信・write・ack）はWebSocketを優先する。
				// Tauri ChannelはメッセージごとにmacOSメインスレッドのevalを経由し、
				// 高頻度出力時に配送待ち行列が入力遅延・invoke応答遅延の支配要因になる。
				let nextSocket: WebSocket | null = null;
				if (streamEndpoint && !wsTransportFailed) {
					try {
						nextSocket = await openTerminalStreamSocket(
							streamEndpoint,
							{ attachmentId: nextAttachmentId, owner: terminalOwner },
							{
								isClosedByUs: (socket) => socketsClosedByUs.has(socket),
								onUnexpectedClose: (socket) => {
									if (!isMounted || epoch !== attachmentEpoch) return;
									// 予期しない切断はChannel transportへ切り替えて単発resyncする
									wsTransportFailed = true;
									if (activeSocket === socket) activeSocket = null;
									recoverAttachment?.(epoch);
								},
								onStreamItem: handleStreamItem,
								onStreamError: (message) => {
									if (!isMounted || epoch !== attachmentEpoch) return;
									console.error(message);
									onTerminalErrorRef.current?.(message);
									recoverAttachment?.();
								},
							},
						);
					} catch (error) {
						console.error(
							"Terminal WebSocket attach failed, falling back to Tauri Channel:",
							error,
						);
						wsTransportFailed = true;
					}
				}
				if (!nextSocket) {
					const nextChannel = new Channel<TerminalSurfaceStreamItem>();
					nextChannel.onmessage = handleStreamItem;
					await invokeTerminalBackendCommand("attach_terminal_surface", {
						owner: terminalOwner,
						attachmentId: nextAttachmentId,
						recovery,
						onEvent: nextChannel,
					});
				}
				// 入力の宛先はbackendがattachmentを受理した後にだけ切り替える。
				// 先に切り替えると、attach完了前の打鍵が新attachment IDと
				// sequence 0..Nで送られて棄却され、以後の入力sequenceが恒久的に
				// 欠番となり全打鍵が無音でバッファされ続ける。
				activeSocket = nextSocket;
				attachmentId = nextAttachmentId;
				inputSequence = 0;
				pendingPerformanceInputSequences = [];
				if (previousSocket) {
					socketsClosedByUs.add(previousSocket);
					previousSocket.close();
				} else if (previousAttachmentId) {
					await invokeTerminalBackendCommand("detach_terminal_surface", {
						attachmentId: previousAttachmentId,
					});
				}
			};
			recoverAttachment = (failedEpoch) => {
				if (!isMounted) return;
				if (
					recoveringSinceEpoch !== null &&
					recoveringSinceEpoch === attachmentEpoch &&
					failedEpoch !== attachmentEpoch
				) {
					return;
				}
				const attempt = attachStream(true);
				const attemptEpoch = attachmentEpoch;
				recoveringSinceEpoch = attemptEpoch;
				void attempt.then(
					() => {
						if (!isMounted) {
							releaseCurrentAttachment();
							return;
						}
						if (attemptEpoch !== attachmentEpoch) return;
						onTerminalErrorRef.current?.(null);
					},
					(error) => {
						if (recoveringSinceEpoch === attemptEpoch) {
							recoveringSinceEpoch = null;
						}
						if (!isMounted) return;
						const message =
							error instanceof TerminalBackendCommandError
								? error.message
								: `Failed to resynchronize terminal: ${getErrorMessage(error)}`;
						console.error(message);
						onTerminalErrorRef.current?.(message);
					},
				);
			};
			await attachStream(false);
			if (!isMounted) {
				releaseCurrentAttachment();
				return;
			}
			const initialized = await Promise.race([
				initialSnapshot.then(() => true),
				unmounted.then(() => false),
			]);
			if (!initialized) return;
			if (autoFocus) terminal.focus();
			if (!isMounted) return;
			onTerminalErrorRef.current?.(null);
			if (!isRunningRef.current) return;
			onTerminalReadyRef.current?.(streamSessionKey);

			// 初回fit()が不正確だった場合のセーフティネット:
			// PTYスポーン後に最新のサイズで再同期する
			requestAnimationFrame(() => {
				if (!isMounted || !fitAddonRef.current || !terminalRef.current) return;
				fitAddonRef.current.fit();
				const { rows, cols } = terminalRef.current;
				syncPtySize(rows, cols);
			});
		};

		initTerminal().catch((error) => {
			console.error("Failed to initialize PTY:", error);
			if (!isMounted) return;
			const message =
				error instanceof TerminalBackendCommandError
					? error.message
					: `Failed to initialize terminal: ${getErrorMessage(error)}`;
			onTerminalErrorRef.current?.(message);
		});

		deliverInput = (data: string) => {
			const activeAttachmentId = attachmentId;
			if (!activeAttachmentId) return;
			const writeEpoch = attachmentEpoch;
			const sequence = inputSequence;
			inputSequence += 1;
			const clientStartedAtUnixMs = performanceProbeActive
				? Date.now()
				: undefined;
			if (performanceProbeActive) {
				pendingPerformanceInputSequences.push(sequence);
				reportTerminalInputPerformancePoint(sequence, "on_data");
			}
			if (activeSocket && activeSocket.readyState === WebSocket.OPEN) {
				activeSocket.send(
					JSON.stringify({
						type: "write",
						owner: terminalOwner,
						attachment_id: activeAttachmentId,
						sequence,
						data,
						...(clientStartedAtUnixMs === undefined
							? {}
							: { client_started_at_unix_ms: clientStartedAtUnixMs }),
					}),
				);
				return;
			}
			void invoke<void>("write_terminal_surface", {
				owner: terminalOwner,
				attachmentId: activeAttachmentId,
				sequence,
				data,
				...(clientStartedAtUnixMs === undefined
					? {}
					: { clientStartedAtUnixMs }),
			}).catch((error) => {
				if (!isMounted || writeEpoch !== attachmentEpoch) return;
				const message = getErrorMessage(error);
				console.error("Failed to dispatch terminal input:", error);
				onTerminalErrorRef.current?.(message);
				recoverAttachment?.();
			});
		};
		const dispatchInput = (data: string) => {
			if (!isMounted || data.length === 0) return;
			if (!startupInput.isDone) {
				startupInput.push(data);
				return;
			}
			if (!isRunningRef.current) return;
			deliverInput(data);
		};
		inputDispatchRef.current = dispatchInput;
		terminal.onData((data) => {
			if (suppressImeEnterData && (data === "\r" || data === "\x1b\r")) {
				suppressImeEnterData = false;
				return;
			}
			dispatchInput(data);
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
				syncPtySize(rows, cols);
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

		return () => {
			isMounted = false;
			resolveUnmount();
			unregisterBufferReader();
			inputDispatchRef.current = () => {};
			liveOutputScheduler.dispose();
			if (resizeTimer !== null) {
				clearTimeout(resizeTimer);
			}
			resizeObserver.disconnect();
			releaseCurrentAttachment();
			if (killOnUnmountRef.current && isRunningRef.current) {
				invoke("kill_terminal_surface", { owner: terminalOwner }).catch(
					(error) => {
						console.error("Failed to kill terminal PTY on unmount:", error);
					},
				);
			}
			isRunningRef.current = false;
			terminal.dispose();
			reportMountedXtermUnmounted();
		};
	}, [
		containerRef,
		autoFocus,
		cwd,
		initialization,
		label,
		terminalOwner,
		terminalStartupCommand,
	]);

	useEffect(() => {
		const terminal = terminalRef.current;
		const container = containerRef.current;
		if (!terminal || !container) return;
		terminal.options.theme = getTerminalTheme(theme, container);
	}, [theme, containerRef]);

	const sendInput = useCallback((data: string) => {
		inputDispatchRef.current(data);
	}, []);

	const requestKill = useCallback(() => {
		killOnUnmountRef.current = true;
	}, []);

	return {
		terminalRef,
		terminalOwner,
		isRunningRef,
		sendInput,
		requestKill,
	};
}
