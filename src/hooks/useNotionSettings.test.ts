import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient as invoke, type StateTarget } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useNotionSettings } from "./useNotionSettings";

const states = stateSubscriptions();
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

const target = (path: string): StateTarget<"notion-config"> => ({
	kind: "notion-config",
	args: [path],
});
const config = (suffix: string) => ({
	api_token: `token-${suffix}`,
	database_id: `db-${suffix}`,
	property_mapping: {
		title: "Name",
		labels: [],
		branch_name: "",
		branch_prefix: "",
	},
});

describe("useNotionSettings", () => {
	beforeEach(() => {
		states.clear();
		vi.mocked(invoke)
			.mockReset()
			.mockResolvedValue(null as never);
	});

	it("取得失敗を未設定と区別し失敗した設定を編集や保存の対象にしない", async () => {
		states.publish(target("/repo/b"), null);
		const { result, rerender } = renderHook(
			({ paths }) => useNotionSettings(paths),
			{ initialProps: { paths: ["/repo/a", "/repo/b"] } },
		);
		act(() =>
			states.fail(target("/repo/a"), {
				code: "CONFIG_UNAVAILABLE",
				message: "Cannot load config",
			}),
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
		expect(vi.mocked(invoke).mock.calls).toEqual([
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
		states.publish(target("/repo/a"), config("a"));
		states.publish(target("/repo/b"), null);

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
		states.publish(target("/repo/a"), null);

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
		states.publish(target("/repo/a"), config("a"));

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
		states.publish(target("/repo/a"), config("a"));
		states.publish(target("/repo/b"), config("b"));

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
		states.publish(target("/repo/a"), config("a"));

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

	it.each([
		["configured", "success"],
		["invalid_token", "Invalid API token"],
		["invalid_database", "Invalid database ID"],
		["network_error", "Network error: Check your connection"],
	])("validate結果%sを%sとして表示する", async (status, expected) => {
		states.publish(target("/repo/a"), null);
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "validate_notion_config") {
				return {
					status,
					properties:
						status === "configured"
							? [{ name: "Name", property_type: "title", options: [] }]
							: [],
				} as never;
			}
			return null as never;
		});

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

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
		expect(draft?.validationStatus).toBe(expected);
		expect(draft?.properties).toHaveLength(status === "configured" ? 1 : 0);
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

	it("should handle validate exception without leaving validating stuck", async () => {
		states.publish(target("/repo/a"), null);
		vi.mocked(invoke).mockImplementation(async (cmd) => {
			if (cmd === "validate_notion_config") {
				throw new Error("Connection refused");
			}
			return null as never;
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
		states.publish(target("/repo/a"), null);

		const { result } = renderHook(() => useNotionSettings(["/repo/a"]));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => {
			result.current.updateDraft("/repo/a", (d) => ({
				...d,
				databaseId: "db-id",
			}));
		});

		expect(result.current.isDirty).toBe(true);

		await act(async () => {
			await result.current.save();
		});

		expect(invoke).not.toHaveBeenCalledWith(
			"save_notion_config",
			expect.anything(),
		);
	});

	it("購読で届いた設定は未保存の編集を上書きせず保存後の値は反映する", async () => {
		states.publish(target("/repo"), config("old"));
		let finishSave!: () => void;
		vi.mocked(invoke).mockImplementation(
			(command) =>
				new Promise<void>((resolve) => {
					if (command === "save_notion_config") finishSave = () => resolve();
				}) as never,
		);
		const { result } = renderHook(() => useNotionSettings(["/repo"]));
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() =>
			result.current.updateDraft("/repo", (draft) => ({
				...draft,
				apiToken: "saved",
			})),
		);
		act(() => states.publish(target("/repo"), config("other")));
		expect(result.current.drafts.get("/repo")?.apiToken).toBe("saved");
		let saving!: Promise<void>;
		act(() => {
			saving = result.current.save();
		});
		await act(async () => {
			finishSave();
			await saving;
		});
		expect(result.current.isDirty).toBe(false);
		act(() =>
			states.publish(target("/repo"), {
				...config("other"),
				api_token: "current",
			}),
		);
		expect(result.current.drafts.get("/repo")?.apiToken).toBe("current");
		expect(result.current.errors.size).toBe(0);
	});
});
