import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DiffRenderer } from "./DiffRenderer";

const original = "line1\nline2\nline3\nline4\nline5\n";
const modified = "line1\nline2\nadded line\nline3\nline5\n";

const defaultProps = {
	filePath: "file.txt",
	selectionStart: null as number | null,
	highlightRange: null as { start: number; end: number } | null,
	onLineTap: vi.fn(),
	onLineLongPress: vi.fn(),
};

describe("DiffRenderer", () => {
	it("should show 'No changes' when original and modified are identical", () => {
		render(
			<DiffRenderer
				original={"same\n"}
				modified={"same\n"}
				{...defaultProps}
			/>,
		);
		expect(screen.getByText("No changes")).toBeInTheDocument();
	});

	it("should render added lines with green background class", () => {
		const { container } = render(
			<DiffRenderer
				original={original}
				modified={modified}
				{...defaultProps}
			/>,
		);
		const addedRows = container.querySelectorAll("tr.bg-green-950\\/40");
		expect(addedRows.length).toBeGreaterThan(0);
	});

	it("should render deleted lines with red background class", () => {
		const { container } = render(
			<DiffRenderer
				original={original}
				modified={modified}
				{...defaultProps}
			/>,
		);
		const deletedRows = container.querySelectorAll("tr.bg-red-950\\/40");
		expect(deletedRows.length).toBeGreaterThan(0);
	});

	it("should show correct line numbers for added and deleted lines", () => {
		const { container } = render(
			<DiffRenderer
				original={"a\nb\nc\n"}
				modified={"a\nx\nc\n"}
				{...defaultProps}
			/>,
		);
		const rows = container.querySelectorAll("tbody tr");
		// row 0: hunk header
		// row 1: context "a" → old=1, new=1
		// row 2: deleted "-b" → old=2, new=""
		// row 3: added "+x" → old="", new=2
		// row 4: context "c" → old=3, new=3

		const cells1 = rows[1].querySelectorAll("td");
		expect(cells1[0].textContent).toBe("1");
		expect(cells1[1].textContent).toBe("1");

		const cells2 = rows[2].querySelectorAll("td");
		expect(cells2[0].textContent).toBe("2");
		expect(cells2[1].textContent).toBe("");

		const cells3 = rows[3].querySelectorAll("td");
		expect(cells3[0].textContent).toBe("");
		expect(cells3[1].textContent).toBe("2");

		const cells4 = rows[4].querySelectorAll("td");
		expect(cells4[0].textContent).toBe("3");
		expect(cells4[1].textContent).toBe("3");
	});

	it("should render hunk separator header", () => {
		render(
			<DiffRenderer
				original={original}
				modified={modified}
				{...defaultProps}
			/>,
		);
		const header = screen.getByText(/^@@.*@@$/);
		expect(header).toBeInTheDocument();
	});

	it("should call onLineTap on short pointer interaction", () => {
		const onLineTap = vi.fn();
		const { container } = render(
			<DiffRenderer
				original={"a\nb\n"}
				modified={"a\nx\n"}
				{...defaultProps}
				onLineTap={onLineTap}
			/>,
		);
		const rows = container.querySelectorAll("tbody tr");
		// row 1: context "a" (newLine=1)
		fireEvent.pointerDown(rows[1]);
		fireEvent.pointerUp(rows[1]);
		expect(onLineTap).toHaveBeenCalledWith(1);
	});

	it("should call onLineLongPress after holding pointer", () => {
		vi.useFakeTimers();
		const onLineLongPress = vi.fn();
		const { container } = render(
			<DiffRenderer
				original={"a\nb\n"}
				modified={"a\nx\n"}
				{...defaultProps}
				onLineLongPress={onLineLongPress}
			/>,
		);
		const rows = container.querySelectorAll("tbody tr");
		fireEvent.pointerDown(rows[1]);
		vi.advanceTimersByTime(500);
		expect(onLineLongPress).toHaveBeenCalledWith(1);
		fireEvent.pointerUp(rows[1]);
		vi.useRealTimers();
	});

	it("should not call onLineTap after long press", () => {
		vi.useFakeTimers();
		const onLineTap = vi.fn();
		const onLineLongPress = vi.fn();
		const { container } = render(
			<DiffRenderer
				original={"a\nb\n"}
				modified={"a\nx\n"}
				{...defaultProps}
				onLineTap={onLineTap}
				onLineLongPress={onLineLongPress}
			/>,
		);
		const rows = container.querySelectorAll("tbody tr");
		fireEvent.pointerDown(rows[1]);
		vi.advanceTimersByTime(500);
		fireEvent.pointerUp(rows[1]);
		expect(onLineLongPress).toHaveBeenCalledTimes(1);
		expect(onLineTap).not.toHaveBeenCalled();
		vi.useRealTimers();
	});

	it("should highlight selectionStart row with amber ring", () => {
		const { container } = render(
			<DiffRenderer
				original={"a\nb\n"}
				modified={"a\nx\n"}
				{...defaultProps}
				selectionStart={1}
			/>,
		);
		const highlighted = container.querySelectorAll("tr.ring-amber-500");
		expect(highlighted.length).toBe(1);
	});

	it("should highlight rows in highlightRange with blue ring", () => {
		const { container } = render(
			<DiffRenderer
				original={"a\nb\nc\nd\n"}
				modified={"a\nx\ny\nd\n"}
				{...defaultProps}
				highlightRange={{ start: 2, end: 3 }}
			/>,
		);
		const highlighted = container.querySelectorAll("tr.ring-blue-500");
		expect(highlighted.length).toBe(2);
	});
});
