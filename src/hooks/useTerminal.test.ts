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

	beforeEach(() => {
		vi.clearAllMocks();

		containerRef = { current: document.createElement("div") };

		mockUnlistenOutput = vi.fn();
		mockUnlistenExit = vi.fn();

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_or_spawn_pty") {
				return Promise.resolve({
					pty_id: 1,
					session_key: "test-uuid-1234",
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
				sessionKey: null,
				worktreePath: "",
				label: null,
				kind: "terminal",
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
