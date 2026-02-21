import { describe, expect, it } from "vitest";
import { type AppSettings, DEFAULT_SETTINGS } from "@/types/settings";
import { configFormReducer } from "./NotionPanel";
import { hooksReducer, settingsReducer } from "./SettingsModal";
import { commitFormReducer } from "./SourceControlPanel";

// --- commitFormReducer ---

describe("commitFormReducer", () => {
	const initial = {
		summary: "",
		description: "",
		error: null as string | null,
		loading: false,
		discardTarget: null as { path: string; paths: string[] } | null,
	};

	it("SET_SUMMARY updates summary", () => {
		const state = commitFormReducer(initial, {
			type: "SET_SUMMARY",
			value: "fix: typo",
		});
		expect(state.summary).toBe("fix: typo");
	});

	it("SET_DESCRIPTION updates description", () => {
		const state = commitFormReducer(initial, {
			type: "SET_DESCRIPTION",
			value: "details here",
		});
		expect(state.description).toBe("details here");
	});

	it("SET_ERROR updates error", () => {
		const state = commitFormReducer(initial, {
			type: "SET_ERROR",
			error: "something failed",
		});
		expect(state.error).toBe("something failed");
	});

	it("SET_ERROR clears error with null", () => {
		const prev = { ...initial, error: "old error" };
		const state = commitFormReducer(prev, { type: "SET_ERROR", error: null });
		expect(state.error).toBeNull();
	});

	it("COMMIT_START sets loading=true and clears error", () => {
		const prev = { ...initial, error: "old error" };
		const state = commitFormReducer(prev, { type: "COMMIT_START" });
		expect(state.loading).toBe(true);
		expect(state.error).toBeNull();
	});

	it("COMMIT_SUCCESS resets loading, summary, and description", () => {
		const prev = {
			...initial,
			loading: true,
			summary: "fix: something",
			description: "detail",
		};
		const state = commitFormReducer(prev, { type: "COMMIT_SUCCESS" });
		expect(state.loading).toBe(false);
		expect(state.summary).toBe("");
		expect(state.description).toBe("");
	});

	it("COMMIT_ERROR sets error and clears loading", () => {
		const prev = { ...initial, loading: true };
		const state = commitFormReducer(prev, {
			type: "COMMIT_ERROR",
			error: "commit failed",
		});
		expect(state.loading).toBe(false);
		expect(state.error).toBe("commit failed");
	});

	it("PUSH_START sets loading=true and clears error", () => {
		const prev = { ...initial, error: "old" };
		const state = commitFormReducer(prev, { type: "PUSH_START" });
		expect(state.loading).toBe(true);
		expect(state.error).toBeNull();
	});

	it("PUSH_END clears loading", () => {
		const prev = { ...initial, loading: true };
		const state = commitFormReducer(prev, { type: "PUSH_END" });
		expect(state.loading).toBe(false);
	});

	it("PUSH_ERROR sets error and clears loading", () => {
		const prev = { ...initial, loading: true };
		const state = commitFormReducer(prev, {
			type: "PUSH_ERROR",
			error: "push rejected",
		});
		expect(state.loading).toBe(false);
		expect(state.error).toBe("push rejected");
	});

	it("SET_DISCARD_TARGET updates discardTarget", () => {
		const target = { path: "src/a.ts", paths: ["src/a.ts"] };
		const state = commitFormReducer(initial, {
			type: "SET_DISCARD_TARGET",
			target,
		});
		expect(state.discardTarget).toEqual(target);
	});

	it("CLEAR_DISCARD resets discardTarget to null", () => {
		const prev = {
			...initial,
			discardTarget: { path: "a.ts", paths: ["a.ts"] },
		};
		const state = commitFormReducer(prev, { type: "CLEAR_DISCARD" });
		expect(state.discardTarget).toBeNull();
	});
});

// --- configFormReducer ---

