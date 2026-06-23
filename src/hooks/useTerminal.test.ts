import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	createBoundedPtyOutputQueue,
	MAX_INITIAL_REFETCH,
	QUEUED_INITIAL_OUTPUT_MAX_BYTES,
	QUEUED_INITIAL_OUTPUT_MAX_ITEMS,
	useTerminal,
} from "./useTerminal";

const mockInvoke = vi.fn();
const mockListen = vi.fn();
let mockOnDataCallback: (data: string) => void = () => {};
let mockTerminalInstance: {
	loadAddon: ReturnType<typeof vi.fn>;
	open: ReturnType<typeof vi.fn>;
	write: ReturnType<typeof vi.fn>;
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
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@xterm/xterm", () => {
	return {
		Terminal: class MockTerminal {
			loadAddon = vi.fn();
			open = vi.fn();
			write = vi.fn();
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
	let mockUnlistenOutput: ReturnType<typeof vi.fn>;
	let mockUnlistenExit: ReturnType<typeof vi.fn>;
	let mockUnlistenEvicted: ReturnType<typeof vi.fn>;

	beforeEach(() => {
		vi.clearAllMocks();
		mockInvoke.mockReset();
		mockListen.mockReset();

		containerRef = { current: document.createElement("div") };

		mockUnlistenOutput = vi.fn();
		mockUnlistenExit = vi.fn();
		mockUnlistenEvicted = vi.fn();

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.resolve({
					pty_id: 1,
					session_key: "test-uuid-1234",
					buffered_output: "",
					buffered_output_sequence: 0,
					is_new: true,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});
		mockListen
			.mockResolvedValueOnce(mockUnlistenOutput)
			.mockResolvedValueOnce(mockUnlistenExit)
			.mockResolvedValueOnce(mockUnlistenEvicted);
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
				sessionKey: null,
				worktreePath: "",
				label: null,
				kind: "terminal",
			});
		});
	});

	it("PTY event listeners are registered", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledWith(
				"pty-output",
				expect.any(Function),
			);
			expect(mockListen).toHaveBeenCalledWith("pty-exit", expect.any(Function));
			expect(mockListen).toHaveBeenCalledWith(
				"pty-evicted",
				expect.any(Function),
			);
		});
	});

	it("PTY ready state is synced as active while mounted and cleared on unmount", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef, "/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("register_active_terminal", {
				worktreePath: "/repo",
				sessionKey: "test-uuid-1234",
				activeToken: expect.any(String),
			});
		});
		const registerCall = mockInvoke.mock.calls.find(
			(call: unknown[]) => call[0] === "register_active_terminal",
		) as [string, { activeToken: string }] | undefined;
		const activeToken = registerCall?.[1].activeToken;

		unmount();

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("unregister_active_terminal", {
				worktreePath: "/repo",
				sessionKey: "test-uuid-1234",
				activeToken,
			});
		});
	});

	it("uses a new active token after remounting the same session", async () => {
		mockListen.mockImplementation(() => Promise.resolve(vi.fn()));
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.resolve({
					pty_id: 1,
					session_key: "same-session",
					buffered_output: "",
					buffered_output_sequence: 0,
					is_new: false,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});

		const first = renderHook(() =>
			useTerminal(containerRef, "/repo", undefined, undefined, "same-session"),
		);
		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("register_active_terminal", {
				worktreePath: "/repo",
				sessionKey: "same-session",
				activeToken: expect.any(String),
			});
		});
		const firstRegister = mockInvoke.mock.calls.find(
			(call: unknown[]) => call[0] === "register_active_terminal",
		) as [string, { activeToken: string }] | undefined;
		const firstToken = firstRegister?.[1].activeToken;

		first.unmount();
		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("unregister_active_terminal", {
				worktreePath: "/repo",
				sessionKey: "same-session",
				activeToken: firstToken,
			});
		});

		mockInvoke.mockClear();
		const secondContainerRef = { current: document.createElement("div") };
		renderHook(() =>
			useTerminal(
				secondContainerRef,
				"/repo",
				undefined,
				undefined,
				"same-session",
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("register_active_terminal", {
				worktreePath: "/repo",
				sessionKey: "same-session",
				activeToken: expect.any(String),
			});
		});
		const secondRegister = mockInvoke.mock.calls.find(
			(call: unknown[]) => call[0] === "register_active_terminal",
		) as [string, { activeToken: string }] | undefined;
		expect(secondRegister?.[1].activeToken).not.toBe(firstToken);
	});

	it("get_or_spawn resolves after unmount still reports the session key for managed panes", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
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
		const onPtyReady = vi.fn();

		const { unmount } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				"repo terminal",
				onPtyReady,
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
			pty_id: 7,
			session_key: "late-session",
			buffered_output: "",
			buffered_output_sequence: 0,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(onPtyReady).toHaveBeenCalledWith(7, "late-session");
		});
		expect(mockInvoke).toHaveBeenCalledWith("unregister_active_terminal", {
			worktreePath: "/repo",
			sessionKey: "late-session",
			activeToken: expect.any(String),
		});
		expect(mockInvoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
	});

	it("requestKill() 後に get_or_spawn が解決した pending PTY は onPtyReady を呼ばない", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
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
		const onPtyReady = vi.fn();

		const { result, unmount } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				undefined,
				"repo terminal",
				onPtyReady,
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
			pty_id: 7,
			session_key: "late-session",
			buffered_output: "",
			buffered_output_sequence: 0,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("unregister_active_terminal", {
				worktreePath: "/repo",
				sessionKey: "late-session",
				activeToken: expect.any(String),
			});
		});
		expect(onPtyReady).not.toHaveBeenCalled();
	});

	it("requestKill() 後に get_or_spawn が解決した pending PTY を kill する", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
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
			pty_id: 7,
			session_key: "late-session",
			buffered_output: "",
			buffered_output_sequence: 0,
			is_new: true,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("kill_pty", { ptyId: 7 });
		});
		expect(mockInvoke).toHaveBeenCalledWith("unregister_active_terminal", {
			worktreePath: "/repo",
			sessionKey: "late-session",
			activeToken: expect.any(String),
		});
	});

	it("PTY initialization failures are displayed and reported to the caller", async () => {
		const onPtyError = vi.fn();
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
				onPtyError,
			),
		);

		await waitFor(() => {
			expect(onPtyError).toHaveBeenCalledWith(
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

	it("PTY cap-looking text without a stable code is treated as a generic init failure", async () => {
		const onPtyError = vi.fn();
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
				onPtyError,
			),
		);

		await waitFor(() => {
			expect(onPtyError).toHaveBeenCalledWith(
				"Failed to initialize terminal: PTY cap reached for worktree /repo",
			);
		});
	});

	it("initial output queue is bounded by item count and bytes", () => {
		const queue = createBoundedPtyOutputQueue();
		const payload = "x".repeat(1024);

		for (
			let sequence = 1;
			sequence <= QUEUED_INITIAL_OUTPUT_MAX_ITEMS * 4;
			sequence += 1
		) {
			queue.enqueue({ pty_id: 1, data: payload, sequence });
		}

		expect(queue.size()).toBeLessThanOrEqual(QUEUED_INITIAL_OUTPUT_MAX_ITEMS);
		expect(queue.bytes()).toBeLessThanOrEqual(QUEUED_INITIAL_OUTPUT_MAX_BYTES);
		expect(queue.hasDropped()).toBe(true);
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
			ptyId: 1,
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

		expect(mockUnlistenOutput).toHaveBeenCalled();
		expect(mockUnlistenExit).toHaveBeenCalled();
		expect(mockUnlistenEvicted).toHaveBeenCalled();
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

		expect(mockInvoke).toHaveBeenCalledWith("kill_pty", { ptyId: 1 });
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("pty-exit 後のアンマウントでは kill_pty が呼ばれない", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		// pty-exit イベントをシミュレート（ptyIdRef.current が null になる）
		const exitListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-exit",
		)?.[1] as (event: {
			payload: { pty_id: number; exit_code: number | null };
		}) => void;
		exitListener({ payload: { pty_id: 1, exit_code: 0 } });

		unmount();

		expect(mockInvoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
	});

	it("pty-evicted for current PTY disables later writes", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});

		const evictedListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-evicted",
		)?.[1] as (event: {
			payload: { pty_id: number; session_key: string; reason: string };
		}) => void;
		evictedListener({
			payload: {
				pty_id: 1,
				session_key: "test-uuid-1234",
				reason: "idle",
			},
		});

		mockInvoke.mockClear();
		mockOnDataCallback("after eviction");

		expect(mockTerminalInstance.write).toHaveBeenCalledWith(
			"\r\n\x1b[90m[Terminal evicted]\x1b[0m\r\n",
		);
		expect(mockInvoke).not.toHaveBeenCalledWith("write_pty", expect.anything());
	});

	it("containerRef が null の場合は初期化されない", () => {
		const nullContainerRef = { current: null };
		const previousInstance = mockTerminalInstance;

		renderHook(() => useTerminal(nullContainerRef));

		expect(mockTerminalInstance).toBe(previousInstance);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("既存セッションのbuffered_outputがターミナルに書き込まれる", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.resolve({
					pty_id: 1,
					session_key: "pre-spawned-key",
					buffered_output: "previously buffered text\r\n$ ",
					buffered_output_sequence: 3,
					is_new: false,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});

		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"previously buffered text\r\n$ ",
			);
		});
	});

	it("初期復元中に届いたlive outputをbuffered_output後に書き込む", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
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

		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});
		const outputListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-output",
		)?.[1] as (event: {
			payload: { pty_id: number; data: string; sequence: number };
		}) => void;
		outputListener({
			payload: { pty_id: 7, data: "live during init", sequence: 4 },
		});

		resolveSpawn({
			pty_id: 7,
			session_key: "pre-spawned-key",
			buffered_output: "previously buffered text\r\n$ ",
			buffered_output_sequence: 3,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"live during init",
			);
		});
		expect(mockTerminalInstance.write.mock.calls.map(([data]) => data)).toEqual(
			["previously buffered text\r\n$ ", "live during init"],
		);
	});

	it("初期復元中に届いたsnapshot済みoutputは重複して書き込まない", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
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

		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});
		const outputListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-output",
		)?.[1] as (event: {
			payload: { pty_id: number; data: string; sequence: number };
		}) => void;
		outputListener({
			payload: { pty_id: 7, data: "already snapshotted", sequence: 3 },
		});

		resolveSpawn({
			pty_id: 7,
			session_key: "pre-spawned-key",
			buffered_output: "previously buffered text\r\n$ already snapshotted",
			buffered_output_sequence: 3,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith(
				"previously buffered text\r\n$ already snapshotted",
			);
		});
		expect(mockTerminalInstance.write.mock.calls.map(([data]) => data)).toEqual(
			["previously buffered text\r\n$ already snapshotted"],
		);
	});

	it("初期復元queueでdropが起きた場合は最新buffered_outputを再取得して欠落を補う", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
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
			if (cmd === "get_pty_buffered_output") {
				return Promise.resolve({
					pty_id: 7,
					session_key: "pre-spawned-key",
					buffered_output: "backend replay through 256",
					buffered_output_sequence: 256,
					is_exited: false,
					exit_code: null,
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
				"pre-spawned-key",
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});
		const outputListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-output",
		)?.[1] as (event: {
			payload: { pty_id: number; data: string; sequence: number };
		}) => void;

		for (
			let sequence = 1;
			sequence <= QUEUED_INITIAL_OUTPUT_MAX_ITEMS + 4;
			sequence += 1
		) {
			outputListener({
				payload: { pty_id: 7, data: `chunk-${sequence}`, sequence },
			});
		}

		resolveSpawn({
			pty_id: 7,
			session_key: "pre-spawned-key",
			buffered_output: "stale backend replay",
			buffered_output_sequence: 0,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_pty_buffered_output", {
				sessionKey: "pre-spawned-key",
				worktreePath: "/repo",
			});
		});
		await waitFor(() => {
			expect(mockTerminalInstance.write).toHaveBeenCalledWith("chunk-260");
		});

		expect(mockTerminalInstance.write.mock.calls.map(([data]) => data)).toEqual(
			[
				"backend replay through 256",
				"chunk-257",
				"chunk-258",
				"chunk-259",
				"chunk-260",
			],
		);
	});

	it("初期復元queueのdropが続いても最大refetch回数で終了して入力を受け付ける", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		const oversizedOutput = "x".repeat(QUEUED_INITIAL_OUTPUT_MAX_BYTES + 1);
		let outputListener: (event: {
			payload: { pty_id: number; data: string; sequence: number };
		}) => void = () => {};
		let refetchCount = 0;
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") return pendingSpawn;
			if (cmd === "get_pty_buffered_output") {
				refetchCount += 1;
				outputListener({
					payload: {
						pty_id: 7,
						data: oversizedOutput,
						sequence: 1000 + refetchCount,
					},
				});
				return Promise.resolve({
					pty_id: 7,
					session_key: "pre-spawned-key",
					buffered_output: `backend replay ${refetchCount}`,
					buffered_output_sequence: 1000 + refetchCount,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});

		const { result } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				"pre-spawned-key",
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});
		outputListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-output",
		)?.[1] as (event: {
			payload: { pty_id: number; data: string; sequence: number };
		}) => void;
		outputListener({
			payload: { pty_id: 7, data: oversizedOutput, sequence: 1 },
		});

		resolveSpawn({
			pty_id: 7,
			session_key: "pre-spawned-key",
			buffered_output: "stale backend replay",
			buffered_output_sequence: 0,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(result.current.ptyIdRef.current).toBe(7);
		});

		const refetchCalls = mockInvoke.mock.calls.filter(
			(call) => call[0] === "get_pty_buffered_output",
		);
		expect(refetchCalls).toHaveLength(MAX_INITIAL_REFETCH);
		expect(mockTerminalInstance.write).toHaveBeenCalledWith(
			`backend replay ${MAX_INITIAL_REFETCH}`,
		);

		mockOnDataCallback("after init");

		expect(mockInvoke).toHaveBeenCalledWith("write_pty", {
			ptyId: 7,
			data: "after init",
		});
	});

	it("初期復元queueのdropが解消したら最大refetch回数未満で終了する", async () => {
		type SpawnResult = {
			pty_id: number;
			session_key: string;
			buffered_output: string;
			buffered_output_sequence: number;
			is_new: boolean;
			is_exited: boolean;
			exit_code: number | null;
		};
		let resolveSpawn!: (value: SpawnResult) => void;
		const pendingSpawn = new Promise<SpawnResult>((resolve) => {
			resolveSpawn = resolve;
		});
		const oversizedOutput = "x".repeat(QUEUED_INITIAL_OUTPUT_MAX_BYTES + 1);
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") return pendingSpawn;
			if (cmd === "get_pty_buffered_output") {
				return Promise.resolve({
					pty_id: 7,
					session_key: "pre-spawned-key",
					buffered_output: "backend replay after drop",
					buffered_output_sequence: 256,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});

		const { result } = renderHook(() =>
			useTerminal(
				containerRef,
				"/repo",
				undefined,
				undefined,
				"pre-spawned-key",
			),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_or_spawn_pty",
				expect.any(Object),
			);
		});
		const outputListener = mockListen.mock.calls.find(
			(call: unknown[]) => call[0] === "pty-output",
		)?.[1] as (event: {
			payload: { pty_id: number; data: string; sequence: number };
		}) => void;
		outputListener({
			payload: { pty_id: 7, data: oversizedOutput, sequence: 1 },
		});

		resolveSpawn({
			pty_id: 7,
			session_key: "pre-spawned-key",
			buffered_output: "stale backend replay",
			buffered_output_sequence: 0,
			is_new: false,
			is_exited: false,
			exit_code: null,
		});

		await waitFor(() => {
			expect(result.current.ptyIdRef.current).toBe(7);
		});

		const refetchCalls = mockInvoke.mock.calls.filter(
			(call) => call[0] === "get_pty_buffered_output",
		);
		expect(refetchCalls.length).toBeLessThan(MAX_INITIAL_REFETCH);
		expect(refetchCalls).toHaveLength(1);
		expect(mockTerminalInstance.write).toHaveBeenCalledWith(
			"backend replay after drop",
		);
	});

	it("新規セッション（is_new: true）のとき起動コマンドが送信される", async () => {
		renderHook(() => useTerminal(containerRef, null, undefined, "startup-cmd"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("write_pty", {
				ptyId: 1,
				data: "startup-cmd\n",
			});
		});
	});

	it("既存セッション（is_new: false）のとき起動コマンドが送信されない", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.resolve({
					pty_id: 1,
					session_key: "test-uuid-existing",
					buffered_output: "",
					buffered_output_sequence: 0,
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
					ptyId: 1,
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
				ptyId: 1,
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
					ptyId: 1,
					rows: 24,
					cols: 80,
				});
			});
		});
	});
});
