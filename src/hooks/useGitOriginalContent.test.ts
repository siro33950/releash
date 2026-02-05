import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useGitOriginalContent } from "./useGitOriginalContent";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@/lib/normalizePath", () => ({
	normalizePath: (p: string) => p,
}));

describe("useGitOriginalContent", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("should return content from get_file_at_ref on success", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_repo_git_dir") return Promise.resolve("/repo/.git");
			if (cmd === "get_file_at_ref") return Promise.resolve("original content");
			return Promise.resolve("");
		});

		const { result } = renderHook(() =>
			useGitOriginalContent("/repo/file.ts", "HEAD", "fallback"),
		);

		await waitFor(() => {
			expect(result.current).toBe("original content");
		});
	});

	it("should return content from get_staged_content when diffBase is staged", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_repo_git_dir") return Promise.resolve("/repo/.git");
			if (cmd === "get_staged_content")
				return Promise.resolve("staged content");
			return Promise.resolve("");
		});

		const { result } = renderHook(() =>
			useGitOriginalContent("/repo/file.ts", "staged", "fallback"),
		);

		await waitFor(() => {
			expect(result.current).toBe("staged content");
		});
	});

	it("should return empty string when invoke fails (untracked file)", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_repo_git_dir") return Promise.resolve("/repo/.git");
			return Promise.reject(new Error("file not found in ref"));
		});

		const { result } = renderHook(() =>
			useGitOriginalContent("/repo/untracked.ts", "HEAD", "current content"),
		);

		await waitFor(() => {
			expect(result.current).toBe("");
		});
	});

	it("should return fallbackContent when filePath is null", () => {
		mockInvoke.mockResolvedValue("");

		const { result } = renderHook(() =>
			useGitOriginalContent(null, "HEAD", "fallback"),
		);

		expect(result.current).toBe("fallback");
	});
});
