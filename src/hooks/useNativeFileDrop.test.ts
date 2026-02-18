import { beforeEach, describe, expect, it, vi } from "vitest";
import { hasVisibleZones, resolveZone } from "./useNativeFileDrop";

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn(() => Promise.resolve(() => {})),
}));

describe("resolveZone", () => {
	let zones: Map<"editor", HTMLElement>;

	beforeEach(() => {
		zones = new Map();
	});

	it("should return null when no zones registered", () => {
		const node = document.createElement("div");
		expect(resolveZone(zones, node)).toBeNull();
	});

	it("should return null when target is null", () => {
		expect(resolveZone(zones, null)).toBeNull();
	});

	it("should return 'editor' when target is inside editor zone", () => {
		const editor = document.createElement("div");
		const child = document.createElement("span");
		editor.appendChild(child);
		zones.set("editor", editor);

		expect(resolveZone(zones, child)).toBe("editor");
	});

	it("should return 'editor' when target is the editor element itself", () => {
		const editor = document.createElement("div");
		zones.set("editor", editor);

		expect(resolveZone(zones, editor)).toBe("editor");
	});

	it("should return null when target is outside all zones", () => {
		const editor = document.createElement("div");
		const unrelated = document.createElement("div");
		zones.set("editor", editor);

		expect(resolveZone(zones, unrelated)).toBeNull();
	});

	it("should return null when target is not a Node", () => {
		expect(resolveZone(zones, {} as EventTarget)).toBeNull();
	});
});

describe("hasVisibleZones", () => {
	let zones: Map<"editor", HTMLElement>;

	beforeEach(() => {
		zones = new Map();
	});

	it("should return false when no zones registered", () => {
		expect(hasVisibleZones(zones)).toBe(false);
	});

	it("should return false when all zones have zero size (display:none)", () => {
		const el = document.createElement("div");
		// jsdomのgetBoundingClientRectはデフォルトで全て0を返す
		zones.set("editor", el);
		expect(hasVisibleZones(zones)).toBe(false);
	});

	it("should return true when a zone has non-zero size", () => {
		const el = document.createElement("div");
		vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
			x: 0,
			y: 0,
			width: 300,
			height: 600,
			left: 0,
			right: 300,
			top: 0,
			bottom: 600,
			toJSON: () => {},
		});
		zones.set("editor", el);
		expect(hasVisibleZones(zones)).toBe(true);
	});
});
