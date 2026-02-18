import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useNotionConfig } from "./useNotionConfig";

describe("useNotionConfig", () => {
	it("should invoke get_notion_config on mount", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(invoke).toHaveBeenCalledWith("get_notion_config", {
			repoPath: "/test/repo",
		});
	});

	it("should set isConfigured to false when no config exists", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.isConfigured).toBe(false);
		expect(result.current.config).toBeNull();
	});

	it("should set isConfigured to true when config exists", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockConfig = {
			api_token: "ntn_test",
			database_id: "db-123",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};
		vi.mocked(invoke).mockResolvedValue(mockConfig);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.isConfigured).toBe(true);
		});

		expect(result.current.config).toEqual(mockConfig);
	});

	it("should call save_notion_config on save", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValue(undefined);

		await result.current.save("ntn_token", "db-456", {
			title: "Name",
			labels: [
				{ name: "Tags", property_type: "multi_select" },
				{ name: "Status", property_type: "status" },
			],
			branch_name: "Branch",
			branch_prefix: "feat/",
		});

		expect(invoke).toHaveBeenCalledWith("save_notion_config", {
			repoPath: "/test/repo",
			apiToken: "ntn_token",
			databaseId: "db-456",
			propertyMapping: {
				title: "Name",
				labels: [
					{ name: "Tags", property_type: "multi_select" },
					{ name: "Status", property_type: "status" },
				],
				branch_name: "Branch",
				branch_prefix: "feat/",
			},
		});
	});

	it("should call delete_notion_config on remove", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValue(undefined);

		await result.current.remove();

		expect(invoke).toHaveBeenCalledWith("delete_notion_config", {
			repoPath: "/test/repo",
		});
	});

	it("should call validate_notion_config on validate", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		const mockValidation = {
			status: "configured",
			properties: [{ name: "Name", property_type: "title" }],
		};
		vi.mocked(invoke).mockResolvedValue(mockValidation);

		const validationResult = await result.current.validate(
			"ntn_token",
			"db-789",
		);

		expect(invoke).toHaveBeenCalledWith("validate_notion_config", {
			apiToken: "ntn_token",
			databaseId: "db-789",
		});
		expect(validationResult).toEqual(mockValidation);
	});
});
