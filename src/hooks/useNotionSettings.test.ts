import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useNotionSettings } from "./useNotionSettings";

describe("useNotionSettings", () => {
	it("取得失敗を未設定と区別し失敗した設定を編集や保存の対象にしない", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (command, args) => {
			if (
				command === "get_notion_config" &&
				(args as { repoPath: string }).repoPath === "/repo/a"
			) {
				throw { code: "CONFIG_UNAVAILABLE", message: "Cannot load config" };
			}
			return null;
		});
		const { result, rerender } = renderHook(
			({ paths }) => useNotionSettings(paths),
			{ initialProps: { paths: ["/repo/a", "/repo/b"] } },
		);
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.errors.get("/repo/a")).toBe("Cannot load config");
		expect(result.current.drafts.has("/repo/a")).toBe(false);
		expect(result.current.drafts.get("/repo/b")?.apiToken).toBe("");

		act(() => {
			result.current.reset();
			for (const path of ["/repo/a", "/repo/b"]) {
				result.current.updateDraft(path, (draft) => ({
					...draft,
					apiToken: "new-token",
					databaseId: "new-db",
				}));
			}
			result.current.markForDelete("/repo/a");
		});
		await act(async () => {
			await result.current.validate("/repo/a");
			await result.current.save();
		});
		expect(
			vi
				.mocked(invoke)
				.mock.calls.filter(([command]) => command !== "get_notion_config"),
		).toEqual([
			[
				"save_notion_config",
				{
					repoPath: "/repo/b",
					apiToken: "new-token",
					databaseId: "new-db",
					propertyMapping: {
						title: "Name",
						labels: [],
						branch_name: "",
						branch_prefix: "",
					},
				},
			],
		]);
		expect(result.current.drafts.has("/repo/a")).toBe(false);
		rerender({ paths: [] });
		expect(result.current.errors.size).toBe(0);
	});

	it("should load configs for each repo path", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd, args) => {
			if (cmd === "get_notion_config") {
				const { repoPath } = args as { repoPath: string };
				if (repoPath === "/repo/a") {
					return {
						api_token: "token-a",
						database_id: "db-a",
						property_mapping: {
							title: "Name",
							labels: [],
							branch_name: "",
							branch_prefix: "",
						},
					};
				}
				return null;
			}
			return null;
		});

		const { result } = renderHook(() =>
			useNotionSettings(["/repo/a", "/repo/b"]),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.drafts.size).toBe(2);
		expect(result.current.drafts.get("/repo/a")?.apiToken).toBe("token-a");
		expect(result.current.drafts.get("/repo/b")?.apiToken).toBe("");
	});

	it("should report isDirty when draft changes", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.isDirty).toBe(false);

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "new-token",
			}));
		});

		expect(result.current.isDirty).toBe(true);
	});

	it("should report isDirty when marked for delete", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockResolvedValue({
			api_token: "token-a",
			database_id: "db-a",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.isDirty).toBe(false);

		act(() => {
			result.current.markForDelete("/repo/a");
		});

		expect(result.current.isDirty).toBe(true);
	});

	it("should save changed configs and delete marked ones", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd, args) => {
			if (cmd === "get_notion_config") {
				const { repoPath } = args as { repoPath: string };
				if (repoPath === "/repo/a") {
					return {
						api_token: "token-a",
						database_id: "db-a",
						property_mapping: {
							title: "Name",
							labels: [],
							branch_name: "",
							branch_prefix: "",
						},
					};
				}
				if (repoPath === "/repo/b") {
					return {
						api_token: "token-b",
						database_id: "db-b",
						property_mapping: {
							title: "Name",
							labels: [],
							branch_name: "",
							branch_prefix: "",
						},
					};
				}
				return null;
			}
			return undefined;
		});

		const { result } = renderHook(() =>
			useNotionSettings(["/repo/a", "/repo/b"]),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "new-token-a",
			}));
			result.current.markForDelete("/repo/b");
		});

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValue(null);

		await act(async () => {
			await result.current.save();
		});

		expect(invoke).toHaveBeenCalledWith("save_notion_config", {
			repoPath: "/repo/a",
			apiToken: "new-token-a",
			databaseId: "db-a",
			propertyMapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		});

		expect(invoke).toHaveBeenCalledWith("delete_notion_config", {
			repoPath: "/repo/b",
		});
	});

	it("should reset drafts to configs", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockResolvedValue({
			api_token: "token-a",
			database_id: "db-a",
			property_mapping: {
				title: "Name",
				labels: [],
				branch_name: "",
				branch_prefix: "",
			},
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "changed",
			}));
		});

		expect(result.current.isDirty).toBe(true);

		act(() => {
			result.current.reset();
		});

		expect(result.current.isDirty).toBe(false);
		expect(result.current.drafts.get("/repo/a")?.apiToken).toBe("token-a");
	});

	it("should validate a repo config", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return null;
			if (cmd === "validate_notion_config") {
				return {
					status: "configured",
					properties: [{ name: "Name", property_type: "title", options: [] }],
				};
			}
			return null;
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "test-token",
				databaseId: "test-db",
			}));
		});

		await act(async () => {
			await result.current.validate("/repo/a");
		});

		const draft = result.current.drafts.get("/repo/a");
		expect(draft?.validationStatus).toBe("success");
		expect(draft?.properties).toHaveLength(1);
		expect(draft?.validating).toBe(false);
	});

	it("should handle empty repoPaths", async () => {
		const { result } = renderHook(() => useNotionSettings([]));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.drafts.size).toBe(0);
		expect(result.current.isDirty).toBe(false);
	});

	it("should set validationStatus for invalid_token", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return null;
			if (cmd === "validate_notion_config") {
				return { status: "invalid_token", properties: [] };
			}
			return null;
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "bad-token",
				databaseId: "db",
			}));
		});

		await act(async () => {
			await result.current.validate("/repo/a");
		});

		expect(result.current.drafts.get("/repo/a")?.validationStatus).toBe(
			"Invalid API token",
		);
		expect(result.current.drafts.get("/repo/a")?.validating).toBe(false);
	});

	it("should set validationStatus for invalid_database", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return null;
			if (cmd === "validate_notion_config") {
				return { status: "invalid_database", properties: [] };
			}
			return null;
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "token",
				databaseId: "bad-db",
			}));
		});

		await act(async () => {
			await result.current.validate("/repo/a");
		});

		expect(result.current.drafts.get("/repo/a")?.validationStatus).toBe(
			"Invalid database ID",
		);
	});

	it("should set validationStatus for network_error", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return null;
			if (cmd === "validate_notion_config") {
				return { status: "network_error", properties: [] };
			}
			return null;
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "token",
				databaseId: "db",
			}));
		});

		await act(async () => {
			await result.current.validate("/repo/a");
		});

		expect(result.current.drafts.get("/repo/a")?.validationStatus).toBe(
			"Network error: Check your connection",
		);
	});

	it("should handle validate exception without leaving validating stuck", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "get_notion_config") return null;
			if (cmd === "validate_notion_config") {
				throw new Error("Connection refused");
			}
			return null;
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				apiToken: "token",
				databaseId: "db",
			}));
		});

		await act(async () => {
			await result.current.validate("/repo/a");
		});

		const draft = result.current.drafts.get("/repo/a");
		expect(draft?.validating).toBe(false);
		expect(draft?.validationStatus).toContain("Connection refused");
	});

	it("should not call save_notion_config when apiToken is empty", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockResolvedValue(null);

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				databaseId: "db-id",
			}));
		});

		expect(result.current.isDirty).toBe(true);

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValue(null);

		await act(async () => {
			await result.current.save();
		});

		const saveCalls = vi
			.mocked(invoke)
			.mock.calls.filter(([cmd]) => cmd === "save_notion_config");
		expect(saveCalls).toHaveLength(0);
	});

	it.each([false, true])(
		"保存後の古い取得signalで回復後のロードを無効にしない: error=%s",
		async (failure) => {
			const { invokeClient: invoke, onClientRefresh } = await import(
				"@/lib/client"
			);
			const config = {
				api_token: "old",
				database_id: "db",
				property_mapping: {
					title: "Name",
					labels: [],
					branch_name: "",
					branch_prefix: "",
				},
			};
			let finishSave!: () => void;
			let finishLoad!: (value: typeof config) => void;
			let failLoad!: (cause: unknown) => void;
			let loadingRecovery = false;
			vi.mocked(invoke).mockImplementation((command) => {
				if (command === "save_notion_config")
					return new Promise<void>((resolve) => {
						finishSave = resolve;
					});
				if (command === "get_notion_config" && loadingRecovery)
					return new Promise((resolve, reject) => {
						finishLoad = resolve;
						failLoad = reject;
					});
				return Promise.resolve(config);
			});
			const { result } = renderHook(() => useNotionSettings(["/repo"]));
			await waitFor(() => expect(result.current.loading).toBe(false));
			act(() =>
				result.current.updateDraft("/repo", (draft) => ({
					...draft,
					apiToken: "saved",
				})),
			);
			let saving!: Promise<void>;
			act(() => {
				saving = result.current.save();
			});
			act(() => {
				loadingRecovery = true;
				vi.mocked(onClientRefresh).mock.lastCall?.[0]();
			});
			expect(result.current.loading).toBe(true);
			await act(async () => {
				finishSave();
				await saving;
			});
			await act(async () => {
				if (failure) failLoad(new Error("recovery failed"));
				else finishLoad({ ...config, api_token: "current" });
			});
			expect(result.current.loading).toBe(false);
			if (failure)
				expect(result.current.errors.get("/repo")).toBe("recovery failed");
			else {
				expect(result.current.drafts.get("/repo")?.apiToken).toBe("current");
				expect(result.current.errors.size).toBe(0);
			}
		},
	);
});
