import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MentionPopup } from "./MentionPopup";

const files = ["src/main.rs", "src/lib.rs", "src/components/Button.tsx"];

describe("MentionPopup", () => {
	it("renders file list when open", () => {
		render(
			<MentionPopup
				open={true}
				files={files}
				selectedIndex={0}
				onSelect={vi.fn()}
				onClose={vi.fn()}
			>
				<div>anchor</div>
			</MentionPopup>,
		);
		expect(screen.getByTestId("mention-file-list")).toBeDefined();
		expect(screen.getAllByRole("option")).toHaveLength(3);
	});

	it("does not render when closed", () => {
		render(
			<MentionPopup
				open={false}
				files={files}
				selectedIndex={0}
				onSelect={vi.fn()}
				onClose={vi.fn()}
			>
				<div>anchor</div>
			</MentionPopup>,
		);
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("does not render when files is empty", () => {
		render(
			<MentionPopup
				open={true}
				files={[]}
				selectedIndex={0}
				onSelect={vi.fn()}
				onClose={vi.fn()}
			>
				<div>anchor</div>
			</MentionPopup>,
		);
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("highlights the selected item", () => {
		render(
			<MentionPopup
				open={true}
				files={files}
				selectedIndex={1}
				onSelect={vi.fn()}
				onClose={vi.fn()}
			>
				<div>anchor</div>
			</MentionPopup>,
		);
		const options = screen.getAllByRole("option");
		expect(options[0].dataset.selected).toBe("false");
		expect(options[1].dataset.selected).toBe("true");
		expect(options[2].dataset.selected).toBe("false");
	});

	it("calls onSelect when an item is clicked", () => {
		const onSelect = vi.fn();
		render(
			<MentionPopup
				open={true}
				files={files}
				selectedIndex={0}
				onSelect={onSelect}
				onClose={vi.fn()}
			>
				<div>anchor</div>
			</MentionPopup>,
		);
		const options = screen.getAllByRole("option");
		fireEvent.mouseDown(options[1]);
		expect(onSelect).toHaveBeenCalledWith("src/lib.rs");
	});
});