describe("configFormReducer", () => {
	const initial = {
		apiToken: "",
		databaseId: "",
		mapping: {
			title: "Name",
			labels: [] as { name: string; property_type: string }[],
			branch_name: "",
			branch_prefix: "",
		},
		validating: false,
		saving: false,
		properties: [] as { name: string; property_type: string }[],
		validationStatus: null as string | null,
		deleting: false,
		saveError: null as string | null,
		deleteError: null as string | null,
	};

	it("SET_API_TOKEN updates apiToken", () => {
		const state = configFormReducer(initial, {
			type: "SET_API_TOKEN",
			value: "ntn_abc",
		});
		expect(state.apiToken).toBe("ntn_abc");
	});

	it("SET_DATABASE_ID updates databaseId", () => {
		const state = configFormReducer(initial, {
			type: "SET_DATABASE_ID",
			value: "db-123",
		});
		expect(state.databaseId).toBe("db-123");
	});

	it("UPDATE_MAPPING partially updates mapping", () => {
		const state = configFormReducer(initial, {
			type: "UPDATE_MAPPING",
			update: { branch_prefix: "feat/" },
		});
		expect(state.mapping.branch_prefix).toBe("feat/");
		expect(state.mapping.title).toBe("Name");
	});

	it("UPDATE_MAPPING merges with existing mapping", () => {
		const prev = {
			...initial,
			mapping: { ...initial.mapping, title: "Title", branch_name: "branch" },
		};
		const state = configFormReducer(prev, {
			type: "UPDATE_MAPPING",
			update: { title: "NewTitle" },
		});
		expect(state.mapping.title).toBe("NewTitle");
		expect(state.mapping.branch_name).toBe("branch");
	});

	it("VALIDATE_START sets validating=true and clears validationStatus", () => {
		const prev = { ...initial, validationStatus: "old status" };
		const state = configFormReducer(prev, { type: "VALIDATE_START" });
		expect(state.validating).toBe(true);
		expect(state.validationStatus).toBeNull();
	});

	it("VALIDATE_SUCCESS sets properties and validationStatus", () => {
		const properties = [{ name: "Name", property_type: "title" }];
		const state = configFormReducer(initial, {
			type: "VALIDATE_SUCCESS",
			properties,
			status: "success",
		});
		expect(state.validating).toBe(false);
		expect(state.properties).toEqual(properties);
		expect(state.validationStatus).toBe("success");
	});

	it("VALIDATE_ERROR sets validationStatus to error message", () => {
		const prev = { ...initial, validating: true };
		const state = configFormReducer(prev, {
			type: "VALIDATE_ERROR",
			error: "network error",
		});
		expect(state.validating).toBe(false);
		expect(state.validationStatus).toBe("network error");
	});

	it("SAVE_START sets saving=true and clears saveError", () => {
		const prev = { ...initial, saveError: "old" };
		const state = configFormReducer(prev, { type: "SAVE_START" });
		expect(state.saving).toBe(true);
		expect(state.saveError).toBeNull();
	});

	it("SAVE_ERROR sets saveError and clears saving", () => {
		const prev = { ...initial, saving: true };
		const state = configFormReducer(prev, {
			type: "SAVE_ERROR",
			error: "save failed",
		});
		expect(state.saving).toBe(false);
		expect(state.saveError).toBe("save failed");
	});

	it("SAVE_END clears saving", () => {
		const prev = { ...initial, saving: true };
		const state = configFormReducer(prev, { type: "SAVE_END" });
		expect(state.saving).toBe(false);
	});

	it("DELETE_START sets deleting=true and clears deleteError", () => {
		const prev = { ...initial, deleteError: "old" };
		const state = configFormReducer(prev, { type: "DELETE_START" });
		expect(state.deleting).toBe(true);
		expect(state.deleteError).toBeNull();
	});

	it("DELETE_ERROR sets deleteError and clears deleting", () => {
		const prev = { ...initial, deleting: true };
		const state = configFormReducer(prev, {
			type: "DELETE_ERROR",
			error: "delete failed",
		});
		expect(state.deleting).toBe(false);
		expect(state.deleteError).toBe("delete failed");
	});

	it("DELETE_END clears deleting", () => {
		const prev = { ...initial, deleting: true };
		const state = configFormReducer(prev, { type: "DELETE_END" });
		expect(state.deleting).toBe(false);
	});
});

// --- settingsReducer ---

