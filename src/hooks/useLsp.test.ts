import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TauriTransport } from "../lib/lsp/tauri-transport";
import { useLsp } from "./useLsp";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

type ListenCallback = (event: { payload: Record<string, unknown> }) => void;
let capturedListeners: Map<string, ListenCallback>;

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn((eventName: string, callback: ListenCallback) => {
		capturedListeners.set(eventName, callback);
		return Promise.resolve(() => {
			capturedListeners.delete(eventName);
		});
	}),
}));

let fakeSessionIdCounter = 0;

function createFakeTransport(): TauriTransport {
	fakeSessionIdCounter += 1;
	const sessionId = fakeSessionIdCounter;
	return {
		sessionId,
		dispose: vi.fn(),
		setOpen: vi.fn(),
		setClosed: vi.fn(),
		handleMessage: vi.fn(),
		send: vi.fn(),
		setListener: vi.fn(),
		setWorktreePath: vi.fn(),
		toString: () => `FakeTransport(session=${sessionId})`,
		state: {
			get value() {
				return { state: "open" as const };
			},
			get onChange() {
				return () => ({ dispose: () => {} });
			},
		},
	} as unknown as TauriTransport;
}

const mockCreateTauriTransport = vi.fn();
vi.mock("../lib/lsp/tauri-transport", () => ({
	createTauriTransport: (...args: unknown[]) =>
		mockCreateTauriTransport(...args),
}));

const defaultConfig = {
	command: "/usr/bin/jdtls",
	args: ["--stdio"],
	enabled: true,
};

function fireLspError(sessionId: number, error: string) {
	const listener = capturedListeners.get("lsp-error");
	if (listener) {
		listener({ payload: { session_id: sessionId, error } });
	}
}

