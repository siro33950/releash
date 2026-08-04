import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTerminal } from "./useTerminal";

const REPO_WORKSPACE_OWNER = {
	kind: "workspace",
	workspacePath: "/repo",
} as const;

const mockInvoke = vi.fn();
const mockListen = vi.fn();
const mockChannels: Array<{ onmessage: (message: unknown) => void }> = [];
let mockOnDataCallback: (data: string) => void = () => {};
let mockTerminalInstance: {
	loadAddon: ReturnType<typeof vi.fn>;
	open: ReturnType<typeof vi.fn>;
	write: ReturnType<typeof vi.fn>;
	resize: ReturnType<typeof vi.fn>;
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
			mockChannels.push(this);
		}
	},
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@xterm/xterm", () => {
	return {
		Terminal: class MockTerminal {
			loadAddon = vi.fn();
			open = vi.fn();
			write = vi.fn((_data: string, callback?: () => void) => callback?.());
			resize = vi.fn((cols: number, rows: number) => {
				this.cols = cols;
				this.rows = rows;
			});
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
			rows = 24;
			cols = 80;

			constructor() {
				mockTerminalInstance = this;
			}
		},
	};
});

let mockFitAddonInstance: { fit: ReturnType<typeof vi.fn> };

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
		mockChannels.length = 0;

		containerRef = { current: document.createElement("div") };

		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_pty") {
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
				if (cmd === "attach_pty") {
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

	it("get_or_spawn_pty が正しい引数で呼び出される", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_or_spawn_pty", {
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
			expect(mockInvoke).toHaveBeenCalledWith("attach_pty", {
				owner: { kind: "workspace", workspacePath: "" },
				attachmentId: expect.any(String),
				onEvent: mockChannels[0],
			});
		});
		expect(mockChannels).toHaveLength(1);
		expect(mockListen).not.toHaveBeenCalled();
	});

	it("unmountはstream購読だけを解除しPTY lifecycleを変更しない", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef, "/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		unmount();
		expect(mockInvoke).toHaveBeenCalledWith("detach_pty", {
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
				if (cmd === "get_or_spawn_pty") {
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
				if (cmd === "attach_pty") {
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
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				REPO_WORKSPACE_OWNER,
			),
		);
		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.objectContaining({ owner: REPO_WORKSPACE_OWNER }),
			);
		});

		first.unmount();

		mockInvoke.mockClear();
		const secondContainerRef = { current: document.createElement("div") };
		renderHook(() =>
			useTerminal(
				secondContainerRef,
				"/repo",
				undefined,
				undefined,
				REPO_WORKSPACE_OWNER,
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
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
			if (cmd === "get_or_spawn_pty") return pendingSpawn;
			return Promise.resolve();
		});
		const onTerminalReady = vi.fn();

		const { unmount } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				"repo terminal",
				onTerminalReady,
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
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
		expect(mockInvoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
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
			if (cmd === "get_or_spawn_pty") return pendingSpawn;
			return Promise.resolve();
		});
		const onTerminalReady = vi.fn();
		const shouldKillPendingTerminal = vi.fn(() => false);

		const { unmount } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				"repo terminal",
				onTerminalReady,
				undefined,
				shouldKillPendingTerminal,
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
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
			expect(mockInvoke).toHaveBeenCalledWith("kill_pty", {
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
			if (cmd === "get_or_spawn_pty") return pendingSpawn;
			return Promise.resolve();
		});
		const onTerminalReady = vi.fn();

		const { result, unmount } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				"repo terminal",
				onTerminalReady,
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
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
			if (cmd === "get_or_spawn_pty") return pendingSpawn;
			return Promise.resolve();
		});

		const { result, unmount } = renderHook(() =>
			useTerminal(containerRef, "/repo"),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
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
			expect(mockInvoke).toHaveBeenCalledWith("kill_pty", {
				owner: REPO_WORKSPACE_OWNER,
			});
		});
	});

	it("PTY初期化失敗を表示して呼び出し元へ通知する", async () => {
		const onTerminalError = vi.fn();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.reject({
					code: "CAP_REACHED",
					message: "PTY limit unavailable for worktree /repo",
				});
			}
			return Promise.resolve();
		});

		renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				undefined,
				undefined,
				onTerminalError,
			),
		);

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Terminal limit reached: PTY limit unavailable for worktree /repo",
			);
		});
		expect(mockTerminalInstance.write).toHaveBeenCalledWith(
			"\r\n\x1b[31mTerminal limit reached: PTY limit unavailable for worktree /repo\x1b[0m\r\n",
		);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"register_active_terminal",
			expect.objectContaining({ sessionKey: "test-uuid-1234" }),
		);
	});

	it("安定codeのない上限風messageは一般初期化失敗として扱う", async () => {
		const onTerminalError = vi.fn();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.reject("PTY cap reached for worktree /repo");
			}
			return Promise.resolve();
		});

		renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				undefined,
				undefined,
				onTerminalError,
			),
		);

		await waitFor(() => {
			expect(onTerminalError).toHaveBeenCalledWith(
				"Failed to initialize terminal: PTY cap reached for worktree /repo",
			);
		});
	});

	it("ユーザー入力時に write_pty が呼び出される", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		mockOnDataCallback("test input");

		expect(mockInvoke).toHaveBeenCalledWith("write_pty", {
			owner: { kind: "workspace", workspacePath: "" },
			data: "test input",
		});
	});

	it("アンマウント時にデフォルトでは kill_pty が呼ばれない（PTY保持）", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		unmount();

		expect(mockInvoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("requestKill() 後のアンマウントで kill_pty が呼ばれる", async () => {
		const { result, unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		result.current.requestKill();
		unmount();

		expect(mockInvoke).toHaveBeenCalledWith("kill_pty", {
			owner: { kind: "workspace", workspacePath: "" },
		});
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("pty-exit 後のアンマウントでは kill_pty が呼ばれない", async () => {
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

		expect(mockInvoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
	});

	it("backendで検証済みのresizeとexitを受信順に投影する", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
			expect(mockTerminalInstance.resize).toHaveBeenCalledWith(80, 24);
		});
		mockTerminalInstance.resize.mockClear();
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
				if (command === "get_or_spawn_pty") return pendingSpawn;
				if (command === "attach_pty") {
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

		const { result } = renderHook(() => useTerminal(containerRef, "/repo"));
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
			expect(result.current.sessionKeyRef.current).toBe("late-exit");
			expect(result.current.isRunningRef.current).toBe(false);
		});
		expect(mockTerminalInstance.write).toHaveBeenCalledWith(
			"\r\n\x1b[90m[Process exited with code 23]\x1b[0m\r\n",
			expect.any(Function),
		);
		mockInvoke.mockClear();
		mockOnDataCallback("must not be written");
		expect(mockInvoke).not.toHaveBeenCalledWith("write_pty", expect.anything());
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
				if (cmd === "get_or_spawn_pty") {
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
				if (cmd === "attach_pty") {
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
				if (cmd === "get_or_spawn_pty") {
					return Promise.resolve({
						session_key: "pre-spawned-key",
						terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
						restored_from_checkpoint: false,
						is_new: false,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_pty") {
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

	it("live outputのxterm反映完了後にだけ後続resizeを投影する", async () => {
		let completeLiveWrite: (() => void) | undefined;
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockChannels).toHaveLength(1);
			expect(mockTerminalInstance.resize).toHaveBeenCalledWith(80, 24);
		});
		mockTerminalInstance.resize.mockClear();
		mockTerminalInstance.write.mockImplementation(
			(data: string, callback?: () => void) => {
				if (data === "ordered live output") {
					completeLiveWrite = callback;
					return;
				}
				callback?.();
			},
		);

		mockChannels[0].onmessage({
			type: "output",
			session_key: "test-uuid-1234",
			data: "ordered live output",
			sequence: 1,
		});
		mockChannels[0].onmessage({
			type: "resize",
			session_key: "test-uuid-1234",
			cols: 111,
			rows: 37,
			sequence: 2,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"ordered live output",
				expect.any(Function),
			);
		});
		expect(mockTerminalInstance.resize).not.toHaveBeenCalled();

		completeLiveWrite?.();
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
				if (cmd === "get_or_spawn_pty") {
					return pendingSpawn;
				}
				if (cmd === "attach_pty") {
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

		const { result } = renderHook(() => useTerminal(containerRef));
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
		expect(result.current.sessionKeyRef.current).toBe("terminal-surface");
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
			expect(result.current.sessionKeyRef.current).toBe("terminal-surface");
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
				if (cmd === "get_or_spawn_pty") {
					return Promise.resolve({
						session_key: "pre-spawned-key",
						terminal_surface: { replay: "", sequence: 0, cols: 80, rows: 24 },
						restored_from_checkpoint: false,
						is_new: false,
						is_exited: false,
						exit_code: null,
					});
				}
				if (cmd === "attach_pty") {
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
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				REPO_WORKSPACE_OWNER,
			),
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
		renderHook(() => useTerminal(containerRef, null, undefined, "startup-cmd"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.objectContaining({ startupCommand: "startup-cmd" }),
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_pty",
			expect.objectContaining({ data: "startup-cmd\n" }),
		);
	});

	it("cold restoreでは新しいPTYでも起動コマンドを自動再実行しない", async () => {
		mockInvoke.mockImplementation(
			(cmd: string, args?: Record<string, unknown>) => {
				if (cmd === "get_or_spawn_pty") {
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
				if (cmd === "attach_pty") {
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

		renderHook(() => useTerminal(containerRef, null, undefined, "startup-cmd"));

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"restored screen",
				expect.any(Function),
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_pty",
			expect.objectContaining({ data: "startup-cmd\n" }),
		);
	});

	it("既存セッション（is_new: false）のとき起動コマンドが送信されない", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
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

		renderHook(() => useTerminal(containerRef, null, undefined, "startup-cmd"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		expect(mockInvoke).not.toHaveBeenCalledWith(
			"write_pty",
			expect.objectContaining({ data: "startup-cmd\n" }),
		);
	});

	describe("attachCustomKeyEventHandler", () => {
		it("ペイン操作キーを抑止する", () => {
			renderHook(() => useTerminal(containerRef));

			expect(
				mockTerminalInstance.attachCustomKeyEventHandler,
			).toHaveBeenCalledTimes(1);
			const handler = mockTerminalInstance.attachCustomKeyEventHandler.mock
				.calls[0][0] as (event: Partial<KeyboardEvent>) => boolean;

			// Cmd+D → false (垂直分割)
			expect(
				handler({ metaKey: true, ctrlKey: false, altKey: false, key: "d" }),
			).toBe(false);
			// Cmd+Shift+D → false (水平分割)
			expect(
				handler({ metaKey: true, ctrlKey: false, altKey: false, key: "D" }),
			).toBe(false);
			// Cmd+Option+ArrowRight → false (フォーカス移動)
			expect(
				handler({
					metaKey: true,
					ctrlKey: false,
					altKey: true,
					key: "ArrowRight",
				}),
			).toBe(false);
			// Cmd+Option+ArrowLeft → false
			expect(
				handler({
					metaKey: true,
					ctrlKey: false,
					altKey: true,
					key: "ArrowLeft",
				}),
			).toBe(false);
		});

		it("通常キーは抑止しない", () => {
			renderHook(() => useTerminal(containerRef));

			const handler = mockTerminalInstance.attachCustomKeyEventHandler.mock
				.calls[0][0] as (event: Partial<KeyboardEvent>) => boolean;

			// 通常の文字入力 → true
			expect(
				handler({ metaKey: false, ctrlKey: false, altKey: false, key: "a" }),
			).toBe(true);
			// Cmd+C (コピー) → true
			expect(
				handler({ metaKey: true, ctrlKey: false, altKey: false, key: "c" }),
			).toBe(true);
			// 矢印キー（修飾なし） → true
			expect(
				handler({
					metaKey: false,
					ctrlKey: false,
					altKey: false,
					key: "ArrowRight",
				}),
			).toBe(true);
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

		it("ゼロサイズ時に resize_pty が呼ばれない", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_pty",
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
				"resize_pty",
				expect.any(Object),
			);
		});
	});

	describe("リサイズデバウンス", () => {
		it("連続リサイズ時に resize_pty がデバウンスされ1回だけ呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_pty",
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
				"resize_pty",
				expect.any(Object),
			);

			// デバウンス後に1回だけ呼ばれることを検証
			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("resize_pty", {
					owner: { kind: "workspace", workspacePath: "" },
					rows: 24,
					cols: 80,
				});
			});

			const resizeCalls = mockInvoke.mock.calls.filter(
				(call) => call[0] === "resize_pty",
			);
			expect(resizeCalls).toHaveLength(1);
		});

		it("非表示復帰時はデバウンスなしで即座に resize_pty が呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_pty",
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
			expect(mockInvoke).toHaveBeenCalledWith("resize_pty", {
				owner: { kind: "workspace", workspacePath: "" },
				rows: 24,
				cols: 80,
			});
		});

		it("デバウンス保留中にアンマウントしてもエラーが発生しない", async () => {
			const { unmount } = renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_pty",
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
				"resize_pty",
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
		it("get_or_spawn_pty の後に resize_pty が requestAnimationFrame で呼ばれる", async () => {
			renderHook(() => useTerminal(containerRef));

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"get_or_spawn_pty",
					expect.any(Object),
				);
			});

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("resize_pty", {
					owner: { kind: "workspace", workspacePath: "" },
					rows: 24,
					cols: 80,
				});
			});
		});
	});
});
