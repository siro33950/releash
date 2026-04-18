import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import { MentionPopup } from "./MentionPopup";

const files = ["src/main.rs", "src/lib.rs", "src/components/Button.tsx"];

function renderMentionPopup(
	props: Partial<React.ComponentProps<typeof MentionPopup>> = {},
) {
	const anchorRef = createRef<HTMLDivElement>();
	return render(
		<>
			<div ref={anchorRef}>anchor</div>
			<MentionPopup
				open={true}
				files={files}
				selectedIndex={0}
				onSelect={vi.fn()}
				onClose={vi.fn()}
				anchorRef={anchorRef}
				{...props}
			/>
		</>,
	);
}

describe("MentionPopup", () => {
	it("renders file list when open", () => {
		renderMentionPopup();
		expect(screen.getByTestId("mention-file-list")).toBeDefined();
		expect(screen.getAllByRole("option")).toHaveLength(3);
	});

	it("does not render when closed", () => {
		renderMentionPopup({ open: false });
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("does not render when files is empty", () => {
		renderMentionPopup({ files: [] });
		expect(screen.queryByTestId("mention-file-list")).toBeNull();
	});

	it("highlights the selected item", () => {
		renderMentionPopup({ selectedIndex: 1 });
		const options = screen.getAllByRole("option");
		expect(options[0].dataset.selected).toBe("false");
		expect(options[1].dataset.selected).toBe("true");
		expect(options[2].dataset.selected).toBe("false");
	});

	it("calls onSelect when an item is clicked", () => {
		const onSelect = vi.fn();
		renderMentionPopup({ onSelect });
		const options = screen.getAllByRole("option");
		fireEvent.mouseDown(options[1]);
		expect(onSelect).toHaveBeenCalledWith("src/lib.rs");
	});
});
