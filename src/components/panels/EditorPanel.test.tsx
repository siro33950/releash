import { invoke } from "@tauri-apps/api/core";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, type Mock, vi } from "vitest";
import type { TabInfo } from "@/types/editor";
import { EditorPanel } from "./EditorPanel";

vi.mock("react-resizable-panels", () => ({
	Group: ({ children, ...props }: { children: ReactNode }) => (
		<div data-testid="resizable-group" {...props}>
			{children}
		</div>
	),
	Panel: ({ children, ...props }: { children: ReactNode }) => (
		<div data-testid="resizable-panel" {...props}>
			{children}
		</div>
	),
	Separator: (props: Record<string, unknown>) => (
		<div data-testid="resizable-separator" {...props} />
	),
}));

vi.mocked(invoke as Mock).mockResolvedValue([]);

const mockTab: TabInfo = {
	path: "/test/file.ts",
	name: "file.ts",
	content: "const x = 1;",
	originalContent: "const x = 1;",
	isDirty: false,
	language: "typescript",
	eol: "LF",
};

describe("EditorPanel", () => {
	it("should render EmptyState and ReviewPanel when no tabs", () => {
		render(
			<EditorPanel
				tabs={[]}
				activeTab={null}
				onTabClick={vi.fn()}
				onTabClose={vi.fn()}
				diffBase="HEAD"
				diffMode="split"
				onDiffBaseChange={vi.fn()}
				onDiffModeChange={vi.fn()}
			/>,
		);

		expect(screen.getByText("No file selected")).toBeInTheDocument();
		expect(screen.getByText("Comments")).toBeInTheDocument();
	});

	it("should render tabs when tabs exist", () => {
		render(
			<EditorPanel
				tabs={[mockTab]}
				activeTab={mockTab}
				onTabClick={vi.fn()}
				onTabClose={vi.fn()}
				diffBase="HEAD"
				diffMode="split"
				onDiffBaseChange={vi.fn()}
				onDiffModeChange={vi.fn()}
			/>,
		);

		expect(screen.getByText("file.ts")).toBeInTheDocument();
	});

	it("should call onTabClick when tab is clicked", async () => {
		const user = userEvent.setup();
		const onTabClick = vi.fn();

		render(
			<EditorPanel
				tabs={[mockTab]}
				activeTab={mockTab}
				onTabClick={onTabClick}
				onTabClose={vi.fn()}
				diffBase="HEAD"
				diffMode="split"
				onDiffBaseChange={vi.fn()}
				onDiffModeChange={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("file.ts"));
		expect(onTabClick).toHaveBeenCalledWith("/test/file.ts");
	});

	it("should call onTabClose when close button is clicked", async () => {
		const user = userEvent.setup();
		const onTabClose = vi.fn();

		render(
			<EditorPanel
				tabs={[mockTab]}
				activeTab={mockTab}
				onTabClick={vi.fn()}
				onTabClose={onTabClose}
				diffBase="HEAD"
				diffMode="split"
				onDiffBaseChange={vi.fn()}
				onDiffModeChange={vi.fn()}
			/>,
		);

		await user.click(screen.getByLabelText("Close file.ts"));
		expect(onTabClose).toHaveBeenCalledWith("/test/file.ts");
	});
});