describe("settingsReducer", () => {
	const initial = {
		activeSection: "appearance" as const,
		draft: DEFAULT_SETTINGS,
		appDirty: false,
		saving: false,
		prevOpen: false,
	};

	it("SET_SECTION updates activeSection", () => {
		const state = settingsReducer(initial, {
			type: "SET_SECTION",
			section: "editor",
		});
		expect(state.activeSection).toBe("editor");
	});

	it("UPDATE_DRAFT applies updater function and sets appDirty=true", () => {
		const state = settingsReducer(initial, {
			type: "UPDATE_DRAFT",
			updater: (d: AppSettings) => ({ ...d, fontSize: 20 }),
		});
		expect(state.draft.fontSize).toBe(20);
		expect(state.appDirty).toBe(true);
	});

	it("UPDATE_DRAFT preserves other draft fields", () => {
		const state = settingsReducer(initial, {
			type: "UPDATE_DRAFT",
			updater: (d: AppSettings) => ({ ...d, theme: "light" }),
		});
		expect(state.draft.theme).toBe("light");
		expect(state.draft.fontSize).toBe(DEFAULT_SETTINGS.fontSize);
	});

	it("SYNC_OPEN when opening (open=true, prevOpen=false) resets draft and appDirty", () => {
		const customSettings = { ...DEFAULT_SETTINGS, fontSize: 20 };
		const prev = { ...initial, appDirty: true };
		const state = settingsReducer(prev, {
			type: "SYNC_OPEN",
			open: true,
			settings: customSettings,
		});
		expect(state.prevOpen).toBe(true);
		expect(state.draft.fontSize).toBe(20);
		expect(state.appDirty).toBe(false);
	});

	it("SYNC_OPEN when already open does not reset draft", () => {
		const prev = {
			...initial,
			prevOpen: true,
			draft: { ...DEFAULT_SETTINGS, fontSize: 18 },
			appDirty: true,
		};
		const state = settingsReducer(prev, {
			type: "SYNC_OPEN",
			open: true,
			settings: DEFAULT_SETTINGS,
		});
		expect(state.draft.fontSize).toBe(18);
		expect(state.appDirty).toBe(true);
	});

	it("SYNC_OPEN when closing only updates prevOpen", () => {
		const prev = { ...initial, prevOpen: true };
		const state = settingsReducer(prev, {
			type: "SYNC_OPEN",
			open: false,
			settings: DEFAULT_SETTINGS,
		});
		expect(state.prevOpen).toBe(false);
	});

	it("SAVE_START sets saving=true and appDirty=false", () => {
		const prev = { ...initial, appDirty: true };
		const state = settingsReducer(prev, { type: "SAVE_START" });
		expect(state.saving).toBe(true);
		expect(state.appDirty).toBe(false);
	});

	it("SAVE_END clears saving", () => {
		const prev = { ...initial, saving: true };
		const state = settingsReducer(prev, { type: "SAVE_END" });
		expect(state.saving).toBe(false);
	});

	it("SAVE_ERROR sets appDirty back to true", () => {
		const prev = { ...initial, saving: true, appDirty: false };
		const state = settingsReducer(prev, { type: "SAVE_ERROR" });
		expect(state.appDirty).toBe(true);
	});
});

// --- hooksReducer ---

describe("hooksReducer", () => {
	const initial = {
		config: "",
		loading: false,
		applying: false,
		status: "not_configured" as const,
		copied: false,
		error: null as string | null,
		success: false,
	};

	it("LOAD_START sets loading=true and clears error and success", () => {
		const prev = { ...initial, error: "old", success: true };
		const state = hooksReducer(prev, { type: "LOAD_START" });
		expect(state.loading).toBe(true);
		expect(state.error).toBeNull();
		expect(state.success).toBe(false);
	});

	it("LOAD_SUCCESS sets config and status, clears loading", () => {
		const prev = { ...initial, loading: true };
		const state = hooksReducer(prev, {
			type: "LOAD_SUCCESS",
			config: '{"hooks":[]}',
			status: "active",
		});
		expect(state.loading).toBe(false);
		expect(state.config).toBe('{"hooks":[]}');
		expect(state.status).toBe("active");
	});

	it("LOAD_ERROR sets error and clears loading", () => {
		const prev = { ...initial, loading: true };
		const state = hooksReducer(prev, {
			type: "LOAD_ERROR",
			error: "load failed",
		});
		expect(state.loading).toBe(false);
		expect(state.error).toBe("load failed");
	});

	it("APPLY_START sets applying=true and clears error", () => {
		const prev = { ...initial, error: "old" };
		const state = hooksReducer(prev, { type: "APPLY_START" });
		expect(state.applying).toBe(true);
		expect(state.error).toBeNull();
	});

	it("APPLY_SUCCESS sets status=active and success=true", () => {
		const prev = { ...initial, applying: true };
		const state = hooksReducer(prev, { type: "APPLY_SUCCESS" });
		expect(state.applying).toBe(false);
		expect(state.status).toBe("active");
		expect(state.success).toBe(true);
	});

	it("APPLY_ERROR sets error and clears applying", () => {
		const prev = { ...initial, applying: true };
		const state = hooksReducer(prev, {
			type: "APPLY_ERROR",
			error: "apply failed",
		});
		expect(state.applying).toBe(false);
		expect(state.error).toBe("apply failed");
	});

	it("SET_COPIED updates copied flag", () => {
		const state = hooksReducer(initial, {
			type: "SET_COPIED",
			copied: true,
		});
		expect(state.copied).toBe(true);

		const state2 = hooksReducer(state, {
			type: "SET_COPIED",
			copied: false,
		});
		expect(state2.copied).toBe(false);
	});

	it("COPY_ERROR sets error", () => {
		const state = hooksReducer(initial, {
			type: "COPY_ERROR",
			error: "clipboard denied",
		});
		expect(state.error).toBe("clipboard denied");
	});
});
