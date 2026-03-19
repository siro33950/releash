import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ThinkingIndicator } from "./ThinkingIndicator";

describe("ThinkingIndicator", () => {
	describe("spinner mode (no content)", () => {
		it("renders 'Thinking...' text with spinner when isStreaming", () => {
			render(<ThinkingIndicator isStreaming />);
			expect(screen.getByText("Thinking...")).toBeDefined();
		});

		it("renders 'Thinking...' text with spinner when no props", () => {
			render(<ThinkingIndicator />);
			expect(screen.getByText("Thinking...")).toBeDefined();
		});

		it("shows spinning loader icon", () => {
			const { container } = render(<ThinkingIndicator isStreaming />);
			const spinner = container.querySelector(".animate-spin");
			expect(spinner).not.toBeNull();
		});

		it("has thinking-indicator testid", () => {
			render(<ThinkingIndicator isStreaming />);
			expect(screen.getByTestId("thinking-indicator")).toBeDefined();
		});
	});

	describe("collapsible mode (with content)", () => {
		it("renders collapsed by default with toggle button", () => {
			render(<ThinkingIndicator content="some thinking" />);
			expect(screen.getByTestId("thinking-toggle")).toBeDefined();
			expect(screen.queryByTestId("thinking-content")).toBeNull();
		});

		it("expands to show content when toggle is clicked", () => {
			render(<ThinkingIndicator content="some thinking" />);
			fireEvent.click(screen.getByTestId("thinking-toggle"));
			expect(screen.getByTestId("thinking-content")).toBeDefined();
			expect(screen.getByText("some thinking")).toBeDefined();
		});

		it("collapses when toggle is clicked again", () => {
			render(<ThinkingIndicator content="some thinking" />);
			fireEvent.click(screen.getByTestId("thinking-toggle"));
			expect(screen.getByTestId("thinking-content")).toBeDefined();
			fireEvent.click(screen.getByTestId("thinking-toggle"));
			expect(screen.queryByTestId("thinking-content")).toBeNull();
		});

		it("shows spinner when isStreaming is true", () => {
			const { container } = render(
				<ThinkingIndicator content="thinking..." isStreaming />,
			);
			const spinner = container.querySelector(".animate-spin");
			expect(spinner).not.toBeNull();
		});

		it("does not show spinner when isStreaming is false", () => {
			const { container } = render(
				<ThinkingIndicator content="done thinking" />,
			);
			const spinner = container.querySelector(".animate-spin");
			expect(spinner).toBeNull();
		});

		it("has thinking-indicator testid", () => {
			render(<ThinkingIndicator content="some thinking" />);
			expect(screen.getByTestId("thinking-indicator")).toBeDefined();
		});
	});
});
