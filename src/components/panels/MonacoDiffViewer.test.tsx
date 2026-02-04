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

		expect(container.firstChild).toHaveClass(
			"h-full",
			"w-full",
			"bg-background",
		);
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

		expect(container.querySelector(".h-full.w-full")).toBeInTheDocument();
	});

	it("should render gutter mode when diffMode is gutter", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				diffMode="gutter"
			/>,
		);

		expect(container.querySelector(".h-full.w-full")).toBeInTheDocument();
	});

	it("should render inline mode when diffMode is inline", () => {
		const { container } = render(
			<MonacoDiffViewer
				originalContent="original"
				modifiedContent="modified"
				diffMode="inline"
			/>,
		);

		expect(container.querySelector(".h-full.w-full")).toBeInTheDocument();
	});
});