describe("useLsp", () => {
	beforeEach(() => {
		vi.restoreAllMocks();
		mockInvoke.mockReset();
		mockCreateTauriTransport.mockReset();
		capturedListeners = new Map();
		fakeSessionIdCounter = 0;
	});

	it("detect_lsp_server が設定を返す → running に遷移", async () => {
		const fakeTransport = createFakeTransport();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(defaultConfig);
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockResolvedValue(fakeTransport);

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(result.current.sessionId).toBe(fakeTransport.sessionId);
		expect(mockCreateTauriTransport).toHaveBeenCalledWith(
			"/workspace",
			"java",
			defaultConfig.command,
			defaultConfig.args,
		);
	});

	it("detect_lsp_server が enabled=false を返す → idle のまま（自動インストールしない）", async () => {
		const disabledConfig = {
			command: "",
			args: [],
			enabled: false,
		};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(disabledConfig);
			return Promise.resolve(null);
		});

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		// Wait for async operations to settle
		await vi.waitFor(() => {
			expect(result.current.status).toBe("idle");
		});

		expect(result.current.sessionId).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"install_lsp_server",
			expect.anything(),
		);
		expect(mockCreateTauriTransport).not.toHaveBeenCalled();
	});

	it("サポート言語でサーバー未検出 → 自動ダウンロード → running に遷移", async () => {
		const fakeTransport = createFakeTransport();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(null);
			if (cmd === "get_supported_lsp_languages")
				return Promise.resolve(["java"]);
			if (cmd === "install_lsp_server") return Promise.resolve(defaultConfig);
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockResolvedValue(fakeTransport);

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(mockInvoke).toHaveBeenCalledWith("install_lsp_server", {
			language: "java",
		});
		expect(result.current.sessionId).toBe(fakeTransport.sessionId);
	});

	it("サポート言語でサーバー未検出 → downloading 中間状態を経由して running に遷移", async () => {
		const fakeTransport = createFakeTransport();
		let resolveInstall: (config: typeof defaultConfig) => void;
		const installPromise = new Promise<typeof defaultConfig>((resolve) => {
			resolveInstall = resolve;
		});

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(null);
			if (cmd === "get_supported_lsp_languages")
				return Promise.resolve(["java"]);
			if (cmd === "install_lsp_server") return installPromise;
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockResolvedValue(fakeTransport);

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("downloading");
		});

		// biome-ignore lint/style/noNonNullAssertion: resolveInstall is assigned in Promise constructor
		resolveInstall!(defaultConfig);

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(result.current.sessionId).toBe(fakeTransport.sessionId);
	});

	it("ダウンロード失敗 → error 状態 + retryManually で復帰", async () => {
		const fakeTransport = createFakeTransport();
		let installCallCount = 0;

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(null);
			if (cmd === "get_supported_lsp_languages")
				return Promise.resolve(["java"]);
			if (cmd === "install_lsp_server") {
				installCallCount++;
				if (installCallCount === 1) {
					return Promise.reject(new Error("Download failed: network error"));
				}
				return Promise.resolve(defaultConfig);
			}
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockResolvedValue(fakeTransport);

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("error");
		});

		expect(result.current.error).toBe("Download failed: network error");

		act(() => {
			result.current.retryManually();
		});

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});
	});

	it("lsp-error イベント → 自動再起動", async () => {
		const fakeTransport1 = createFakeTransport();
		const fakeTransport2 = createFakeTransport();
		let transportCallCount = 0;

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(defaultConfig);
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockImplementation(() => {
			transportCallCount++;
			if (transportCallCount === 1) return Promise.resolve(fakeTransport1);
			return Promise.resolve(fakeTransport2);
		});

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(result.current.sessionId).toBe(fakeTransport1.sessionId);

		expect(capturedListeners.has("lsp-error")).toBe(true);

		act(() => {
			fireLspError(fakeTransport1.sessionId, "Server crashed");
		});

		expect(result.current.crashCount).toBe(1);
		expect(fakeTransport1.dispose).toHaveBeenCalled();

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(result.current.sessionId).toBe(fakeTransport2.sessionId);
	});

	it("3分以内に5回クラッシュ → stopped", async () => {
		vi.useFakeTimers();

		const transports = Array.from({ length: 6 }, () => createFakeTransport());
		let transportCallCount = 0;

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(defaultConfig);
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockImplementation(() => {
			const t = transports[transportCallCount];
			transportCallCount++;
			return Promise.resolve(t);
		});

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(capturedListeners.has("lsp-error")).toBe(true);

		// 4回クラッシュ → 自動再起動 (MAX_RESTARTS = 4)
		for (let i = 0; i < 4; i++) {
			act(() => {
				fireLspError(transports[i].sessionId, `Crash ${i + 1}`);
			});

			await vi.waitFor(() => {
				expect(result.current.status).toBe("running");
			});

			vi.advanceTimersByTime(1000);
		}

		expect(result.current.crashCount).toBe(4);

		// 5回目 → stopped
		act(() => {
			fireLspError(transports[4].sessionId, "Crash 5");
		});

		expect(result.current.status).toBe("stopped");
		expect(result.current.crashCount).toBe(5);

		vi.useRealTimers();
	});

	it("retryManually → クラッシュカウントリセット + 再起動", async () => {
		vi.useFakeTimers();

		const transports = Array.from({ length: 8 }, () => createFakeTransport());
		let transportCallCount = 0;

		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "detect_lsp_server") return Promise.resolve(defaultConfig);
			return Promise.resolve(null);
		});
		mockCreateTauriTransport.mockImplementation(() => {
			const t = transports[transportCallCount];
			transportCallCount++;
			return Promise.resolve(t);
		});

		const { result } = renderHook(() => useLsp("/workspace", "java"));

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		expect(capturedListeners.has("lsp-error")).toBe(true);

		// 5回クラッシュ → stopped
		for (let i = 0; i < 4; i++) {
			act(() => {
				fireLspError(transports[i].sessionId, `Crash ${i + 1}`);
			});
			await vi.waitFor(() => {
				expect(result.current.status).toBe("running");
			});
			vi.advanceTimersByTime(1000);
		}

		act(() => {
			fireLspError(transports[4].sessionId, "Crash 5");
		});

		expect(result.current.status).toBe("stopped");
		expect(result.current.crashCount).toBe(5);

		// retryManually → リセット + 再起動
		act(() => {
			result.current.retryManually();
		});

		expect(result.current.crashCount).toBe(0);

		await vi.waitFor(() => {
			expect(result.current.status).toBe("running");
		});

		vi.useRealTimers();
	});
});
