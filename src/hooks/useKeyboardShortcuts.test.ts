import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";

describe("useKeyboardShortcuts", () => {
	it("should be a no-op (shortcuts handled by native menu accelerators)", () => {
		const onSave = vi.fn();
		renderHook(() => useKeyboardShortcuts({ onSave }));

		const event = new KeyboardEvent("keydown", {
			key: "s",
			metaKey: true,
			bubbles: true,
		});
		window.dispatchEvent(event);

		expect(onSave).not.toHaveBeenCalled();
	});

	it("should accept options without error", () => {
		expect(() => {
			renderHook(() =>
				useKeyboardShortcuts({ onSave: vi.fn(), onSearch: vi.fn() }),
			);
		}).not.toThrow();
	});
});
