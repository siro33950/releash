import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MonacoDiffViewer } from "./MonacoDiffViewer";

describe("MonacoDiffViewer", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should render container element", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		const el = container.firstChild as HTMLElement;
		expect(el).toHaveClass("bg-background");
		expect(el.style.position).toBe("absolute");
		expect(el.style.inset).toBe("0");
	});

	it("should apply custom className", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				className="custom-class"
			/>,
		);

		expect(container.firstChild).toHaveClass("custom-class");
	});

	it("should render split mode by default", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
			/>,
		);

		const editors = container.querySelectorAll<HTMLElement>(
			"[style*='height: 100%']",
		);
		expect(editors.length).toBeGreaterThan(0);
	});

	it("should render gutter mode when diffMode is gutter", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				diffMode="gutter"
			/>,
		);

		const editors = container.querySelectorAll<HTMLElement>(
			"[style*='height: 100%']",
		);
		expect(editors.length).toBeGreaterThan(0);
	});

	it("should render inline mode when diffMode is inline", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				diffMode="inline"
			/>,
		);

		const editors = container.querySelectorAll<HTMLElement>(
			"[style*='height: 100%']",
		);
		expect(editors.length).toBeGreaterThan(0);
	});
});
