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
			options: Record<string, unknown> = {};
			rows = 24;
			cols = 80;

			constructor() {
				mockTerminalInstance = this;
			}
		},
	};
});

vi.mock("@xterm/addon-fit", () => {
	return {
		FitAddon: class MockFitAddon {
			fit = vi.fn();
		},
	};
});

describe("useTerminal", () => {
	let containerRef: { current: HTMLDivElement | null };
	let mockUnlistenOutput: ReturnType<typeof vi.fn>;
	let mockUnlistenExit: ReturnType<typeof vi.fn>;

	beforeEach(() => {
		vi.clearAllMocks();

		containerRef = { current: document.createElement("div") };

		mockUnlistenOutput = vi.fn();
		mockUnlistenExit = vi.fn();

		mockInvoke.mockResolvedValue(1);
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

	it("spawn_pty が正しい引数で呼び出される", async () => {
		renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("spawn_pty", {
				rows: 24,
				cols: 80,
				cwd: null,
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
			expect(mockInvoke).toHaveBeenCalledWith("spawn_pty", expect.any(Object));
		});

		mockOnDataCallback("test input");

		expect(mockInvoke).toHaveBeenCalledWith("write_pty", {
			ptyId: 1,
			data: "test input",
		});
	});

	it("アンマウント時にクリーンアップが実行される", async () => {
		const { unmount } = renderHook(() => useTerminal(containerRef));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("spawn_pty", expect.any(Object));
		});

		unmount();

		expect(mockUnlistenOutput).toHaveBeenCalled();
		expect(mockUnlistenExit).toHaveBeenCalled();
		expect(mockInvoke).toHaveBeenCalledWith("kill_pty", { ptyId: 1 });
		expect(mockTerminalInstance.dispose).toHaveBeenCalled();
	});

	it("containerRef が null の場合は初期化されない", () => {
		const nullContainerRef = { current: null };
		const previousInstance = mockTerminalInstance;

		renderHook(() => useTerminal(nullContainerRef));

		expect(mockTerminalInstance).toBe(previousInstance);
		expect(mockInvoke).not.toHaveBeenCalled();
	});
});
