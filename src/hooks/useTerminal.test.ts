import { renderHook, waitFor } from "@testing-library/react";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalOutputScheduler } from "@/lib/terminalOutputScheduler";
import { resetTerminalPerformanceSwitchesCache } from "@/lib/terminalPerformanceSwitches";
import { resetTerminalStreamEndpointCache } from "@/lib/terminalStreamEndpoint";
import { useTerminal } from "./useTerminal";

class MockWebSocket {
	static instances: MockWebSocket[] = [];
	static readonly CONNECTING = 0;
	static readonly OPEN = 1;
	static readonly CLOSING = 2;
	static readonly CLOSED = 3;
	readyState = MockWebSocket.CONNECTING;
	sent: string[] = [];
	onopen: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onmessage: ((event: { data: string }) => void) | null = null;
	url: string;
	protocols?: string[];

	constructor(url: string, protocols?: string[]) {
		this.url = url;
		this.protocols = protocols;
		MockWebSocket.instances.push(this);
	}

	send(data: string) {
		this.sent.push(data);
	}

	close() {
		this.readyState = MockWebSocket.CLOSED;
		this.onclose?.();
	}

	open() {
		this.readyState = MockWebSocket.OPEN;
		this.onopen?.();
	}

	// 実ブラウザは接続失敗時にerror→closeを連鎖発火する
	failConnection() {
		this.onerror?.();
		this.readyState = MockWebSocket.CLOSED;
		this.onclose?.();
	}

	receive(payload: unknown) {
		this.onmessage?.({ data: JSON.stringify(payload) });
	}

	acceptAttach() {
		const attach = this.sent
			.map((frame) => JSON.parse(frame))
			.find((frame) => frame.type === "attach_surface");
		if (!attach) throw new Error("attach_surface frame is missing");
		this.receive({ status: "attached", id: attach.id });
	}
}
vi.stubGlobal("WebSocket", MockWebSocket);

const mockWebglAddonInstances: Array<{
	onContextLoss: ReturnType<typeof vi.fn>;
	dispose: ReturnType<typeof vi.fn>;
}> = [];
vi.mock("@xterm/addon-webgl", () => ({
	WebglAddon: class MockWebglAddon {
		onContextLoss = vi.fn();
		dispose = vi.fn();
		constructor() {
			mockWebglAddonInstances.push(this);
		}
	},
}));

const REPO_WORKSPACE_OWNER = {
	kind: "workspace",
	workspacePath: "/repo",
} as const;

const mockInvoke = vi.fn();
const mockListen = vi.fn();
const mockOpenUrl = vi.fn();
const mockChannels: Array<{ onmessage: (message: unknown) => void }> = [];
let mockChannelConstructionError: Error | null = null;
let mockOnDataCallback: (data: string) => void = () => {};
let mockTerminalConstructorOptions: Record<string, unknown> = {};
let mockTerminalInstance: {
	loadAddon: ReturnType<typeof vi.fn>;
	open: ReturnType<typeof vi.fn>;
	element: HTMLElement | undefined;
	focus: ReturnType<typeof vi.fn>;
	write: ReturnType<typeof vi.fn>;
	resize: ReturnType<typeof vi.fn>;
	input: ReturnType<
		typeof vi.fn<(data: string, wasUserInput?: boolean) => void>
	>;
	onData: ReturnType<typeof vi.fn>;
	attachCustomKeyEventHandler: ReturnType<typeof vi.fn>;
	dispose: ReturnType<typeof vi.fn>;
	refresh: ReturnType<typeof vi.fn>;
	options: Record<string, unknown>;
	rows: number;
	cols: number;
};

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
	Channel: class MockChannel {
		onmessage = (_message: unknown) => {};

		constructor() {
			if (mockChannelConstructionError) {
				throw mockChannelConstructionError;
			}
			mockChannels.push(this);
		}
	},
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

vi.mock("@xterm/xterm", () => {
	return {
		Terminal: class MockTerminal {
			loadAddon = vi.fn();
			open = vi.fn((parent: HTMLElement) => {
				this.element = document.createElement("div");
				this.element.className = "xterm";
				parent.append(this.element);
			});
			focus = vi.fn();
			write = vi.fn((_data: string, callback?: () => void) => callback?.());
			resize = vi.fn((cols: number, rows: number) => {
				this.cols = cols;
				this.rows = rows;
			});
			input = vi.fn((data: string) => mockOnDataCallback(data));
			onData = vi
				.fn()
				.mockImplementation((callback: (data: string) => void) => {
					mockOnDataCallback = callback;
					return { dispose: vi.fn() };
				});
			attachCustomKeyEventHandler = vi.fn();
			dispose = vi.fn();
			refresh = vi.fn();
			options: Record<string, unknown> = {};
			element: HTMLElement | undefined;
			rows = 24;
			cols = 80;

			constructor(options: Record<string, unknown> = {}) {
				mockTerminalConstructorOptions = options;
				mockTerminalInstance = this;
			}
		},
	};
});

interface KeyboardSequenceInit {
	key: string;
	keyCode: number;
	shiftKey?: boolean;
	metaKey?: boolean;
	ctrlKey?: boolean;
	altKey?: boolean;
	isComposing?: boolean;
}

function dispatchKeyboardSequence(
	init: KeyboardSequenceInit,
	xtermEncodedData: string | null,
) {
	const handler = mockTerminalInstance.attachCustomKeyEventHandler.mock
		.calls[0][0] as (event: KeyboardEvent) => boolean;
	const events = (["keydown", "keypress", "keyup"] as const).map(
		(type) =>
			({
				type,
				key: init.key,
				keyCode: init.keyCode,
				shiftKey: init.shiftKey ?? false,
				metaKey: init.metaKey ?? false,
				ctrlKey: init.ctrlKey ?? false,
				altKey: init.altKey ?? false,
				isComposing: init.isComposing ?? false,
				preventDefault: vi.fn(),
				stopPropagation: vi.fn(),
			}) as unknown as KeyboardEvent,
	);
	const delegated = events.map((event) => handler(event));
	if (
		delegated[0] &&
		xtermEncodedData !== null &&
		!events[0].isComposing &&
		events[0].keyCode !== 229
	) {
		mockTerminalInstance.input(xtermEncodedData, true);
	}
	return { delegated, events };
}

function dispatchXtermCompositionEnter(
	init: KeyboardSequenceInit,
	compositionData: string,
) {
	const handler = mockTerminalInstance.attachCustomKeyEventHandler.mock
		.calls[0][0] as (event: KeyboardEvent) => boolean;
	const event = {
		type: "keydown",
		key: init.key,
		keyCode: init.keyCode,
		shiftKey: init.shiftKey ?? false,
		metaKey: init.metaKey ?? false,
		ctrlKey: init.ctrlKey ?? false,
		altKey: init.altKey ?? false,
		isComposing: init.isComposing ?? false,
		preventDefault: vi.fn(),
		stopPropagation: vi.fn(),
	} as unknown as KeyboardEvent;
	const delegated = handler(event);
	if (delegated) {
		if (event.keyCode === 229) {
			mockOnDataCallback(compositionData);
		} else {
			mockOnDataCallback(compositionData);
			mockOnDataCallback(event.altKey ? "\x1b\r" : "\r");
		}
	}
	return { delegated, event };
}

function terminalInputWrites() {
	return mockInvoke.mock.calls
		.filter(([command]) => command === "write_terminal_surface")
		.map(
			([, args]) =>
				args as {
					owner: unknown;
					sequence: number;
					data: string;
				},
		);
}

let mockFitAddonInstance: { fit: ReturnType<typeof vi.fn> };

interface MockWebLinksAddon {
	handler: (event: MouseEvent, url: string) => void;
	options: {
		hover: (event: MouseEvent, url: string, range: unknown) => void;
		leave: (event: MouseEvent, url: string) => void;
		urlRegex?: RegExp;
	};
}

function getMockWebLinksAddon(): MockWebLinksAddon {
	const addon = mockTerminalInstance.loadAddon.mock.calls
		.map(([loadedAddon]) => loadedAddon)
		.find((loadedAddon) => loadedAddon instanceof WebLinksAddon);
	if (!addon) throw new Error("WebLinksAddon was not loaded");
	return addon as unknown as MockWebLinksAddon;
}

vi.mock("@xterm/addon-fit", () => {
	return {
		FitAddon: class MockFitAddon {
			fit = vi.fn();

			constructor() {
				mockFitAddonInstance = this;
			}
		},
	};
});

let resizeObserverCallback: () => void;
let resizeObserverDisconnect: ReturnType<typeof vi.fn<() => void>>;

class MockResizeObserver {
	constructor(callback: () => void) {
		resizeObserverCallback = callback;
	}
	observe = vi.fn();
	disconnect = vi.fn().mockImplementation(() => {
		resizeObserverDisconnect?.();
	});
	unobserve = vi.fn();
}

resizeObserverDisconnect = vi.fn();
vi.stubGlobal("ResizeObserver", MockResizeObserver);

