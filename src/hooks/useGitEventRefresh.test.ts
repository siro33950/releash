import { renderHook } from "@testing-library/react";
import {
	afterEach,
	beforeEach,
	describe,
	expect,
	it,
	type Mock,
	vi,
} from "vitest";
import { useGitEventRefresh } from "./useGitEventRefresh";

type ListenCallback = (event: { payload: Record<string, string> }) => void;

const mockListen = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

describe("useGitEventRefresh", () => {
	beforeEach(() => {
		vi.useFakeTimers();
		mockListen.mockResolvedValue(vi.fn());
	});

	afterEach(() => {
		vi.useRealTimers();
		vi.clearAllMocks();
	});

	it("should not register listeners when enabled=false", () => {
		const onRefresh = vi.fn();
		renderHook(() => useGitEventRefresh("/repo", onRefresh, false));
		expect(mockListen).not.toHaveBeenCalled();
	});

	it("should not register listeners when rootPath=null", () => {
		const onRefresh = vi.fn();
		renderHook(() => useGitEventRefresh(null, onRefresh));
		expect(mockListen).not.toHaveBeenCalled();
	});

	it("should register both listeners when enabled and rootPath provided", async () => {
		const onRefresh = vi.fn();
		renderHook(() => useGitEventRefresh("/repo", onRefresh));

		await vi.waitFor(() => {
			expect(mockListen).toHaveBeenCalledTimes(2);
		});

		expect(mockListen).toHaveBeenCalledWith(
			"file-change",
			expect.any(Function),
		);
		expect(mockListen).toHaveBeenCalledWith(
			"git-status-changed",
			expect.any(Function),
		);
	});

	it("should debounce onRefresh on file-change event with matching path", async () => {
		const onRefresh = vi.fn();
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				setTimeout(() => cb({ payload: { path: "/repo/src/main.ts" } }), 0);
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitEventRefresh("/repo", onRefresh));

		await vi.advanceTimersByTimeAsync(0);
		expect(onRefresh).not.toHaveBeenCalled();

		await vi.advanceTimersByTimeAsync(300);
		expect(onRefresh).toHaveBeenCalledTimes(1);
	});

	it("should ignore file-change event with non-matching path", async () => {
		const onRefresh = vi.fn();
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				setTimeout(() => cb({ payload: { path: "/other-repo/file.ts" } }), 0);
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitEventRefresh("/repo", onRefresh));

		await vi.advanceTimersByTimeAsync(0);
		await vi.advanceTimersByTimeAsync(300);
		expect(onRefresh).not.toHaveBeenCalled();
	});

	it("should debounce onRefresh on git-status-changed event with matching repo_path", async () => {
		const onRefresh = vi.fn();
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "git-status-changed") {
				setTimeout(() => cb({ payload: { repo_path: "/repo" } }), 0);
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitEventRefresh("/repo", onRefresh));

		await vi.advanceTimersByTimeAsync(0);
		expect(onRefresh).not.toHaveBeenCalled();

		await vi.advanceTimersByTimeAsync(300);
		expect(onRefresh).toHaveBeenCalledTimes(1);
	});

	it("should ignore git-status-changed event with non-matching repo_path", async () => {
		const onRefresh = vi.fn();
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "git-status-changed") {
				setTimeout(() => cb({ payload: { repo_path: "/other-repo" } }), 0);
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitEventRefresh("/repo", onRefresh));

		await vi.advanceTimersByTimeAsync(0);
		await vi.advanceTimersByTimeAsync(300);
		expect(onRefresh).not.toHaveBeenCalled();
	});

	it("should call unlisten on cleanup", async () => {
		const mockUnlisten = vi.fn();
		mockListen.mockResolvedValue(mockUnlisten);

		const { unmount } = renderHook(() => useGitEventRefresh("/repo", vi.fn()));

		await vi.waitFor(() => {
			expect(mockListen).toHaveBeenCalledTimes(2);
		});

		unmount();
		expect(mockUnlisten).toHaveBeenCalledTimes(2);
	});

	it("should debounce multiple rapid events into single call", async () => {
		const onRefresh = vi.fn();
		let fileChangeCb: ListenCallback | null = null;

		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCb = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitEventRefresh("/repo", onRefresh));

		await vi.waitFor(() => {
			expect(fileChangeCb).not.toBeNull();
		});

		(fileChangeCb as unknown as Mock)({ payload: { path: "/repo/a.ts" } });
		(fileChangeCb as unknown as Mock)({ payload: { path: "/repo/b.ts" } });
		(fileChangeCb as unknown as Mock)({ payload: { path: "/repo/c.ts" } });

		await vi.advanceTimersByTimeAsync(300);
		expect(onRefresh).toHaveBeenCalledTimes(1);
	});
});
