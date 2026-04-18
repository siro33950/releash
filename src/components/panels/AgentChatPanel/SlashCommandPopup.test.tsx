import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";
import type { SlashCommand } from "@/hooks/useSlashCommands";
import { SlashCommandPopup } from "./SlashCommandPopup";

const commands: SlashCommand[] = [
	{ name: "plan-spec", description: "Create plan spec" },
	{ name: "review", description: "Code review", argumentHint: "<file>" },
	{ name: "commit", description: "Create a commit" },
];

function renderPopup(
	props: Partial<React.ComponentProps<typeof SlashCommandPopup>> = {},
) {
	const anchorRef = createRef<HTMLDivElement>();
	const result = render(
		<>
			<div ref={anchorRef} data-testid="anchor">
				Anchor
			</div>
			<SlashCommandPopup
				open={true}
				commands={commands}
				selectedIndex={0}
				onSelect={vi.fn()}
				onClose={vi.fn()}
				anchorRef={anchorRef}
				{...props}
			/>
		</>,
	);
	return result;
}

describe("SlashCommandPopup", () => {
	it("renders command list when open with commands", () => {
		renderPopup();
		expect(screen.getByTestId("slash-command-list")).toBeDefined();
		expect(screen.getAllByRole("option")).toHaveLength(3);
	});

	it("displays command name, description, and argument hint", () => {
		renderPopup();
		const options = screen.getAllByRole("option");
		expect(options[0].textContent).toContain("/plan-spec");
		expect(options[0].textContent).toContain("Create plan spec");
		expect(options[1].textContent).toContain("/review");
		expect(options[1].textContent).toContain("<file>");
	});

	it("highlights selected option", () => {
		renderPopup({ selectedIndex: 1 });
		const options = screen.getAllByRole("option");
		expect(options[0].dataset.selected).toBe("false");
		expect(options[1].dataset.selected).toBe("true");
		expect(options[2].dataset.selected).toBe("false");
	});

	it("calls onSelect on mousedown", () => {
		const onSelect = vi.fn();
		renderPopup({ onSelect });
		const options = screen.getAllByRole("option");
		fireEvent.mouseDown(options[2]);
		expect(onSelect).toHaveBeenCalledWith(commands[2]);
	});

	it("does not render listbox when commands is empty", () => {
		renderPopup({ commands: [] });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("does not render listbox when open is false", () => {
		renderPopup({ open: false });
		expect(screen.queryByTestId("slash-command-list")).toBeNull();
	});

	it("renders anchor element", () => {
		renderPopup();
		expect(screen.getByTestId("anchor")).toBeDefined();
	});
});
