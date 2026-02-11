import { invoke } from "@tauri-apps/api/core";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { describe, expect, it, type Mock, vi } from "vitest";
import type { TabInfo } from "@/types/editor";
import { EditorPanel } from "./EditorPanel";

vi.mock("@react-symbols/icons/utils", () => ({
	FileIcon: ({
		fileName,
		className,
	}: {
		fileName: string;
		className?: string;
	}) => (
		<span
			data-testid="file-icon"
			data-filename={fileName}
			className={className}
		/>
	),
	FolderIcon: ({
		folderName,
		className,
	}: {
		folderName: string;
		className?: string;
	}) => (
		<span
			data-testid="folder-icon"
			data-foldername={folderName}
			className={className}
		/>
	),
}));

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

	it("should render Breadcrumb when activeTab exists and rootPath is provided", () => {
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
				rootPath="/test"
			/>,
		);

		const breadcrumb = screen.getByTestId("breadcrumb");
		expect(within(breadcrumb).getByTestId("file-icon")).toHaveAttribute(
			"data-filename",
			"file.ts",
		);
	});

	it("should not render Breadcrumb when activeTab is null", () => {
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
				rootPath="/test"
			/>,
		);

		expect(screen.queryByTestId("breadcrumb")).not.toBeInTheDocument();
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
