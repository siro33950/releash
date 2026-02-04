import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EditorPanel } from "./EditorPanel";

describe("EditorPanel", () => {
	it("should render container element", () => {
		render(<EditorPanel />);

		const container = document.querySelector(".h-full.w-full");
		expect(container).toBeInTheDocument();
	});

	it("should have full height and width", () => {
		const { container } = render(<EditorPanel />);

		const editorContainer = container.firstChild as HTMLElement;
		expect(editorContainer).toHaveClass("h-full", "w-full");
	});
});
