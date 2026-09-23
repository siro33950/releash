import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRepoList } from "./useRepoList";

const mockInvoke = vi.fn();
const mockListen = vi.fn();
vi.mock("@/lib/client", () => ({
	invokeClient: (...args: unknown[]) => mockInvoke(...args),
	listenClient: (...args: unknown[]) => mockListen(...args),
}));

describe("useRepoList", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockInvoke.mockResolvedValue(undefined);
	});
	it("登録一覧を読み取らず購読もしない", () => {
		renderHook(() => useRepoList());
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