describe("useTerminal", () => {
	let containerRef: { current: HTMLDivElement | null };

	beforeEach(() => {
		vi.clearAllMocks();
		mockInvoke.mockReset();
		mockListen.mockReset();
		mockOpenUrl.mockReset().mockResolvedValue(undefined);
		mockChannels.length = 0;
		mockChannelConstructionError = null;
		mockTerminalConstructorOptions = {};
		mockWebglAddonInstances.length = 0;
		MockWebSocket.instances.length = 0;
		resetTerminalPerformanceSwitchesCache();
		resetTerminalStreamEndpointCache();
		delete window.__RELEASH_TERMINAL_PERFORMANCE__;

		containerRef = { current: document.createElement("div") };

		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (
					cmd === "get_or_spawn_terminal_surface" ||
					cmd === "get_terminal_surface"
				) {
					return Promise.resolve({
						session_key: "test-uuid-1234",
						terminal_surface: {
							replay: "",
							sequence: 0,
							cols: 80,
							rows: 24,
						},
						restored_from_checkpoint: false,
						is_new: true,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					queueMicrotask(() => {
						channel.onmessage({
							type: "snapshot",
							surface: {
								session_key: "test-uuid-1234",
								terminal_surface: {
									replay: "",
									sequence: 0,
									cols: 80,
									rows: 24,
								},
								is_exited: false,
								exit_code: null,
							},
						});
					});
					return Promise.resolve();
				}
				return Promise.resolve();
			},
		);
	});

	it("Terminal と FitAddon が正しく生成される", () => {
		renderHook(() => useTerminal(containerRef));

		expect(mockTerminalInstance).toBeDefined();
		expect(mockTerminalInstance.loadAddon).toHaveBeenCalled();
		expect(mockTerminalInstance.open).toHaveBeenCalledWith(
			containerRef.current,
		);
	});

	it("Terminal constructor optionsへlinkHandlerを渡す", () => {
		renderHook(() => useTerminal(containerRef));

		expect(mockTerminalConstructorOptions.linkHandler).toEqual({
			activate: expect.any(Function),
			hover: expect.any(Function),
			leave: expect.any(Function),
		});
	});

	it("OSC 8 linkのactivateでopenUrlを使いwindow.openを呼ばない", () => {
		const windowOpen = vi.spyOn(window, "open");
		renderHook(() => useTerminal(containerRef));
		const linkHandler = mockTerminalConstructorOptions.linkHandler as {
			activate: (event: MouseEvent, url: string, range: unknown) => void;
		};
		const url = "https://example.com/osc-8";

		linkHandler.activate(new MouseEvent("click"), url, {});

		expect(mockOpenUrl).toHaveBeenCalledWith(url);
		expect(windowOpen).not.toHaveBeenCalled();
		windowOpen.mockRestore();
	});

	it("WebLinksAddonをTerminal初期化時に同期loadする", () => {
		renderHook(() => useTerminal(containerRef));

		expect(mockTerminalInstance.loadAddon).toHaveBeenCalledWith(
			expect.any(WebLinksAddon),
		);
		expect(mockWebglAddonInstances).toHaveLength(0);
	});

	it("WebLinksAddon handlerでopenUrlを呼ぶ", () => {
		const windowOpen = vi.spyOn(window, "open");
		renderHook(() => useTerminal(containerRef));
		const webLinksAddon = getMockWebLinksAddon();
		const url = "https://example.com/plain-text";

		webLinksAddon.handler(new MouseEvent("click"), url);

		expect(mockOpenUrl).toHaveBeenCalledWith(url);
		expect(windowOpen).not.toHaveBeenCalled();
		windowOpen.mockRestore();
	});

	it("OSC 8 linkHandlerのhoverとleaveをtooltipへ接続する", () => {
		renderHook(() => useTerminal(containerRef));
		const linkHandler = mockTerminalConstructorOptions.linkHandler as {
			hover: (event: MouseEvent, url: string, range: unknown) => void;
			leave: (event: MouseEvent, url: string, range: unknown) => void;
		};
		const url = "https://example.com/osc-8-tooltip";

		linkHandler.hover(new MouseEvent("mousemove"), url, {});

		expect(mockTerminalInstance.element).toHaveTextContent(url);
		expect(
			mockTerminalInstance.element?.querySelector(".xterm-hover"),
		).toHaveClass("xterm-hover");

		linkHandler.leave(new MouseEvent("mouseleave"), url, {});

		expect(
			mockTerminalInstance.element?.querySelector(".xterm-hover"),
		).toBeNull();
	});

	it("WebLinksAddon optionsのhoverとleaveをtooltipへ接続する", () => {
		renderHook(() => useTerminal(containerRef));
		const webLinksAddon = getMockWebLinksAddon();
		const url = "https://example.com/plain-text-tooltip";

		webLinksAddon.options.hover(new MouseEvent("mousemove"), url, {});

		expect(mockTerminalInstance.element).toHaveTextContent(url);

		webLinksAddon.options.leave(new MouseEvent("mouseleave"), url);

		expect(
			mockTerminalInstance.element?.querySelector(".xterm-hover"),
		).toBeNull();
	});

	it("WebLinksAddon optionsで既定urlRegexを上書きしない", () => {
		renderHook(() => useTerminal(containerRef));
		const webLinksAddon = getMockWebLinksAddon();

		expect(webLinksAddon.options).not.toHaveProperty("urlRegex");
	});

	it("linkHandlerでallowNonHttpProtocolsを有効にしない", () => {
		renderHook(() => useTerminal(containerRef));

		expect(mockTerminalConstructorOptions.linkHandler).not.toHaveProperty(
			"allowNonHttpProtocols",
		);
	});

	it("unmount時に表示中のlink tooltipを解放する", () => {
		const { unmount } = renderHook(() => useTerminal(containerRef));
		const linkHandler = mockTerminalConstructorOptions.linkHandler as {
			hover: (event: MouseEvent, url: string, range: unknown) => void;
		};
		linkHandler.hover(
			new MouseEvent("mousemove"),
			"https://example.com/cleanup",
			{},
		);
		const terminalElement = mockTerminalInstance.element;

		unmount();

		expect(terminalElement?.querySelector(".xterm-hover")).toBeNull();
	});

	it("既定でWebGL addonをロードしcontext loss時のfallbackを設定する", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockWebglAddonInstances).toHaveLength(1);
		});
		expect(mockTerminalInstance.loadAddon).toHaveBeenCalledWith(
			mockWebglAddonInstances[0],
		);
		expect(mockWebglAddonInstances[0].onContextLoss).toHaveBeenCalled();
	});

	it("disableWebglRenderer switch時はWebGL addonをロードしない", async () => {
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_terminal_performance_switches") {
					return Promise.resolve({
						disableOutputFlowControl: false,
						disableTerminalJournal: false,
						disableTerminalWebsocket: false,
						disableRendererWriteSerialization: false,
						disableWebglRenderer: true,
					});
				}
				return baseImplementation?.(cmd, args);
			},
		);

		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels.length).toBeGreaterThan(0);
		});
		expect(mockWebglAddonInstances).toHaveLength(0);
	});

	it("dark themeは専用の黒背景とxterm標準ANSI paletteを使う", () => {
		const container = containerRef.current;
		if (!container) throw new Error("terminal container is missing");
		container.style.backgroundColor = "rgb(0, 0, 0)";

		renderHook(() => useTerminal(containerRef, { theme: "dark" }));

		const terminalTheme = mockTerminalConstructorOptions.theme as Record<
			string,
			unknown
		>;
		expect(terminalTheme.background).toBe("rgb(0, 0, 0)");
		expect(terminalTheme).not.toHaveProperty("black");
		expect(terminalTheme).not.toHaveProperty("yellow");
		expect(terminalTheme).not.toHaveProperty("brightYellow");
	});

	it("get_or_spawn_terminal_surface が正しい引数で呼び出される", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_or_spawn_terminal_surface", {
				rows: 24,
				cols: 80,
				cwd: null,
				owner: { kind: "workspace", workspacePath: "" },
				label: null,
				startupCommand: null,
			});
		});
	});

	it("backend attachment Channelを使いglobal PTY listenerを登録しない", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("attach_terminal_surface", {
				owner: { kind: "workspace", workspacePath: "" },
				attachmentId: expect.any(String),
				recovery: false,
				onEvent: mockChannels[0],
			});
		});
		expect(mockChannels).toHaveLength(1);
		expect(mockListen).not.toHaveBeenCalled();
	});

	it("unmountはstream購読だけを解除しPTY lifecycleを変更しない", async () => {
		const { unmount } = renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo" }),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});

		unmount();
		expect(mockInvoke).toHaveBeenCalledWith("detach_terminal_surface", {
			attachmentId: expect.any(String),
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"register_active_terminal",
			expect.anything(),
		);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"unregister_active_terminal",
			expect.anything(),
		);
	});

	it("remount後も同じownerを使用する", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_terminal_surface") {
					return Promise.resolve({
						session_key: "same-session",
						terminal_surface: {
							replay: "",
							sequence: 0,
							cols: 80,
							rows: 24,
						},
						restored_from_checkpoint: false,
						is_new: false,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "same-session",
							terminal_surface: {
								replay: "",
								sequence: 0,
								cols: 80,
								rows: 24,
							},
							is_exited: false,
							exit_code: null,
						},
					});
				}
				return Promise.resolve();
			},
		);

		const first = renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo", owner: REPO_WORKSPACE_OWNER }),
		);
		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.objectContaining({ owner: REPO_WORKSPACE_OWNER }),
			);
		});

		first.unmount();

		mockInvoke.mockClear();
		const secondContainerRef = { current: document.createElement("div") };
		renderHook(() =>
			useTerminal(secondContainerRef, {
				cwd: "/repo",
				owner: REPO_WORKSPACE_OWNER,
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.objectContaining({ owner: REPO_WORKSPACE_OWNER }),
			);
		});
	});

	it("unmount後にget_or_spawnが完了してもmanaged paneへsession keyを通知する", async () => {
		type SpawnResult = {
			session_key: string;
			terminal_surface: {
				replay: string;
				sequence: number;
				cols: number;
				rows: number;
			};
			restored_from_checkpoint: boolean;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") return pendingSpawn;
			return Promise.resolve();
		});
		const onTerminalReady = vi.fn();

		const { unmount } = renderHook(() =>
			useTerminal(containerRef, {
				cwd: "/repo",
				label: "repo terminal",
				onTerminalReady,
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});
		unmount();
		mockInvoke.mockClear();

		resolveSpawn({
			session_key: "late-session",
			terminal_surface: {
				replay: "",
				sequence: 0,
				cols: 80,
				rows: 24,
			},
			restored_from_checkpoint: false,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(onTerminalReady).toHaveBeenCalledWith("late-session");
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"kill_terminal_surface",
			expect.anything(),
		);
	});

	it("pending kill中に遅れて生成されたmanaged PTYはready通知せず終了する", async () => {
		type SpawnResult = {
			session_key: string;
			terminal_surface: {
				replay: string;
				sequence: number;
				cols: number;
				rows: number;
			};
			restored_from_checkpoint: boolean;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") return pendingSpawn;
			return Promise.resolve();
		});
		const onTerminalReady = vi.fn();
		const shouldKillPendingTerminal = vi.fn(() => false);

		const { unmount } = renderHook(() =>
			useTerminal(containerRef, {
				cwd: "/repo",
				label: "repo terminal",
				onTerminalReady,
				shouldKillPendingTerminal,
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});
		shouldKillPendingTerminal.mockReturnValue(true);
		unmount();
		mockInvoke.mockClear();

		resolveSpawn({
			session_key: "late-session",
			terminal_surface: {
				replay: "",
				sequence: 0,
				cols: 80,
				rows: 24,
			},
			restored_from_checkpoint: false,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("kill_terminal_surface", {
				owner: REPO_WORKSPACE_OWNER,
			});
		});
		expect(onTerminalReady).not.toHaveBeenCalled();
	});

	it("requestKill() 後に get_or_spawn が解決した pending PTY は onTerminalReady を呼ばない", async () => {
		type SpawnResult = {
			session_key: string;
			terminal_surface: {
				replay: string;
				sequence: number;
				cols: number;
				rows: number;
			};
			restored_from_checkpoint: boolean;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") return pendingSpawn;
			return Promise.resolve();
		});
		const onTerminalReady = vi.fn();

		const { result, unmount } = renderHook(() =>
			useTerminal(containerRef, {
				cwd: "/repo",
				label: "repo terminal",
				onTerminalReady,
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});
		result.current.requestKill();
		unmount();
		mockInvoke.mockClear();

		resolveSpawn({
			session_key: "late-session",
			terminal_surface: {
				replay: "",
				sequence: 0,
				cols: 80,
				rows: 24,
			},
			restored_from_checkpoint: false,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		expect(onTerminalReady).not.toHaveBeenCalled();
	});

	it("requestKill() 後に get_or_spawn が解決した pending PTY を kill する", async () => {
		type SpawnResult = {
			session_key: string;
			terminal_surface: {
				replay: string;
				sequence: number;
				cols: number;
				rows: number;
			};
			restored_from_checkpoint: boolean;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") return pendingSpawn;
			return Promise.resolve();
		});

		const { result, unmount } = renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo" }),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});
		result.current.requestKill();
		unmount();
		mockInvoke.mockClear();

		resolveSpawn({
			session_key: "late-session",
			terminal_surface: {
				replay: "",
				sequence: 0,
				cols: 80,
				rows: 24,
			},
			restored_from_checkpoint: false,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("kill_terminal_surface", {
				owner: REPO_WORKSPACE_OWNER,
			});
		});
	});

	it("PTY_ERRORのbackend messageをalert用callbackだけへ通知する", async () => {
		const onTerminalError = vi.fn();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") {
				return Promise.reject({
					code: "PTY_ERROR",
					message: "Terminal initialization failed. Try again.",
				});
			}
			return Promise.resolve();
		});

		renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo", onTerminalError }),
		);

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Terminal initialization failed. Try again.",
			);
		});
		expect(mockTerminalInstance.write).not.toHaveBeenCalledWith(
			"\r\n\x1b[31mTerminal initialization failed. Try again.\x1b[0m\r\n",
		);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"register_active_terminal",
			expect.objectContaining({ sessionKey: "test-uuid-1234" }),
		);
	});

	it("プレーン文字列の初期化失敗をそのまま通知する", async () => {
		const onTerminalError = vi.fn();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") {
				return Promise.reject("Terminal initialization failed. Try again.");
			}
			return Promise.resolve();
		});

		renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo", onTerminalError }),
		);

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Terminal initialization failed. Try again.",
			);
		});
	});

	it("frontend内部の初期化失敗には操作文脈を付けて通知する", async () => {
		const onTerminalError = vi.fn();
		const schedulerSetup = vi
			.spyOn(TerminalOutputScheduler.prototype, "setMaxWritesInFlight")
			.mockImplementationOnce(() => {
				throw new Error("renderer attachment setup failed");
			});

		try {
			renderHook(() =>
				useTerminal(containerRef, { cwd: "/repo", onTerminalError }),
			);

			await waitFor(() => {
				expect(onTerminalError).toHaveBeenCalledWith(
					"Failed to initialize terminal: renderer attachment setup failed",
				);
			});
		} finally {
			schedulerSetup.mockRestore();
		}
	});

	it("ユーザー入力時に write_terminal_surface が呼び出される", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"attach_terminal_surface",
				expect.any(Object),
			);
		});
		const attachmentCall = mockInvoke.mock.calls.find(
			([command]) => command === "attach_terminal_surface",
		);
		if (!attachmentCall)
			throw new Error("attach_terminal_surface call is missing");
		const attachmentId = (attachmentCall[1] as { attachmentId: string })
			.attachmentId;

		mockOnDataCallback("test input");

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("write_terminal_surface", {
				owner: { kind: "workspace", workspacePath: "" },
				attachmentId,
				sequence: 0,
				data: "test input",
			});
		});
	});

	it("Channel write失敗を無加工で通知し新attachmentへ自動resyncする", async () => {
		const onTerminalError = vi.fn();
		const onTerminalReady = vi.fn();
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "write_terminal_surface") {
					return Promise.reject("Terminal input could not be sent. Try again.");
				}
				return baseImplementation?.(cmd, args);
			},
		);

		renderHook(() =>
			useTerminal(containerRef, { onTerminalError, onTerminalReady }),
		);
		await waitFor(() => {
			expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
		});
		onTerminalError.mockClear();

		mockOnDataCallback("test input");

		await waitFor(() => {
			expect(onTerminalError.mock.calls).toEqual([
				["Terminal input could not be sent. Try again."],
				[null],
			]);
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "attach_terminal_surface",
				),
			).toHaveLength(2);
		});
	});

	it("未解決の応答があってもIME確定、Enter、次keyを到着順にdispatchする", async () => {
		let completeFirstWrite!: () => void;
		const firstWrite = new Promise<void>((resolve) => {
			completeFirstWrite = resolve;
		});
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"attach_terminal_surface",
				expect.any(Object),
			);
		});
		mockInvoke.mockClear();
		mockInvoke.mockImplementation((command: string) => {
			if (command === "write_terminal_surface") return firstWrite;
			return Promise.resolve();
		});

		mockOnDataCallback("変換");
		mockOnDataCallback("\r");
		mockOnDataCallback("次");

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(3);
		});
		expect(mockInvoke.mock.calls.map(([, args]) => args.data)).toEqual([
			"変換",
			"\r",
			"次",
		]);
		expect(mockInvoke.mock.calls.map(([, args]) => args.sequence)).toEqual([
			0, 1, 2,
		]);
		expect(
			new Set(mockInvoke.mock.calls.map(([, args]) => args.attachmentId)).size,
		).toBe(1);
		completeFirstWrite();
	});

	it("Rustが通知した入力不能をattachment streamから表示する", async () => {
		const onTerminalError = vi.fn();
		renderHook(() => useTerminal(containerRef, { onTerminalError }));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"attach_terminal_surface",
				expect.any(Object),
			);
		});
		mockChannels[mockChannels.length - 1]?.onmessage({
			type: "input_unavailable",
			session_key: "test-uuid-1234",
			message: "Terminal input could not be sent. Try again.",
		});
		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Terminal input could not be sent. Try again.",
			);
		});
	});

	it("input_unavailable受信時は新attachmentへ一度だけ自動resyncする", async () => {
		const onTerminalError = vi.fn();
		const onTerminalReady = vi.fn();
		renderHook(() =>
			useTerminal(containerRef, { onTerminalError, onTerminalReady }),
		);

		await waitFor(() => {
			expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
		});
		onTerminalError.mockClear();
		const firstAttachCall = mockInvoke.mock.calls.find(
			([command]) => command === "attach_terminal_surface",
		);
		if (!firstAttachCall)
			throw new Error("attach_terminal_surface call is missing");
		const firstAttachmentId = (firstAttachCall[1] as { attachmentId: string })
			.attachmentId;

		mockChannels[0].onmessage({
			type: "input_unavailable",
			session_key: "test-uuid-1234",
			message: "Terminal input could not be sent. Try again.",
		});

		await waitFor(() => {
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "attach_terminal_surface",
				),
			).toHaveLength(2);
		});
		expect(onTerminalError).toHaveBeenCalledWith(
			"Terminal input could not be sent. Try again.",
		);
		const attachCalls = mockInvoke.mock.calls.filter(
			([command]) => command === "attach_terminal_surface",
		);
		expect(attachCalls).toHaveLength(2);
		expect(attachCalls[0][1]).toEqual(
			expect.objectContaining({ recovery: false }),
		);
		expect(attachCalls[1][1]).toEqual(
			expect.objectContaining({ recovery: true }),
		);
		expect(onTerminalError.mock.calls).toEqual([
			["Terminal input could not be sent. Try again."],
			[null],
		]);
		const secondAttachmentId = (attachCalls[1][1] as { attachmentId: string })
			.attachmentId;
		expect(secondAttachmentId).not.toBe(firstAttachmentId);
	});

	it("resync commandのプレーン文字列rejectを接頭辞なしで通知する", async () => {
		const onTerminalError = vi.fn();
		let attachCalls = 0;
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "attach_terminal_surface") {
					attachCalls += 1;
					if (attachCalls === 2) {
						return Promise.reject("backend resync failed");
					}
				}
				return baseImplementation?.(cmd, args);
			},
		);
		renderHook(() => useTerminal(containerRef, { onTerminalError }));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockChannels[0].onmessage({
			type: "input_unavailable",
			session_key: "test-uuid-1234",
			message: "stale attachment",
		});

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith("backend resync failed");
		});
	});

	it("stream itemのapply失敗を通知し新attachmentへのresync成功でクリアする", async () => {
		const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		const onTerminalError = vi.fn();
		const onTerminalReady = vi.fn();
		renderHook(() =>
			useTerminal(containerRef, { onTerminalError, onTerminalReady }),
		);

		await waitFor(() => {
			expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
		});
		onTerminalError.mockClear();
		mockTerminalInstance.resize.mockImplementationOnce(() => {
			throw new Error("resize boom");
		});

		mockChannels[0].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 120,
			rows: 40,
			sequence: 1,
		});
		await waitFor(() => {
			expect(onTerminalError.mock.calls).toEqual([
				["Failed to apply terminal stream item: resize boom"],
				[null],
			]);
			expect(mockChannels).toHaveLength(2);
		});
		expect(errorSpy).toHaveBeenCalledWith(
			"Failed to apply terminal stream item: resize boom",
		);
		errorSpy.mockRestore();
	});

	it("recovery中にstream itemのapply失敗が連続してもresyncを張り直さない", async () => {
		const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		const attachResolvers: Array<() => void> = [];
		let attachCalls = 0;
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "attach_terminal_surface") {
					attachCalls += 1;
					if (attachCalls === 1) {
						const channel = args?.onEvent as {
							onmessage: (message: unknown) => void;
						};
						queueMicrotask(() => {
							channel.onmessage({
								type: "snapshot",
								surface: {
									session_key: "test-uuid-1234",
									terminal_surface: {
										replay: "",
										sequence: 0,
										cols: 80,
										rows: 24,
									},
									is_exited: false,
									exit_code: null,
								},
							});
						});
						return Promise.resolve();
					}
					return new Promise<void>((resolve) => {
						attachResolvers.push(resolve);
					});
				}
				return baseImplementation?.(cmd, args);
			},
		);

		const onTerminalReady = vi.fn();
		renderHook(() => useTerminal(containerRef, { onTerminalReady }));
		await waitFor(() => {
			expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
		});
		mockTerminalInstance.resize.mockImplementation(() => {
			throw new Error("resize boom");
		});

		mockChannels[0].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 120,
			rows: 40,
			sequence: 1,
		});
		await waitFor(() => {
			expect(mockChannels).toHaveLength(2);
			expect(attachResolvers).toHaveLength(1);
		});
		mockChannels[1].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 121,
			rows: 41,
			sequence: 2,
		});
		mockChannels[1].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 122,
			rows: 42,
			sequence: 3,
		});

		await waitFor(() => {
			expect(errorSpy).toHaveBeenCalledTimes(3);
		});
		expect(attachCalls).toBe(2);
		attachResolvers[0]();
		errorSpy.mockRestore();
	});

	it("unmount中に完了したrecovery attachは新attachmentをreleaseする", async () => {
		const pendingAttachResolvers: Array<() => void> = [];
		let attachCalls = 0;
		const attachmentIds: string[] = [];
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "attach_terminal_surface") {
					attachCalls += 1;
					attachmentIds.push(String(args?.attachmentId));
					if (attachCalls >= 2) {
						return new Promise<void>((resolve) => {
							pendingAttachResolvers.push(() => resolve());
						});
					}
				}
				return baseImplementation?.(cmd, args);
			},
		);

		const { unmount } = renderHook(() => useTerminal(containerRef));
		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockChannels[0].onmessage({
			type: "input_unavailable",
			session_key: "test-uuid-1234",
			message: "stale attachment",
		});
		await waitFor(() => {
			expect(attachCalls).toBe(2);
		});

		unmount();
		pendingAttachResolvers[0]?.();

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("detach_terminal_surface", {
				attachmentId: attachmentIds[1],
			});
		});
	});

	it("resync中のfrontend内部例外には操作文脈を付けて通知する", async () => {
		const onTerminalError = vi.fn();
		renderHook(() => useTerminal(containerRef, { onTerminalError }));
		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockChannelConstructionError = new Error("renderer recovery setup failed");
		mockChannels[0].onmessage({
			type: "input_unavailable",
			session_key: "test-uuid-1234",
			message: "stale attachment",
		});

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Failed to resynchronize terminal: renderer recovery setup failed",
			);
		});
	});

	it("resync中のdetach command rejectを接頭辞なしで通知する", async () => {
		const onTerminalError = vi.fn();
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "detach_terminal_surface") {
					return Promise.reject({
						code: "PTY_ERROR",
						message: "Terminal detachment failed. Try again.",
					});
				}
				return baseImplementation?.(cmd, args);
			},
		);
		renderHook(() => useTerminal(containerRef, { onTerminalError }));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockChannels[0].onmessage({
			type: "input_unavailable",
			session_key: "test-uuid-1234",
			message: "stale attachment",
		});

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Terminal detachment failed. Try again.",
			);
		});
	});

	it("アンマウント時にデフォルトでは kill_terminal_surface が呼ばれない（PTY保持）", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});

		unmount();

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"kill_terminal_surface",
			expect.anything(),
		);
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("requestKill() 後のアンマウントで kill_terminal_surface が呼ばれる", async () => {
		const { result, unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});

		result.current.requestKill();
		unmount();

		expect(mockInvoke).toHaveBeenCalledWith("kill_terminal_surface", {
			owner: { kind: "workspace", workspacePath: "" },
		});
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("pty-exit 後のアンマウントでは kill_terminal_surface が呼ばれない", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});

		// process exit後もsurface identityは保持し、書き込みだけを停止する。
		mockChannels[0].onmessage({
			type: "exit",
			session_key: "test-uuid-1234",
			exit_code: 0,
			sequence: 1,
		});
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"\r\n\x1b[90m[Process exited with code 0]\x1b[0m\r\n",
				expect.any(Function),
			);
		});

		unmount();

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"kill_terminal_surface",
			expect.anything(),
		);
	});

	it("backendで検証済みのresizeとexitを受信順に投影する", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
			expect(mockInvoke).toHaveBeenCalledWith(
				"resize_terminal_surface",
				expect.objectContaining({ rows: 24, cols: 80 }),
			);
		});
		expect(mockTerminalInstance.resize).not.toHaveBeenCalled();
		mockTerminalInstance.write.mockClear();

		mockChannels[0].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 111,
			rows: 37,
			sequence: 1,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.resize).toHaveBeenCalledWith(111, 37);
		});
		mockChannels[0].onmessage({
			type: "exit",
			session_key: "test-uuid-1234",
			exit_code: 0,
			sequence: 2,
		});
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"\r\n\x1b[90m[Process exited with code 0]\x1b[0m\r\n",
				expect.any(Function),
			);
		});
	});

	it("renderer queue超過時は一度だけ再attachしsnapshot後のoutputへ復帰する", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockInvoke.mockClear();
		mockTerminalInstance.write.mockImplementation(
			(_data: string, callback?: () => void) => callback?.(),
		);

		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "a".repeat(2 * 1024 * 1024),
			sequence: 1,
		});
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "b".repeat(256 * 1024),
			sequence: 2,
		});

		await waitFor(() => {
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "attach_terminal_surface",
				),
			).toHaveLength(1);
			expect(mockChannels).toHaveLength(2);
		});
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "detach_terminal_surface",
			),
		).toHaveLength(1);

		mockTerminalInstance.write.mockClear();
		mockChannels[1].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "after-resync",
			sequence: 3,
		});
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"after-resync",
				expect.any(Function),
			);
		});
	});

	it("初回snapshot前の打鍵は破棄せずsnapshot適用後に順序どおり送出する", async () => {
		const attachResolvers: Array<() => void> = [];
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "attach_terminal_surface") {
					return new Promise<void>((resolve) => {
						attachResolvers.push(() => resolve());
					});
				}
				return baseImplementation?.(cmd, args);
			},
		);

		renderHook(() => useTerminal(containerRef));
		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});

		mockOnDataCallback("a");
		mockOnDataCallback("b");
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "write_terminal_surface",
			),
		).toHaveLength(0);

		await waitFor(() => {
			expect(attachResolvers).toHaveLength(1);
		});
		attachResolvers[0]();
		mockChannels[0].onmessage({
			type: "snapshot",
			surface: {
				session_key: "test-uuid-1234",
				terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
				is_exited: false,
				exit_code: null,
			},
		});

		await waitFor(() => {
			const writes = mockInvoke.mock.calls.filter(
				([command]) => command === "write_terminal_surface",
			);
			expect(writes).toHaveLength(2);
			expect(writes[0][1]).toMatchObject({ data: "a", sequence: 0 });
			expect(writes[1][1]).toMatchObject({ data: "b", sequence: 1 });
		});
	});

	it("再attach完了前の入力は旧attachmentへ送られ、完了後はsequence 0から新attachmentへ送る", async () => {
		const secondAttachResolvers: Array<() => void> = [];
		let attachCalls = 0;
		const attachmentIds: string[] = [];
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "attach_terminal_surface") {
					attachCalls += 1;
					attachmentIds.push(String(args?.attachmentId));
					if (attachCalls >= 2) {
						return new Promise<void>((resolve) => {
							secondAttachResolvers.push(() => resolve());
						});
					}
				}
				return baseImplementation?.(cmd, args);
			},
		);

		renderHook(() => useTerminal(containerRef));
		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockOnDataCallback("a");
		await waitFor(() => {
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "write_terminal_surface",
				),
			).toHaveLength(1);
		});

		// renderer queue超過で再attach（2回目のattach_terminal_surfaceは未解決のまま保持）
		mockTerminalInstance.write.mockImplementation(
			(_data: string, callback?: () => void) => callback?.(),
		);
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "a".repeat(2 * 1024 * 1024),
			sequence: 1,
		});
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "b".repeat(256 * 1024),
			sequence: 2,
		});
		await waitFor(() => {
			expect(attachCalls).toBe(2);
		});

		// attach未完了の間の打鍵は旧attachmentのsequence連番で送られる
		mockOnDataCallback("x");
		await waitFor(() => {
			const writes = mockInvoke.mock.calls.filter(
				([command]) => command === "write_terminal_surface",
			);
			expect(writes).toHaveLength(2);
			expect(writes[1][1]).toMatchObject({
				data: "x",
				attachmentId: attachmentIds[0],
				sequence: 1,
			});
		});

		// attach完了＋snapshot後は新attachmentへsequence 0から送られる
		secondAttachResolvers[0]?.();
		await waitFor(() => {
			expect(mockChannels).toHaveLength(2);
		});
		const resizePtyCallsBefore = mockInvoke.mock.calls.filter(
			([command]) => command === "resize_terminal_surface",
		).length;
		mockChannels[1].onmessage({
			type: "snapshot",
			surface: {
				session_key: "test-uuid-1234",
				terminal_surface: { replay: "", sequence: 2, cols: 80, rows: 24 },
				is_exited: false,
				exit_code: null,
			},
		});
		await waitFor(() => {
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "resize_terminal_surface",
				).length,
			).toBeGreaterThan(resizePtyCallsBefore);
		});
		mockOnDataCallback("y");
		await waitFor(() => {
			const writes = mockInvoke.mock.calls.filter(
				([command]) => command === "write_terminal_surface",
			);
			expect(writes).toHaveLength(3);
			expect(writes[2][1]).toMatchObject({
				data: "y",
				attachmentId: attachmentIds[1],
				sequence: 0,
			});
		});
	});

	it("stream endpointが有効ならWebSocketでattachし入力・ackもWSで送る", async () => {
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_terminal_stream_endpoint") {
					return Promise.resolve({
						url: "ws://127.0.0.1:9999/v1/terminal",
						authSubprotocol: "releash-bearer.test-token",
					});
				}
				return baseImplementation?.(cmd, args);
			},
		);

		renderHook(() => useTerminal(containerRef));
		await waitFor(() => {
			expect(MockWebSocket.instances).toHaveLength(1);
		});
		const socket = MockWebSocket.instances[0];
		expect(socket.url).toBe("ws://127.0.0.1:9999/v1/terminal");
		expect(socket.protocols).toEqual(["releash-bearer.test-token"]);

		socket.open();
		await waitFor(() => {
			expect(socket.sent).toHaveLength(1);
		});
		const attach = JSON.parse(socket.sent[0]);
		expect(attach.type).toBe("attach_surface");
		expect(attach.attachment_id).toBe(attach.id);
		socket.acceptAttach();

		socket.receive({
			status: "event",
			item: {
				type: "snapshot",
				surface: {
					session_key: "test-uuid-1234",
					terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
					is_exited: false,
					exit_code: null,
				},
			},
		});
		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"resize_terminal_surface",
				expect.objectContaining({ rows: 24, cols: 80 }),
			);
		});

		mockOnDataCallback("a");
		await waitFor(() => {
			expect(socket.sent.length).toBeGreaterThanOrEqual(2);
		});
		const write = JSON.parse(socket.sent[1]);
		expect(write).toMatchObject({
			type: "write",
			attachment_id: attach.attachment_id,
			sequence: 0,
			data: "a",
		});
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "write_terminal_surface",
			),
		).toHaveLength(0);
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "attach_terminal_surface",
			),
		).toHaveLength(0);

		// 出力parse後のackもWSで送られる
		socket.receive({
			status: "event",
			item: {
				type: "output",
				session_key: "test-uuid-1234",
				data: "echo-a",
				sequence: 1,
			},
		});
		await waitFor(() => {
			const ack = socket.sent
				.map((raw) => JSON.parse(raw))
				.find((frame) => frame.type === "ack");
			expect(ack).toMatchObject({
				attachment_id: attach.attachment_id,
				sequence: 1,
			});
		});
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "ack_terminal_surface_output",
			),
		).toHaveLength(0);
	});

	it("WebSocket接続に失敗したらTauri Channelへfallbackする", async () => {
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_terminal_stream_endpoint") {
					return Promise.resolve({
						url: "ws://127.0.0.1:9999/v1/terminal",
						authSubprotocol: "releash-bearer.test-token",
					});
				}
				return baseImplementation?.(cmd, args);
			},
		);

		renderHook(() => useTerminal(containerRef));
		await waitFor(() => {
			expect(MockWebSocket.instances).toHaveLength(1);
		});
		MockWebSocket.instances[0].failConnection();

		await waitFor(() => {
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "attach_terminal_surface",
				),
			).toHaveLength(1);
			expect(mockChannels).toHaveLength(1);
		});
		mockOnDataCallback("a");
		await waitFor(() => {
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "write_terminal_surface",
				),
			).toHaveLength(1);
		});
		// error→close連鎖でfallbackとrecoveryのattachが二重発行されないこと
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "attach_terminal_surface",
			),
		).toHaveLength(1);
		expect(mockChannels).toHaveLength(1);
		expect(MockWebSocket.instances).toHaveLength(1);
	});

	it("初期replay中のprocess exitを失わずsurface identityを保持する", async () => {
		type SpawnResult = {
			session_key: string;
			terminal_surface: {
				replay: string;
				sequence: number;
				cols: number;
				rows: number;
			};
			restored_from_checkpoint: boolean;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		mockInvoke.mockImplementation(
			(command: string, args?: Record<string, unknown>) => {
				if (command === "get_or_spawn_terminal_surface") return pendingSpawn;
				if (command === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "late-exit",
							terminal_surface: {
								replay: "final screen",
								sequence: 4,
								cols: 80,
								rows: 24,
							},
							is_exited: false,
							exit_code: null,
						},
					});
					channel.onmessage({
						type: "exit",
						session_key: "late-exit",
						exit_code: 23,
						sequence: 5,
					});
				}
				return Promise.resolve();
			},
		);

		const onTerminalReady = vi.fn();
		const { result } = renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo", onTerminalReady }),
		);
		resolveSpawn({
			session_key: "late-exit",
			terminal_surface: {
				replay: "final screen",
				sequence: 4,
				cols: 80,
				rows: 24,
			},
			restored_from_checkpoint: false,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(onTerminalReady).toHaveBeenCalledWith("late-exit");
			expect(result.current.isRunningRef.current).toBe(false);
		});
		expect(mockTerminalInstance.write).toHaveBeenCalledWith(
			"\r\n\x1b[90m[Process exited with code 23]\x1b[0m\r\n",
			expect.any(Function),
		);
		mockInvoke.mockClear();
		mockOnDataCallback("must not be written");
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_terminal_surface",
			expect.anything(),
		);
	});

	it("containerRef が null の場合は初期化されない", () => {
		const nullContainerRef = { current: null };
		const previousInstance = mockTerminalInstance;

		renderHook(() => useTerminal(nullContainerRef));

		expect(mockTerminalInstance).toBe(previousInstance);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("既存セッションのTerminal Surfaceがターミナルに書き込まれる", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_terminal_surface") {
					return Promise.resolve({
						session_key: "pre-spawned-key",
						terminal_surface: {
							replay: "previously buffered text\r\n$ ",
							sequence: 3,
							cols: 80,
							rows: 24,
						},
						restored_from_checkpoint: false,
						is_new: false,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "pre-spawned-key",
							terminal_surface: {
								replay: "previously buffered text\r\n$ ",
								sequence: 3,
								cols: 80,
								rows: 24,
							},
							is_exited: false,
							exit_code: null,
						},
					});
				}
				return Promise.resolve();
			},
		);

		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"previously buffered text\r\n$ ",
				expect.any(Function),
			);
		});
	});

	it("attach streamのlive outputをsnapshot後に書き込む", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_terminal_surface") {
					return Promise.resolve({
						session_key: "pre-spawned-key",
						terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
						restored_from_checkpoint: false,
						is_new: false,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "pre-spawned-key",
							terminal_surface: {
								replay: "previously buffered text\r\n$ ",
								sequence: 3,
								cols: 80,
								rows: 24,
							},
							is_exited: false,
							exit_code: null,
						},
					});
					channel.onmessage({
						type: "output",
						session_key: "pre-spawned-key",
						data: "live during init",
						sequence: 4,
					});
				}
				return Promise.resolve();
			},
		);

		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"live during init",
				expect.any(Function),
			);
		});
		expect(mockTerminalInstance.write.mock.calls.map(([data]) => data)).toEqual(
			["previously buffered text\r\n$ ", "live during init"],
		);
	});

	it("live outputはxtermの描画中に到着したchunkを一つにcoalesceする", async () => {
		let completeLiveWrite: (() => void) | undefined;
		let completeCoalescedWrite: (() => void) | undefined;
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
			expect(mockInvoke).toHaveBeenCalledWith(
				"resize_terminal_surface",
				expect.objectContaining({ rows: 24, cols: 80 }),
			);
		});
		mockTerminalInstance.write.mockImplementation(
			(data: string, callback?: () => void) => {
				if (data === "chunk-1") {
					completeLiveWrite = callback;
					return;
				}
				if (data === "chunk-2chunk-3") {
					completeCoalescedWrite = callback;
					return;
				}
				callback?.();
			},
		);

		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "chunk-1",
			sequence: 1,
		});
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "chunk-2",
			sequence: 2,
		});
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "chunk-3",
			sequence: 3,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"chunk-1",
				expect.any(Function),
			);
		});
		expect(mockTerminalInstance.write).not.toHaveBeenCalledWith(
			"chunk-2",
			expect.anything(),
		);
		expect(mockTerminalInstance.write).not.toHaveBeenCalledWith(
			"chunk-3",
			expect.anything(),
		);
		completeLiveWrite?.();
		expect(mockTerminalInstance.write).not.toHaveBeenCalledWith(
			"chunk-2chunk-3",
			expect.anything(),
		);
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"chunk-2chunk-3",
				expect.any(Function),
			);
		});
		completeCoalescedWrite?.();
	});

	it("live outputはxterm parse完了後だけattachmentへ累積ACKする", async () => {
		let parsed!: () => void;
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		const attachmentCall = mockInvoke.mock.calls.find(
			([command]) => command === "attach_terminal_surface",
		);
		const attachmentId = (
			attachmentCall?.[1] as { attachmentId?: string } | undefined
		)?.attachmentId;
		expect(attachmentId).toEqual(expect.any(String));
		mockTerminalInstance.write.mockImplementation(
			(_data: string, callback?: () => void) => {
				if (callback) parsed = callback;
			},
		);
		mockInvoke.mockClear();

		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "provider output",
			sequence: 7,
		});
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"provider output",
				expect.any(Function),
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"ack_terminal_surface_output",
			expect.anything(),
		);

		parsed();
		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("ack_terminal_surface_output", {
				attachmentId,
				sequence: 7,
			});
		});
	});

	it("ack commandのIPC失敗へ操作文脈を付けて通知する", async () => {
		const onTerminalError = vi.fn();
		const baseImplementation = mockInvoke.getMockImplementation();
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "ack_terminal_surface_output") {
					return Promise.reject("IPC bridge unavailable");
				}
				return baseImplementation?.(cmd, args);
			},
		);
		renderHook(() => useTerminal(containerRef, { onTerminalError }));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "provider output",
			sequence: 7,
		});

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Failed to acknowledge terminal output: IPC bridge unavailable",
			);
		});
	});

	it("性能probe有効時はfirst parseとfirst paintを匿名backend phaseへ記録する", async () => {
		window.__RELEASH_TERMINAL_PERFORMANCE__ = {
			recordInputPoint: vi.fn(),
			recordPhase: vi.fn(),
			recordRendererMetrics: vi.fn(),
		};
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "first provider frame",
			sequence: 1,
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"record_terminal_launch_renderer_phase",
				{
					phase: "first_xterm_parsed",
					durationMs: expect.any(Number),
				},
			);
			expect(mockInvoke).toHaveBeenCalledWith(
				"record_terminal_launch_renderer_phase",
				{
					phase: "first_paint",
					durationMs: expect.any(Number),
				},
			);
		});
	});

	it("AgentSession作成開始時刻からfirst parseとpaintまでを同一runとして記録する", async () => {
		const launchOrigin = performance.now() - 50;
		const takeLaunchOrigin = vi.fn().mockReturnValue(launchOrigin);
		window.__RELEASH_TERMINAL_PERFORMANCE__ = {
			recordInputPoint: vi.fn(),
			recordPhase: vi.fn(),
			recordRendererMetrics: vi.fn(),
			takeLaunchOrigin,
		};

		renderHook(() =>
			useTerminal(containerRef, {
				cwd: "/repo",
				theme: "dark",
				owner: {
					kind: "session",
					workspacePath: "/repo",
					sessionId: "agent-session-1",
				},
				label: "Codex AgentSession",
				initialization: "attach-existing",
			}),
		);

		await waitFor(() => expect(mockChannels).toHaveLength(1));
		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "first provider frame",
			sequence: 1,
		});

		await waitFor(() => {
			expect(takeLaunchOrigin).toHaveBeenCalledWith("agent-session-1");
			const parsed = mockInvoke.mock.calls.find(
				([command, args]) =>
					command === "record_terminal_launch_renderer_phase" &&
					(args as { phase?: string }).phase === "first_xterm_parsed",
			);
			expect(
				(parsed?.[1] as { durationMs?: number } | undefined)?.durationMs,
			).toBeGreaterThanOrEqual(50);
		});
	});

	it("backend resizeはstream順にxtermへ投影する", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
		});
		mockTerminalInstance.resize.mockClear();
		mockChannels[0].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 111,
			rows: 37,
			sequence: 1,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.resize).toHaveBeenCalledWith(111, 37);
		});
	});

	it("checkpointサイズへの復元とreplay完了後にだけqueued live outputを反映する", async () => {
		let resolveSpawn!: (value: unknown) => void;
		const pendingSpawn = new Promise((resolve) => {
			resolveSpawn = resolve;
		});
		let completeReplay: (() => void) | undefined;
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_terminal_surface") {
					return pendingSpawn;
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "terminal-surface",
							terminal_surface: {
								replay: "semantic checkpoint",
								sequence: 9,
								cols: 111,
								rows: 37,
							},
							is_exited: false,
							exit_code: null,
						},
					});
					channel.onmessage({
						type: "output",
						session_key: "terminal-surface",
						data: "live after checkpoint",
						sequence: 10,
					});
				}
				return Promise.resolve();
			},
		);

		const onTerminalReady = vi.fn();
		renderHook(() => useTerminal(containerRef, { onTerminalReady }));
		mockTerminalInstance.write.mockImplementation(
			(data: string, callback?: () => void) => {
				if (data === "semantic checkpoint") {
					completeReplay = callback;
					return;
				}
				callback?.();
			},
		);
		resolveSpawn({
			session_key: "terminal-surface",
			terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
			restored_from_checkpoint: false,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.resize).toHaveBeenCalledWith(111, 37);
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"semantic checkpoint",
				expect.any(Function),
			);
		});
		expect(mockTerminalInstance.write).not.toHaveBeenCalledWith(
			"live after checkpoint",
		);
		expect(onTerminalReady).not.toHaveBeenCalled();
		expect(
			mockTerminalInstance.resize.mock.invocationCallOrder[0],
		).toBeLessThan(
			mockTerminalInstance.write.mock.invocationCallOrder.find(
				(_, index) =>
					mockTerminalInstance.write.mock.calls[index]?.[0] ===
					"semantic checkpoint",
			) ?? Number.POSITIVE_INFINITY,
		);

		completeReplay?.();
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"live after checkpoint",
				expect.any(Function),
			);
			expect(onTerminalReady).toHaveBeenCalledWith("terminal-surface");
		});
		const replayCall = mockTerminalInstance.write.mock.calls.findIndex(
			([data]) => data === "semantic checkpoint",
		);
		const liveCall = mockTerminalInstance.write.mock.calls.findIndex(
			([data]) => data === "live after checkpoint",
		);
		expect(replayCall).toBeGreaterThanOrEqual(0);
		expect(liveCall).toBeGreaterThan(replayCall);
	});

	it("frontendでsequence連続性を再判定しない", () => {
		const source = useTerminal.toString();

		expect(source).not.toContain("lastSequence");
		expect(source).not.toMatch(/sequence\s*>/);
	});

	it("backend resync snapshot以後のoutputだけを継続適用する", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_terminal_surface") {
					return Promise.resolve({
						session_key: "pre-spawned-key",
						terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
						restored_from_checkpoint: false,
						is_new: false,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					for (const message of [
						{
							type: "snapshot",
							surface: {
								session_key: "pre-spawned-key",
								terminal_surface: {
									replay: "stale backend replay",
									sequence: 0,
									cols: 80,
									rows: 24,
								},
								is_exited: false,
								exit_code: null,
							},
						},
						{
							type: "snapshot",
							surface: {
								session_key: "pre-spawned-key",
								terminal_surface: {
									replay: "backend replay through 256",
									sequence: 256,
									cols: 80,
									rows: 24,
								},
								is_exited: false,
								exit_code: null,
							},
						},
						{
							type: "output",
							session_key: "pre-spawned-key",
							data: "chunk-257",
							sequence: 257,
						},
					]) {
						channel.onmessage(message);
					}
				}
				return Promise.resolve();
			},
		);

		renderHook(() =>
			useTerminal(containerRef, { cwd: "/repo", owner: REPO_WORKSPACE_OWNER }),
		);

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"chunk-257",
				expect.any(Function),
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"get_terminal_surface",
			expect.anything(),
		);
		expect(mockTerminalInstance.write.mock.calls.map(([data]) => data)).toEqual(
			["stale backend replay", "backend replay through 256", "chunk-257"],
		);
	});

	it("起動コマンドをget_or_spawnへ渡しfrontendでは新規復元判定をしない", async () => {
		renderHook(() =>
			useTerminal(containerRef, {
				cwd: null,
				terminalStartupCommand: "startup-cmd",
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.objectContaining({ startupCommand: "startup-cmd" }),
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_terminal_surface",
			expect.objectContaining({ data: "startup-cmd\n" }),
		);
	});

	it("AgentSessionは既存Terminal Surfaceにattachして入力focusを得る", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_terminal_surface") {
					return Promise.resolve({
						session_key: "agent-session-surface",
						terminal_surface: {
							replay: "provider screen",
							sequence: 4,
							cols: 80,
							rows: 24,
						},
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "agent-session-surface",
							terminal_surface: {
								replay: "provider screen",
								sequence: 4,
								cols: 80,
								rows: 24,
							},
							is_exited: false,
							exit_code: null,
						},
					});
				}
				return Promise.resolve();
			},
		);

		renderHook(() =>
			useTerminal(containerRef, {
				cwd: "/repo",
				owner: {
					kind: "session",
					workspacePath: "/repo",
					sessionId: "agent-session-1",
				},
				initialization: "attach-existing",
				autoFocus: true,
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_terminal_surface", {
				owner: {
					kind: "session",
					workspacePath: "/repo",
					sessionId: "agent-session-1",
				},
			});
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"get_or_spawn_terminal_surface",
			expect.anything(),
		);
		expect(mockTerminalInstance.focus).toHaveBeenCalledTimes(1);
	});

	it("cold restoreでは新しいPTYでも起動コマンドを自動再実行しない", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_terminal_surface") {
					return Promise.resolve({
						session_key: "restored-session",
						terminal_surface: {
							replay: "restored screen",
							sequence: 9,
							cols: 80,
							rows: 24,
						},
						restored_from_checkpoint: true,
						is_new: true,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_terminal_surface") {
					const channel = args?.onEvent as {
						onmessage: (message: unknown) => void;
					};
					channel.onmessage({
						type: "snapshot",
						surface: {
							session_key: "restored-session",
							terminal_surface: {
								replay: "restored screen",
								sequence: 9,
								cols: 80,
								rows: 24,
							},
							is_exited: false,
							exit_code: null,
						},
					});
				}
				return Promise.resolve();
			},
		);

		renderHook(() =>
			useTerminal(containerRef, {
				cwd: null,
				terminalStartupCommand: "startup-cmd",
			}),
		);

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"restored screen",
				expect.any(Function),
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_terminal_surface",
			expect.objectContaining({ data: "startup-cmd\n" }),
		);
	});

	it("既存セッション（is_new: false）のとき起動コマンドが送信されない", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_terminal_surface") {
				return Promise.resolve({
					session_key: "test-uuid-existing",
					terminal_surface: {
						replay: "",
						sequence: 0,
						cols: 80,
						rows: 24,
					},
					restored_from_checkpoint: false,
					is_new: false,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});

		renderHook(() =>
			useTerminal(containerRef, {
				cwd: null,
				terminalStartupCommand: "startup-cmd",
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_terminal_surface",
				expect.any(Object),
			);
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_terminal_surface",
			expect.objectContaining({ data: "startup-cmd\n" }),
		);
	});

	describe("attachCustomKeyEventHandler", () => {
		const agentSessionOwner = {
			kind: "session",
			workspacePath: "/repo",
			sessionId: "agent-session-1",
		} as const;
		const surfaces = [
			{
				label: "workspace",
				owner: REPO_WORKSPACE_OWNER,
				options: { cwd: "/repo", owner: REPO_WORKSPACE_OWNER },
			},
			{
				label: "Agent session",
				owner: agentSessionOwner,
				options: {
					cwd: "/repo",
					owner: agentSessionOwner,
					initialization: "attach-existing" as const,
				},
			},
		];

		describe.each(surfaces)("$label terminal surface", (surface) => {
			async function renderRunningSurface() {
				const hook = renderHook(() =>
					useTerminal(containerRef, surface.options),
				);
				await waitFor(() => {
					expect(hook.result.current.isRunningRef.current).toBe(true);
				});
				mockInvoke.mockClear();
				return hook;
			}

			it.each([
				{
					label: "Shift+Enter",
					modifiers: { shiftKey: true },
				},
				{
					label: "Cmd+Enter",
					modifiers: { metaKey: true },
				},
			])("$labelをESC+CRとして一度だけ送る", async ({ modifiers }) => {
				await renderRunningSurface();

				const { delegated, events } = dispatchKeyboardSequence(
					{ key: "Enter", keyCode: 13, ...modifiers },
					"\r",
				);

				expect(delegated).toEqual([false, false, false]);
				expect(events[0].preventDefault).toHaveBeenCalledTimes(1);
				expect(events[1].preventDefault).not.toHaveBeenCalled();
				expect(events[2].preventDefault).not.toHaveBeenCalled();
				expect(mockTerminalInstance.input).toHaveBeenCalledTimes(1);
				expect(mockTerminalInstance.input).toHaveBeenCalledWith("\x1b\r", true);
				expect(terminalInputWrites()).toEqual([
					expect.objectContaining({
						owner: surface.owner,
						sequence: 0,
						data: "\x1b\r",
					}),
				]);
			});

			it("修飾キーなしのEnterをCRのまま送る", async () => {
				await renderRunningSurface();

				const { delegated } = dispatchKeyboardSequence(
					{ key: "Enter", keyCode: 13 },
					"\r",
				);

				expect(delegated).toEqual([true, true, true]);
				expect(terminalInputWrites()).toEqual([
					expect.objectContaining({
						owner: surface.owner,
						sequence: 0,
						data: "\r",
					}),
				]);
			});

			it.each([
				{
					label: "isComposingかつkeyCode 13の修飾キーなし",
					event: { keyCode: 13, isComposing: true },
				},
				{
					label: "isComposingかつkeyCode 13のShift",
					event: { keyCode: 13, isComposing: true, shiftKey: true },
				},
				{
					label: "isComposingかつkeyCode 13のCmd",
					event: { keyCode: 13, isComposing: true, metaKey: true },
				},
				{
					label: "isComposingかつkeyCode 13のCtrl",
					event: { keyCode: 13, isComposing: true, ctrlKey: true },
				},
				{
					label: "isComposingかつkeyCode 13のAlt",
					event: { keyCode: 13, isComposing: true, altKey: true },
				},
				{
					label: "keyCode 229かつ修飾キーなし",
					event: { keyCode: 229 },
				},
				{
					label: "keyCode 229かつShift",
					event: { keyCode: 229, shiftKey: true },
				},
				{
					label: "keyCode 229かつCmd",
					event: { keyCode: 229, metaKey: true },
				},
				{
					label: "keyCode 229かつCtrl",
					event: { keyCode: 229, ctrlKey: true },
				},
				{
					label: "keyCode 229かつAlt",
					event: { keyCode: 229, altKey: true },
				},
			])("$labelのEnterで確定文字列だけを一度送る", async ({ event }) => {
				await renderRunningSurface();

				const { delegated, event: keydown } = dispatchXtermCompositionEnter(
					{ key: "Enter", ...event },
					"確定文字列",
				);

				expect(delegated).toBe(true);
				expect(keydown.preventDefault).not.toHaveBeenCalled();
				expect(mockTerminalInstance.input).not.toHaveBeenCalled();
				expect(terminalInputWrites()).toEqual([
					expect.objectContaining({
						owner: surface.owner,
						sequence: 0,
						data: "確定文字列",
					}),
				]);
			});

			it.each([
				{
					label: "Ctrl+Enter",
					event: { key: "Enter", keyCode: 13, ctrlKey: true },
					encoded: "\r",
				},
				{
					label: "Alt+Enter",
					event: { key: "Enter", keyCode: 13, altKey: true },
					encoded: "\x1b\r",
				},
				{
					label: "Shift+Ctrl+Enter",
					event: {
						key: "Enter",
						keyCode: 13,
						shiftKey: true,
						ctrlKey: true,
					},
					encoded: "\r",
				},
				{
					label: "Shift+Alt+Enter",
					event: {
						key: "Enter",
						keyCode: 13,
						shiftKey: true,
						altKey: true,
					},
					encoded: "\x1b\r",
				},
				{
					label: "Cmd+Ctrl+Enter",
					event: {
						key: "Enter",
						keyCode: 13,
						metaKey: true,
						ctrlKey: true,
					},
					encoded: "\r",
				},
				{
					label: "Cmd+Alt+Enter",
					event: {
						key: "Enter",
						keyCode: 13,
						metaKey: true,
						altKey: true,
					},
					encoded: "\x1b\r",
				},
				{
					label: "Shift+Cmd+Enter",
					event: {
						key: "Enter",
						keyCode: 13,
						shiftKey: true,
						metaKey: true,
					},
					encoded: "\r",
				},
				{
					label: "文字キー",
					event: { key: "a", keyCode: 65 },
					encoded: "a",
				},
				{
					label: "矢印キー",
					event: { key: "ArrowRight", keyCode: 39 },
					encoded: "\x1b[C",
				},
			])("$labelをxtermの既存入力へ委譲する", async ({ event, encoded }) => {
				await renderRunningSurface();

				const { delegated, events } = dispatchKeyboardSequence(event, encoded);

				expect(delegated).toEqual([true, true, true]);
				expect(events[0].preventDefault).not.toHaveBeenCalled();
				expect(terminalInputWrites()).toEqual([
					expect.objectContaining({
						owner: surface.owner,
						sequence: 0,
						data: encoded,
					}),
				]);
			});

			it.each([
				{
					label: "Cmd+D",
					event: { key: "d", keyCode: 68, metaKey: true },
				},
				{
					label: "Cmd+Shift+D",
					event: {
						key: "D",
						keyCode: 68,
						metaKey: true,
						shiftKey: true,
					},
				},
				...(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"] as const).map(
					(key) => ({
						label: `Cmd+Option+${key}`,
						event: { key, keyCode: 0, metaKey: true, altKey: true },
					}),
				),
			])("$labelをPTY入力へ送らない", async ({ event }) => {
				await renderRunningSurface();

				const { delegated } = dispatchKeyboardSequence(event, null);

				expect(delegated).toEqual([false, false, false]);
				expect(mockTerminalInstance.input).not.toHaveBeenCalled();
				expect(terminalInputWrites()).toEqual([]);
			});
		});
	});

	describe("ResizeObserver ゼロサイズガード", () => {
		it("コンテナが 0 次元のとき fitAddon.fit() が呼ばれない", () => {
			renderHook(() => useTerminal(containerRef));

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 0,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 0,
				configurable: true,
			});

			mockFitAddonInstance.fit.mockClear();
			resizeObserverCallback();

			expect(mockFitAddonInstance.fit).not.toHaveBeenCalled();
		});

		it("コンテナが 0→正の次元に戻ったとき terminal.refresh() が呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 0,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 0,
				configurable: true,
			});
			resizeObserverCallback();

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 800,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 600,
				configurable: true,
			});
			resizeObserverCallback();

			await vi.waitFor(() => {
				expect(mockTerminalInstance.refresh).toHaveBeenCalledWith(0, 23);
			});
		});

		it("ゼロサイズ時に resize_terminal_surface が呼ばれない", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_terminal_surface",
					expect.any(Object),
				);
			});

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 0,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 0,
				configurable: true,
			});

			mockInvoke.mockClear();
			resizeObserverCallback();

			expect(mockInvoke).not.toHaveBeenCalledWith(
				"resize_terminal_surface",
				expect.any(Object),
			);
		});
	});

	describe("リサイズデバウンス", () => {
		it("連続リサイズ時に resize_terminal_surface がデバウンスされ1回だけ呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_terminal_surface",
					expect.any(Object),
				);
			});

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 800,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 600,
				configurable: true,
			});

			mockInvoke.mockClear();

			resizeObserverCallback();
			resizeObserverCallback();
			resizeObserverCallback();

			// デバウンス中なので即座には呼ばれない
			expect(mockInvoke).not.toHaveBeenCalledWith(
				"resize_terminal_surface",
				expect.any(Object),
			);

			// デバウンス後に1回だけ呼ばれることを検証
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("resize_terminal_surface", {
					owner: { kind: "workspace", workspacePath: "" },
					rows: 24,
					cols: 80,
				});
			});

			const resizeCalls = mockInvoke.mock.calls.filter(
				(call) => call[0] === "resize_terminal_surface",
			);
			expect(resizeCalls).toHaveLength(1);
		});

		it("非表示復帰時はデバウンスなしで即座に resize_terminal_surface が呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_terminal_surface",
					expect.any(Object),
				);
			});

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 0,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 0,
				configurable: true,
			});
			resizeObserverCallback();

			mockInvoke.mockClear();

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 800,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 600,
				configurable: true,
			});
			resizeObserverCallback();

			// 非表示復帰は即座にリサイズ（デバウンスなし）
			expect(mockInvoke).toHaveBeenCalledWith("resize_terminal_surface", {
				owner: { kind: "workspace", workspacePath: "" },
				rows: 24,
				cols: 80,
			});
		});

		it("デバウンス保留中にアンマウントしてもエラーが発生しない", async () => {
			const { unmount } = renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_terminal_surface",
					expect.any(Object),
				);
			});

			Object.defineProperty(containerRef.current, "clientWidth", {
				value: 800,
				configurable: true,
			});
			Object.defineProperty(containerRef.current, "clientHeight", {
				value: 600,
				configurable: true,
			});

			resizeObserverCallback();
			mockInvoke.mockClear();
			unmount();

			// デバウンスタイムアウト(100ms)より長く待つ
			await new Promise((resolve) => setTimeout(resolve, 200));

			expect(mockInvoke).not.toHaveBeenCalledWith(
				"resize_terminal_surface",
				expect.any(Object),
			);
		});
	});

	describe("初回fit()の再実行", () => {
		it("マウント時に fitAddon.fit() が同期呼び出し後にRAFで再実行される", async () => {
			renderHook(() => useTerminal(containerRef));

			// 同期的な初回fit()は即座に呼ばれる
			expect(mockFitAddonInstance.fit).toHaveBeenCalledTimes(1);

			// RAFでの再実行 + PTYスポーン後のRAFで追加呼び出しが発生する
			await vi.waitFor(() => {
				expect(
					mockFitAddonInstance.fit.mock.calls.length,
				).toBeGreaterThanOrEqual(2);
			});
		});
	});

	describe("PTYスポーン後のリサイズ再同期", () => {
		it("get_or_spawn_terminal_surface の後に resize_terminal_surface が requestAnimationFrame で呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_terminal_surface",
					expect.any(Object),
				);
			});

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("resize_terminal_surface", {
					owner: { kind: "workspace", workspacePath: "" },
					rows: 24,
					cols: 80,
				});
			});
		});
	});

	describe("WS切断リカバリとack経路", () => {
		const streamEndpoint = {
			url: "ws://127.0.0.1:9999/v1/terminal",
			authSubprotocol: "releash-bearer.test-token",
		};
		const wsSnapshot = {
			status: "event",
			item: {
				type: "snapshot",
				surface: {
					session_key: "test-uuid-1234",
					terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
					is_exited: false,
					exit_code: null,
				},
			},
		};

		const mockStreamEndpoint = () => {
			const baseImplementation = mockInvoke.getMockImplementation();
			mockInvoke.mockImplementation(
				(cmd: string, args?: Record<string, unknown>) => {
					if (cmd === "get_terminal_stream_endpoint") {
						return Promise.resolve(streamEndpoint);
					}
					return baseImplementation?.(cmd, args);
				},
			);
		};

		it("初期化中のWS stream errorをresyncし初期化完走時にクリアする", async () => {
			mockStreamEndpoint();
			const onTerminalError = vi.fn();
			const onTerminalReady = vi.fn();

			renderHook(() =>
				useTerminal(containerRef, { onTerminalError, onTerminalReady }),
			);
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const socket = MockWebSocket.instances[0];
			socket.open();
			await waitFor(() => {
				expect(socket.sent).toHaveLength(1);
			});
			socket.acceptAttach();
			socket.receive({
				status: "error",
				error: {
					code: "PTY_ERROR",
					message: "Terminal input could not be sent. Try again.",
				},
			});

			expect(onTerminalError).toHaveBeenCalledWith(
				"Terminal input could not be sent. Try again.",
			);
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});
			const recoverySocket = MockWebSocket.instances[1];
			recoverySocket.open();
			await waitFor(() => {
				expect(recoverySocket.sent).toHaveLength(1);
			});
			recoverySocket.acceptAttach();
			recoverySocket.receive(wsSnapshot);

			await waitFor(() => {
				expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
			});
			expect(onTerminalError).toHaveBeenCalledWith(null);
		});

		it("古いepochの再同期成功では新しい再同期失敗をクリアしない", async () => {
			mockStreamEndpoint();
			const onTerminalError = vi.fn();
			const onTerminalReady = vi.fn();

			renderHook(() =>
				useTerminal(containerRef, { onTerminalError, onTerminalReady }),
			);
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const first = MockWebSocket.instances[0];
			first.open();
			await waitFor(() => {
				expect(first.sent).toHaveLength(1);
			});
			first.acceptAttach();
			first.receive(wsSnapshot);
			await waitFor(() => {
				expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
			});
			onTerminalError.mockClear();

			first.receive({
				status: "event",
				item: {
					type: "input_unavailable",
					session_key: "test-uuid-1234",
					message: "stale attachment",
				},
			});
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});
			const second = MockWebSocket.instances[1];
			second.open();
			await waitFor(() => {
				expect(second.sent).toHaveLength(1);
			});
			mockChannelConstructionError = new Error(
				"renderer recovery setup failed",
			);
			const closeFirst = first.close.bind(first);
			first.close = () => {
				closeFirst();
				second.close();
			};

			second.acceptAttach();

			await waitFor(() => {
				expect(onTerminalError).toHaveBeenCalledWith(
					"Failed to resynchronize terminal: renderer recovery setup failed",
				);
			});
			await Promise.resolve();
			expect(onTerminalError).not.toHaveBeenCalledWith(null);
			expect(onTerminalError).toHaveBeenLastCalledWith(
				"Failed to resynchronize terminal: renderer recovery setup failed",
			);
		});

		it("snapshot後の予期しないWS切断はChannelへ単発resyncし以後の入力はinvoke経路になる", async () => {
			mockStreamEndpoint();

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const socket = MockWebSocket.instances[0];
			socket.open();
			await waitFor(() => {
				expect(socket.sent).toHaveLength(1);
			});
			const wsAttachmentId = JSON.parse(socket.sent[0]).attachment_id as string;
			socket.acceptAttach();
			socket.receive(wsSnapshot);
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"resize_terminal_surface",
					expect.objectContaining({ rows: 24, cols: 80 }),
				);
			});

			// server側の予期しない切断（socketsClosedByUs外）
			socket.close();

			await waitFor(() => {
				expect(
					mockInvoke.mock.calls.filter(
						([command]) => command === "attach_terminal_surface",
					),
				).toHaveLength(1);
				expect(mockChannels).toHaveLength(1);
			});
			const attachCall = mockInvoke.mock.calls.find(
				([command]) => command === "attach_terminal_surface",
			);
			if (!attachCall)
				throw new Error("attach_terminal_surface call is missing");
			const channelAttachmentId = (attachCall[1] as { attachmentId: string })
				.attachmentId;
			expect(channelAttachmentId).not.toBe(wsAttachmentId);
			expect(MockWebSocket.instances).toHaveLength(1);

			mockOnDataCallback("a");
			await waitFor(() => {
				const writes = mockInvoke.mock.calls.filter(
					([command]) => command === "write_terminal_surface",
				);
				expect(writes).toHaveLength(1);
				expect(writes[0][1]).toMatchObject({
					attachmentId: channelAttachmentId,
					sequence: 0,
					data: "a",
				});
			});
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "attach_terminal_surface",
				),
			).toHaveLength(1);
		});

		it("WS stream errorはbackend messageを無加工で通知しresync成功でクリアする", async () => {
			mockStreamEndpoint();
			const onTerminalError = vi.fn();
			const onTerminalReady = vi.fn();

			renderHook(() =>
				useTerminal(containerRef, { onTerminalError, onTerminalReady }),
			);
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const socket = MockWebSocket.instances[0];
			socket.open();
			await waitFor(() => {
				expect(socket.sent).toHaveLength(1);
			});
			socket.acceptAttach();
			socket.receive(wsSnapshot);
			await waitFor(() => {
				expect(onTerminalReady).toHaveBeenCalledWith("test-uuid-1234");
			});
			onTerminalError.mockClear();
			socket.receive({
				status: "error",
				error: {
					code: "PTY_ERROR",
					message: "Terminal input could not be sent. Try again.",
				},
			});

			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});
			const recoverySocket = MockWebSocket.instances[1];
			recoverySocket.open();
			await waitFor(() => {
				expect(recoverySocket.sent).toHaveLength(1);
			});
			recoverySocket.acceptAttach();
			recoverySocket.receive(wsSnapshot);

			await waitFor(() => {
				expect(onTerminalError.mock.calls).toEqual([
					["Terminal input could not be sent. Try again."],
					[null],
				]);
			});
			expect(onTerminalError).not.toHaveBeenCalledWith(
				expect.stringContaining("Terminal stream error:"),
			);
		});

		it("messageのないWS error frameは表示せずsocketを閉じてresyncする", async () => {
			mockStreamEndpoint();
			const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
			const onTerminalError = vi.fn();

			try {
				renderHook(() => useTerminal(containerRef, { onTerminalError }));
				await waitFor(() => {
					expect(MockWebSocket.instances).toHaveLength(1);
				});
				const socket = MockWebSocket.instances[0];
				socket.open();
				await waitFor(() => {
					expect(socket.sent).toHaveLength(1);
				});
				socket.acceptAttach();
				socket.receive(wsSnapshot);
				socket.receive({ status: "error", error: { code: "PTY_ERROR" } });

				await waitFor(() => {
					expect(socket.readyState).toBe(MockWebSocket.CLOSED);
					expect(
						mockInvoke.mock.calls.filter(
							([command]) => command === "attach_terminal_surface",
						),
					).toHaveLength(1);
				});
				expect(
					onTerminalError.mock.calls.filter(([message]) => message !== null),
				).toEqual([]);
				expect(warnSpy).toHaveBeenCalledWith(
					"Closing terminal stream after an error frame without a backend message",
				);
			} finally {
				warnSpy.mockRestore();
			}
		});

		it("recovery中のWSがsnapshot前に切断されてもepoch単位で再入し回復する", async () => {
			mockStreamEndpoint();

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const first = MockWebSocket.instances[0];
			first.open();
			await waitFor(() => {
				expect(first.sent).toHaveLength(1);
			});
			first.acceptAttach();
			first.receive(wsSnapshot);
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"resize_terminal_surface",
					expect.objectContaining({ rows: 24, cols: 80 }),
				);
			});

			// renderer queue超過でWS再attach（recovery）を開始する
			mockTerminalInstance.write.mockImplementation(
				(_data: string, callback?: () => void) => callback?.(),
			);
			first.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "a".repeat(2 * 1024 * 1024),
					sequence: 1,
				},
			});
			first.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "b".repeat(256 * 1024),
					sequence: 2,
				},
			});
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});
			const second = MockWebSocket.instances[1];
			second.open();
			await waitFor(() => {
				expect(second.sent).toHaveLength(1);
			});
			second.acceptAttach();
			await waitFor(() => {
				expect(first.readyState).toBe(MockWebSocket.CLOSED);
			});

			// snapshot到達前の予期しない切断でもフリーズせず再入する
			second.close();

			await waitFor(() => {
				expect(
					mockInvoke.mock.calls.filter(
						([command]) => command === "attach_terminal_surface",
					),
				).toHaveLength(1);
				expect(mockChannels).toHaveLength(1);
			});
			mockChannels[0].onmessage({
				type: "output",
				session_key: "test-uuid-1234",
				data: "after-recovery",
				sequence: 3,
			});
			await waitFor(() => {
				expect(mockTerminalInstance.write).toHaveBeenCalledWith(
					"after-recovery",
					expect.any(Function),
				);
			});

			mockOnDataCallback("x");
			await waitFor(() => {
				const writes = mockInvoke.mock.calls.filter(
					([command]) => command === "write_terminal_surface",
				);
				expect(writes).toHaveLength(1);
				expect(writes[0][1]).toMatchObject({ sequence: 0, data: "x" });
			});
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "attach_terminal_surface",
				),
			).toHaveLength(1);
			expect(MockWebSocket.instances).toHaveLength(2);
		});

		it("WS再attach完了前の入力は旧socketへ送られ、完了後はsequence 0から新socketへ送る", async () => {
			mockStreamEndpoint();

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const first = MockWebSocket.instances[0];
			first.open();
			await waitFor(() => {
				expect(first.sent).toHaveLength(1);
			});
			first.acceptAttach();
			const firstAttachmentId = JSON.parse(first.sent[0])
				.attachment_id as string;
			first.receive(wsSnapshot);
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"resize_terminal_surface",
					expect.objectContaining({ rows: 24, cols: 80 }),
				);
			});
			mockOnDataCallback("a");

			// renderer queue超過で再attach（2本目のWSは未openのまま保持）
			mockTerminalInstance.write.mockImplementation(
				(_data: string, callback?: () => void) => callback?.(),
			);
			first.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "a".repeat(2 * 1024 * 1024),
					sequence: 1,
				},
			});
			first.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "b".repeat(256 * 1024),
					sequence: 2,
				},
			});
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});
			const second = MockWebSocket.instances[1];

			// attach未完了の間の打鍵は旧socketへ旧attachmentのsequence連番で送られる
			mockOnDataCallback("x");
			const firstWrites = first.sent
				.map((raw) => JSON.parse(raw))
				.filter((frame) => frame.type === "write");
			expect(firstWrites).toHaveLength(2);
			expect(firstWrites[0]).toMatchObject({
				attachment_id: firstAttachmentId,
				sequence: 0,
				data: "a",
			});
			expect(firstWrites[1]).toMatchObject({
				attachment_id: firstAttachmentId,
				sequence: 1,
				data: "x",
			});

			// attach完了＋snapshot後は新socketへsequence 0から送られる
			second.open();
			await waitFor(() => {
				expect(second.sent).toHaveLength(1);
			});
			second.acceptAttach();
			await waitFor(() => {
				expect(first.readyState).toBe(MockWebSocket.CLOSED);
			});
			const secondAttachmentId = JSON.parse(second.sent[0])
				.attachment_id as string;
			expect(secondAttachmentId).not.toBe(firstAttachmentId);
			second.receive({
				status: "event",
				item: {
					type: "snapshot",
					surface: {
						session_key: "test-uuid-1234",
						terminal_surface: { replay: "", sequence: 2, cols: 80, rows: 24 },
						is_exited: false,
						exit_code: null,
					},
				},
			});
			await waitFor(() => {
				expect(
					mockInvoke.mock.calls.filter(
						([command]) => command === "resize_terminal_surface",
					).length,
				).toBeGreaterThan(1);
			});
			mockOnDataCallback("y");
			const secondWrites = second.sent
				.map((raw) => JSON.parse(raw))
				.filter((frame) => frame.type === "write");
			expect(secondWrites).toHaveLength(1);
			expect(secondWrites[0]).toMatchObject({
				attachment_id: secondAttachmentId,
				sequence: 0,
				data: "y",
			});
			expect(
				mockInvoke.mock.calls.filter(
					([command]) => command === "write_terminal_surface",
				),
			).toHaveLength(0);
		});

		it("stale epochのoutput ackはWSでなくinvoke ackへ落ちる", async () => {
			mockStreamEndpoint();

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const first = MockWebSocket.instances[0];
			first.open();
			await waitFor(() => {
				expect(first.sent).toHaveLength(1);
			});
			first.acceptAttach();
			const firstAttachmentId = JSON.parse(first.sent[0])
				.attachment_id as string;
			first.receive(wsSnapshot);
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"resize_terminal_surface",
					expect.objectContaining({ rows: 24, cols: 80 }),
				);
			});

			let parsed!: () => void;
			mockTerminalInstance.write.mockImplementation(
				(data: string, callback?: () => void) => {
					if (data === "traced-output") {
						if (callback) parsed = callback;
						return;
					}
					callback?.();
				},
			);
			first.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "traced-output",
					sequence: 7,
				},
			});
			await waitFor(() => {
				expect(mockTerminalInstance.write).toHaveBeenCalledWith(
					"traced-output",
					expect.any(Function),
				);
			});

			// overflowで再attachが始まりepochが進む（新socketは未openのまま）
			first.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "a".repeat(2 * 1024 * 1024),
					sequence: 8,
				},
			});
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});

			parsed();
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("ack_terminal_surface_output", {
					attachmentId: firstAttachmentId,
					sequence: 7,
				});
			});
			const ackFrames = first.sent
				.map((raw) => JSON.parse(raw))
				.filter((frame) => frame.type === "ack");
			expect(ackFrames).toHaveLength(0);
		});

		it("disableOutputFlowControl時はWS経路でackを送らない", async () => {
			const baseImplementation = mockInvoke.getMockImplementation();
			mockInvoke.mockImplementation(
				(cmd: string, args?: Record<string, unknown>) => {
					if (cmd === "get_terminal_performance_switches") {
						return Promise.resolve({
							disableOutputFlowControl: true,
							disableTerminalJournal: false,
							disableTerminalWebsocket: false,
							disableRendererWriteSerialization: false,
							disableWebglRenderer: false,
						});
					}
					if (cmd === "get_terminal_stream_endpoint") {
						return Promise.resolve(streamEndpoint);
					}
					return baseImplementation?.(cmd, args);
				},
			);

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const socket = MockWebSocket.instances[0];
			socket.open();
			await waitFor(() => {
				expect(socket.sent).toHaveLength(1);
			});
			socket.acceptAttach();
			socket.receive(wsSnapshot);
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"resize_terminal_surface",
					expect.objectContaining({ rows: 24, cols: 80 }),
				);
			});

			socket.receive({
				status: "event",
				item: {
					type: "output",
					session_key: "test-uuid-1234",
					data: "echo-a",
					sequence: 1,
				},
			});
			await waitFor(() => {
				expect(mockTerminalInstance.write).toHaveBeenCalledWith(
					"echo-a",
					expect.any(Function),
				);
			});
			const ackFrames = socket.sent
				.map((raw) => JSON.parse(raw))
				.filter((frame) => frame.type === "ack");
			expect(ackFrames).toHaveLength(0);
			expect(mockInvoke).not.toHaveBeenCalledWith(
				"ack_terminal_surface_output",
				expect.anything(),
			);
		});

		it("disableOutputFlowControl時はChannel経路でもinvoke ackを送らない", async () => {
			const baseImplementation = mockInvoke.getMockImplementation();
			mockInvoke.mockImplementation(
				(cmd: string, args?: Record<string, unknown>) => {
					if (cmd === "get_terminal_performance_switches") {
						return Promise.resolve({
							disableOutputFlowControl: true,
							disableTerminalJournal: false,
							disableTerminalWebsocket: false,
							disableRendererWriteSerialization: false,
							disableWebglRenderer: false,
						});
					}
					return baseImplementation?.(cmd, args);
				},
			);

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(mockChannels).toHaveLength(1);
			});
			mockChannels[0].onmessage({
				type: "output",
				session_key: "test-uuid-1234",
				data: "provider output",
				sequence: 5,
			});
			await waitFor(() => {
				expect(mockTerminalInstance.write).toHaveBeenCalledWith(
					"provider output",
					expect.any(Function),
				);
			});
			expect(mockInvoke).not.toHaveBeenCalledWith(
				"ack_terminal_surface_output",
				expect.anything(),
			);
		});

		it("unmount中に完了したWS recoveryの新socketはcloseされる", async () => {
			mockStreamEndpoint();

			const { unmount } = renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(1);
			});
			const first = MockWebSocket.instances[0];
			first.open();
			await waitFor(() => {
				expect(first.sent).toHaveLength(1);
			});
			first.acceptAttach();
			first.receive(wsSnapshot);
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"resize_terminal_surface",
					expect.objectContaining({ rows: 24, cols: 80 }),
				);
			});

			first.receive({
				status: "event",
				item: {
					type: "input_unavailable",
					session_key: "test-uuid-1234",
					message: "stale attachment",
				},
			});
			await waitFor(() => {
				expect(MockWebSocket.instances).toHaveLength(2);
			});
			const second = MockWebSocket.instances[1];

			unmount();
			second.open();
			await waitFor(() => {
				expect(second.sent).toHaveLength(1);
			});
			second.acceptAttach();

			await waitFor(() => {
				expect(second.readyState).toBe(MockWebSocket.CLOSED);
			});
		});
	});

	describe("startup input buffer", () => {
		it("1KiB超過分は警告つきで破棄しsnapshot後に超過前分だけ送出する", async () => {
			const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
			const attachResolvers: Array<() => void> = [];
			const baseImplementation = mockInvoke.getMockImplementation();
			mockInvoke.mockImplementation(
				(cmd: string, args?: Record<string, unknown>) => {
					if (cmd === "attach_terminal_surface") {
						return new Promise<void>((resolve) => {
							attachResolvers.push(() => resolve());
						});
					}
					return baseImplementation?.(cmd, args);
				},
			);

			renderHook(() => useTerminal(containerRef));
			await waitFor(() => {
				expect(mockChannels).toHaveLength(1);
			});

			mockOnDataCallback("a".repeat(1024));
			mockOnDataCallback("x");
			expect(warnSpy).toHaveBeenCalledWith(
				expect.stringContaining("Discarding 1 chars"),
			);

			await waitFor(() => {
				expect(attachResolvers).toHaveLength(1);
			});
			attachResolvers[0]();
			mockChannels[0].onmessage({
				type: "snapshot",
				surface: {
					session_key: "test-uuid-1234",
					terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
					is_exited: false,
					exit_code: null,
				},
			});

			await waitFor(() => {
				const writes = mockInvoke.mock.calls.filter(
					([command]) => command === "write_terminal_surface",
				);
				expect(writes).toHaveLength(1);
				expect(writes[0][1]).toMatchObject({
					data: "a".repeat(1024),
					sequence: 0,
				});
			});
			expect(mockInvoke).not.toHaveBeenCalledWith(
				"write_terminal_surface",
				expect.objectContaining({ data: "x" }),
			);
			warnSpy.mockRestore();
		});

		it("exited snapshotではbuffer済み入力を送出せず破棄する", async () => {
			const attachResolvers: Array<() => void> = [];
			const baseImplementation = mockInvoke.getMockImplementation();
			mockInvoke.mockImplementation(
				(cmd: string, args?: Record<string, unknown>) => {
					if (cmd === "attach_terminal_surface") {
						return new Promise<void>((resolve) => {
							attachResolvers.push(() => resolve());
						});
					}
					return baseImplementation?.(cmd, args);
				},
			);

			const onTerminalError = vi.fn();
			const onTerminalReady = vi.fn();
			renderHook(() =>
				useTerminal(containerRef, { onTerminalError, onTerminalReady }),
			);
			await waitFor(() => {
				expect(mockChannels).toHaveLength(1);
			});

			mockOnDataCallback("abc");
			await waitFor(() => {
				expect(attachResolvers).toHaveLength(1);
			});
			attachResolvers[0]();
			mockChannels[0].onmessage({
				type: "snapshot",
				surface: {
					session_key: "test-uuid-1234",
					terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
					is_exited: true,
					exit_code: 1,
				},
			});

			await waitFor(() => {
				expect(mockTerminalInstance.write).toHaveBeenCalledWith(
					"\r\n\x1b[90m[Process exited with code 1]\x1b[0m\r\n",
					expect.any(Function),
				);
			});
			expect(mockInvoke).not.toHaveBeenCalledWith(
				"write_terminal_surface",
				expect.anything(),
			);
			expect(onTerminalError).toHaveBeenCalledWith(null);
			expect(onTerminalReady).not.toHaveBeenCalled();
		});
	});
});
