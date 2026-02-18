import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useNotionLabelOptions } from "./useNotionLabelOptions";

describe("useNotionLabelOptions", () => {
	it("should invoke fetch_notion_label_options on mount", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue([]);

		const { result } = renderHook(() => useNotionLabelOptions("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(invoke).toHaveBeenCalledWith("fetch_notion_label_options", {
			repoPath: "/test/repo",
		});
	});

	it("should set labelOptions from result", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockOptions = [
			{
				property_name: "Status",
				property_type: "status",
				options: ["Todo", "In Progress", "Done"],
			},
			{
				property_name: "Tags",
				property_type: "multi_select",
				options: ["frontend", "backend"],
			},
		];
		vi.mocked(invoke).mockResolvedValue(mockOptions);

		const { result } = renderHook(() => useNotionLabelOptions("/test/repo"));

		await waitFor(() => {
			expect(result.current.labelOptions).toHaveLength(2);
		});

		expect(result.current.labelOptions[0].property_name).toBe("Status");
		expect(result.current.labelOptions[0].options).toEqual([
			"Todo",
			"In Progress",
			"Done",
		]);
	});

	it("should set empty options on error", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockRejectedValue(new Error("not configured"));

		const { result } = renderHook(() => useNotionLabelOptions("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.labelOptions).toEqual([]);
	});
});
