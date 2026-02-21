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

	// === 1. load()エラーケース ===
	it("should set config to null when load fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockRejectedValue(new Error("load failed"));

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.config).toBeNull();
		expect(result.current.isConfigured).toBe(false);
	});

	it("should set loading to false after load error", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockRejectedValue(new Error("network error"));

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});
	});

	// === 2. save()エラーケース ===
	it("should propagate error when save fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockRejectedValue(new Error("save failed"));

		await expect(
			result.current.save("ntn_token", "db-456", {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			}),
		).rejects.toThrow("save failed");
	});

	it("should reload config after successful save", async () => {
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

		vi.mocked(invoke).mockResolvedValue(null);
		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockResolvedValueOnce(undefined);
		vi.mocked(invoke).mockResolvedValueOnce(mockConfig);

		await result.current.save("ntn_token", "db-456", {
			title: "Name",
			labels: [],
			branch_name: "",
			branch_prefix: "",
		});

		await waitFor(() => {
			expect(result.current.config).toEqual(mockConfig);
		});
	});

	it("should call save_notion_config before reloading config", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValueOnce(undefined);
		vi.mocked(invoke).mockResolvedValueOnce(null);

		await result.current.save("ntn_token", "db-456", {
			title: "Name",
			labels: [],
			branch_name: "",
			branch_prefix: "",
		});

		expect(invoke).toHaveBeenNthCalledWith(
			1,
			"save_notion_config",
			expect.any(Object),
		);
		expect(invoke).toHaveBeenNthCalledWith(
			2,
			"get_notion_config",
			expect.any(Object),
		);
	});

	// === 3. remove()エラーケース ===
	it("should propagate error when remove fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockRejectedValue(new Error("delete failed"));

		await expect(result.current.remove()).rejects.toThrow("delete failed");
	});

	it("should set config to null immediately after successful remove", async () => {
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

		vi.mocked(invoke).mockResolvedValue(undefined);
		await result.current.remove();

		await waitFor(() => {
			expect(result.current.config).toBeNull();
			expect(result.current.isConfigured).toBe(false);
		});
	});

	// === 4. validate()エラーケース ===
	it("should propagate error when validate fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockRejectedValue(new Error("validation failed"));

		await expect(
			result.current.validate("ntn_token", "db-789"),
		).rejects.toThrow("validation failed");
	});

	it("should return validation result with invalid_token status", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		const mockValidation = { status: "invalid_token", properties: [] };
		vi.mocked(invoke).mockResolvedValue(mockValidation);

		const validationResult = await result.current.validate("ntn_bad", "db-789");

		expect(validationResult).toEqual(mockValidation);
	});

	it("should return validation result with invalid_database status", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		const mockValidation = { status: "invalid_database", properties: [] };
		vi.mocked(invoke).mockResolvedValue(mockValidation);

		const validationResult = await result.current.validate(
			"ntn_token",
			"db-bad",
		);

		expect(validationResult).toEqual(mockValidation);
	});

	// === 5. 状態遷移テスト ===
	it("should handle repoPath change and reload config", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const config1 = {
			api_token: "ntn_repo1",
			database_id: "db-1",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};
		const config2 = {
			api_token: "ntn_repo2",
			database_id: "db-2",
			property_mapping: {
				title: "Title",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		};

		vi.mocked(invoke).mockResolvedValue(config1);
		const { result, rerender } = renderHook(
			({ repoPath }) => useNotionConfig(repoPath),
			{ initialProps: { repoPath: "/repo1" } },
		);

		await waitFor(() => {
			expect(result.current.config).toEqual(config1);
		});

		vi.mocked(invoke).mockResolvedValue(config2);
		rerender({ repoPath: "/repo2" });

		await waitFor(() => {
			expect(result.current.config).toEqual(config2);
		});

		expect(invoke).toHaveBeenCalledWith("get_notion_config", {
			repoPath: "/repo2",
		});
	});

	it("should handle multiple sequential saves", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionConfig("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValue(undefined);

		await result.current.save("ntn_1", "db-1", {
			title: "Name",
			labels: [],
			branch_name: "",
			branch_prefix: "",
		});
		await result.current.save("ntn_2", "db-2", {
			title: "Title",
			labels: [],
			branch_name: "",
			branch_prefix: "",
		});

		expect(invoke).toHaveBeenCalledTimes(4); // save1 + reload1 + save2 + reload2
	});

	it("should maintain isConfigured after failed save", async () => {
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

		vi.mocked(invoke).mockRejectedValue(new Error("save failed"));

		try {
			await result.current.save("ntn_new", "db-new", {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			});
		} catch {
			// expected
		}

		// save失敗時にloadが呼ばれないため、isConfiguredはtrueのまま
		expect(result.current.isConfigured).toBe(true);
	});
});
