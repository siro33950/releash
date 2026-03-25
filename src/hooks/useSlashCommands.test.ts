import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	loadSlashCommands,
	setSlashCommands,
	useSlashCommands,
} from "./useSlashCommands";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

beforeEach(() => {
	setSlashCommands([]);
	vi.clearAllMocks();
});

describe("useSlashCommands", () => {
	it("returns empty array initially", () => {
		const { result } = renderHook(() => useSlashCommands());
		expect(result.current).toEqual([]);
	});

	it("returns commands after setSlashCommands is called", () => {
		const { result } = renderHook(() => useSlashCommands());

		act(() => {
			setSlashCommands([
				{ name: "plan-spec", description: "Create plan spec" },
				{
					name: "review",
					description: "Code review",
					argumentHint: "<file>",
				},
			]);
		});

		expect(result.current).toEqual([
			{ name: "plan-spec", description: "Create plan spec" },
			{ name: "review", description: "Code review", argumentHint: "<file>" },
		]);
	});

	it("updates when commands change", () => {
		const { result } = renderHook(() => useSlashCommands());

		act(() => {
			setSlashCommands([{ name: "commit", description: "Create commit" }]);
		});
		expect(result.current).toHaveLength(1);

		act(() => {
			setSlashCommands([
				{ name: "commit", description: "Create commit" },
				{ name: "review", description: "Code review" },
			]);
		});
		expect(result.current).toHaveLength(2);
	});

	it("shares cache across multiple hook instances", () => {
		const { result: r1 } = renderHook(() => useSlashCommands());
		const { result: r2 } = renderHook(() => useSlashCommands());

		act(() => {
			setSlashCommands([
				{ name: "plan-spec", description: "Create plan spec" },
			]);
		});

		expect(r1.current).toBe(r2.current);
	});
});

describe("loadSlashCommands", () => {
	it("invokes scan_slash_commands and updates cache", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockCommands = [
			{ name: "review", description: "Code review" },
			{ name: "commit", description: "Create commit" },
		];
		vi.mocked(invoke).mockResolvedValueOnce(mockCommands);

		const { result } = renderHook(() => useSlashCommands());
		expect(result.current).toEqual([]);

		await act(async () => {
			await loadSlashCommands("/test/path");
		});

		expect(invoke).toHaveBeenCalledWith("scan_slash_commands", {
			cwd: "/test/path",
		});
		expect(result.current).toEqual(mockCommands);
	});
});
