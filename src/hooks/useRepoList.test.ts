import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRepoList } from "./useRepoList";

const mockInvoke = vi.fn();
const mockListen = vi.fn();
const mockSubscribe = vi.fn();
vi.mock("@/lib/client", () => ({
	invokeClient: (...args: unknown[]) => mockInvoke(...args),
	listenClient: (...args: unknown[]) => mockListen(...args),
	subscribeState: (...args: unknown[]) => mockSubscribe(...args),
}));

describe("useRepoList", () => {
	const unsubscribe = vi.fn();
	beforeEach(() => {
		vi.clearAllMocks();
		mockInvoke.mockResolvedValue(undefined);
		mockSubscribe.mockReturnValue(unsubscribe);
	});
	it("購読の一覧を表示し終了時に停止する", () => {
		const { result, unmount } = renderHook(() => useRepoList());
		const [target, receive] = mockSubscribe.mock.calls[0] as [
			string,
			(paths: string[]) => void,
		];
		expect(target).toBe("repository-paths");
		act(() => receive(["/repo"]));
		expect(result.current.repoPaths).toEqual(["/repo"]);
		unmount();
		expect(unsubscribe).toHaveBeenCalledOnce();
		expect(mockInvoke).not.toHaveBeenCalled();
		expect(mockListen).not.toHaveBeenCalled();
	});
	it("should call invoke('add_repo_path') when addRepo is called", async () => {
		const { result } = renderHook(() => useRepoList());
		await act(async () => {});

		act(() => {
			result.current.addRepo("/repo/new");
		});

		expect(mockInvoke).toHaveBeenCalledWith("add_repo_path", {
			path: "/repo/new",
		});
	});

	it("should call invoke('remove_repo_path') when removeRepo is called", async () => {
		const { result } = renderHook(() => useRepoList());
		await act(async () => {});

		act(() => {
			result.current.removeRepo("/repo/a");
		});

		expect(mockInvoke).toHaveBeenCalledWith("remove_repo_path", {
			path: "/repo/a",
		});
	});

	it("should call invoke('add_repo_path') when initFromCwd is called", async () => {
		const { result } = renderHook(() => useRepoList());
		await act(async () => {});

		act(() => {
			result.current.initFromCwd("/workspace/project");
		});

		expect(mockInvoke).toHaveBeenCalledWith("add_repo_path", {
			path: "/workspace/project",
		});
	});
});
