import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DiffSearchBar } from "./DiffSearchBar";

const baseProps = {
	query: "",
	onQueryChange: vi.fn(),
	currentIndex: 0,
	totalMatches: 0,
	onNext: vi.fn(),
	onPrev: vi.fn(),
	onClose: vi.fn(),
};

describe("DiffSearchBar", () => {
	it("renders search input", () => {
		render(<DiffSearchBar {...baseProps} />);
		expect(screen.getByTestId("diff-search-input")).toBeDefined();
	});

	it("focuses input on mount", () => {
		render(<DiffSearchBar {...baseProps} />);
		expect(document.activeElement).toBe(
			screen.getByTestId("diff-search-input"),
		);
	});

	it("calls onQueryChange when typing", () => {
		const onQueryChange = vi.fn();
		render(<DiffSearchBar {...baseProps} onQueryChange={onQueryChange} />);
		fireEvent.change(screen.getByTestId("diff-search-input"), {
			target: { value: "test" },
		});
		expect(onQueryChange).toHaveBeenCalledWith("test");
	});

	it("shows match count in 'current/total' format", () => {
		render(
			<DiffSearchBar
				{...baseProps}
				query="hello"
				currentIndex={0}
				totalMatches={5}
			/>,
		);
		expect(screen.getByTestId("diff-search-count").textContent).toBe("1/5");
	});

	it("shows '0' when query has no matches", () => {
		render(
			<DiffSearchBar
				{...baseProps}
				query="xyz"
				currentIndex={-1}
				totalMatches={0}
			/>,
		);
		expect(screen.getByTestId("diff-search-count").textContent).toBe("0");
	});

	it("does not show count when query is empty", () => {
		render(<DiffSearchBar {...baseProps} query="" totalMatches={0} />);
		expect(screen.queryByTestId("diff-search-count")).toBeNull();
	});

	it("calls onClose on Escape", () => {
		const onClose = vi.fn();
		render(<DiffSearchBar {...baseProps} onClose={onClose} />);
		fireEvent.keyDown(screen.getByTestId("diff-search-input"), {
			key: "Escape",
		});
		expect(onClose).toHaveBeenCalled();
	});

	it("calls onNext on Enter", () => {
		const onNext = vi.fn();
		render(
			<DiffSearchBar
				{...baseProps}
				query="hello"
				totalMatches={3}
				onNext={onNext}
			/>,
		);
		fireEvent.keyDown(screen.getByTestId("diff-search-input"), {
			key: "Enter",
		});
		expect(onNext).toHaveBeenCalled();
	});

	it("calls onPrev on Shift+Enter", () => {
		const onPrev = vi.fn();
		render(
			<DiffSearchBar
				{...baseProps}
				query="hello"
				totalMatches={3}
				onPrev={onPrev}
			/>,
		);
		fireEvent.keyDown(screen.getByTestId("diff-search-input"), {
			key: "Enter",
			shiftKey: true,
		});
		expect(onPrev).toHaveBeenCalled();
	});

	it("disables navigation buttons when no matches", () => {
		render(<DiffSearchBar {...baseProps} query="xyz" totalMatches={0} />);
		expect(screen.getByTestId("diff-search-prev")).toHaveProperty(
			"disabled",
			true,
		);
		expect(screen.getByTestId("diff-search-next")).toHaveProperty(
			"disabled",
			true,
		);
	});

	it("calls onClose when close button is clicked", () => {
		const onClose = vi.fn();
		render(<DiffSearchBar {...baseProps} onClose={onClose} />);
		fireEvent.click(screen.getByTestId("diff-search-close"));
		expect(onClose).toHaveBeenCalled();
	});
});
