import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTerminal } from "./useTerminal";

const mockInvoke = vi.fn();
const mockListen = vi.fn();
let mockOnDataCallback: (data: string) => void = () => {};
let mockTerminalInstance: {
	loadAddon: ReturnType<typeof vi.fn>;
	open: ReturnType<typeof vi.fn>;
	write: ReturnType<typeof vi.fn>;
	onData: ReturnType<typeof vi.fn>;
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
let resizeObserverDisconnect: ReturnType<typeof vi.fn>;

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

	beforeEach(() => {
		vi.clearAllMocks();

		containerRef = { current: document.createElement("div") };

		mockUnlistenOutput = vi.fn();
		mockUnlistenExit = vi.fn();

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.resolve({
					pty_id: 1,
					buffered_output: "",
					is_new: true,
					is_exited: false,
					exit_code: null,
				});
			}
			return Promise.resolve();
		});
		mockListen
			.mockResolvedValueOnce(mockUnlistenOutput)
			.mockResolvedValueOnce(mockUnlistenExit);
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
				worktreePath: "",
			});
		});
	});

	it("pty-output と pty-exit のリスナーが登録される", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledWith(
				"pty-output",
				expect.any(Function),
			);
			expect(mockListen).toHaveBeenCalledWith("pty-exit", expect.any(Function));
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
			ptyId: 1,
			data: "test input",
		});
	});

	it("アンマウント時にクリーンアップが実行される（kill_pty は呼ばれない）", async () => {
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
		expect(mockInvoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("containerRef が null の場合は初期化されない", () => {
		const nullContainerRef = { current: null };
		const previousInstance = mockTerminalInstance;

		renderHook(() => useTerminal(nullContainerRef));

		expect(mockTerminalInstance).toBe(previousInstance);
		expect(mockInvoke).not.toHaveBeenCalled();
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
});
