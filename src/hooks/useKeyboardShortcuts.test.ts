import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";

describe("useKeyboardShortcuts", () => {
	it("should call onSave when Cmd+S is pressed", () => {
		const onSave = vi.fn();
		renderHook(() => useKeyboardShortcuts({ onSave }));

		const event = new KeyboardEvent("keydown", {
			key: "s",
			metaKey: true,
			bubbles: true,
		});
		window.dispatchEvent(event);

		expect(onSave).toHaveBeenCalledOnce();
	});

	it("should call onSave when Ctrl+S is pressed", () => {
		const onSave = vi.fn();
		renderHook(() => useKeyboardShortcuts({ onSave }));

		const event = new KeyboardEvent("keydown", {
			key: "s",
			ctrlKey: true,
			bubbles: true,
		});
		window.dispatchEvent(event);

		expect(onSave).toHaveBeenCalledOnce();
	});

	it("should not call onSave when S is pressed without modifier", () => {
		const onSave = vi.fn();
		renderHook(() => useKeyboardShortcuts({ onSave }));

		const event = new KeyboardEvent("keydown", {
			key: "s",
			bubbles: true,
		});
		window.dispatchEvent(event);

		expect(onSave).not.toHaveBeenCalled();
	});

	it("should not call onSave when onSave is undefined", () => {
		renderHook(() => useKeyboardShortcuts({}));

		const event = new KeyboardEvent("keydown", {
			key: "s",
			metaKey: true,
			bubbles: true,
		});

		expect(() => window.dispatchEvent(event)).not.toThrow();
	});
});
