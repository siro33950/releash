import { describe, expect, it } from "vitest";
import { type AppSettings, DEFAULT_SETTINGS } from "@/types/settings";
import { hooksReducer, settingsReducer } from "./SettingsModal";

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

	it("SAVE_ERROR sets appDirty back to true and clears saving", () => {
		const prev = { ...initial, saving: true, appDirty: false };
		const state = settingsReducer(prev, { type: "SAVE_ERROR" });
		expect(state.saving).toBe(false);
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

	it("LOAD_SUCCESS sets config and status, clears loading and error", () => {
		const prev = { ...initial, loading: true, error: "old error" };
		const state = hooksReducer(prev, {
			type: "LOAD_SUCCESS",
			config: '{"hooks":[]}',
			status: "active",
		});
		expect(state.loading).toBe(false);
		expect(state.error).toBeNull();
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

	it("APPLY_START sets applying=true and clears error and success", () => {
		const prev = { ...initial, error: "old", success: true };
		const state = hooksReducer(prev, { type: "APPLY_START" });
		expect(state.applying).toBe(true);
		expect(state.error).toBeNull();
		expect(state.success).toBe(false);
	});

	it("APPLY_SUCCESS sets status=active and success=true, clears error", () => {
		const prev = { ...initial, applying: true, error: "old error" };
		const state = hooksReducer(prev, { type: "APPLY_SUCCESS" });
		expect(state.applying).toBe(false);
		expect(state.status).toBe("active");
		expect(state.success).toBe(true);
		expect(state.error).toBeNull();
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
