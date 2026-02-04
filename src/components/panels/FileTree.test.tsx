import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { FileNode } from "@/types/file-tree";
import { FileTree } from "./FileTree";

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
	DefaultFolderOpenedIcon: ({ className }: { className?: string }) => (
		<span data-testid="folder-open-icon" className={className} />
	),
}));

const mockTree: FileNode[] = [
	{
		name: "src",
		path: "/project/src",
		type: "folder",
		children: [
			{ name: "index.ts", path: "/project/src/index.ts", type: "file" },
			{
				name: "utils",
				path: "/project/src/utils",
				type: "folder",
				children: [
					{
						name: "helper.ts",
						path: "/project/src/utils/helper.ts",
						type: "file",
					},
				],
			},
		],
	},
	{
		name: "package.json",
		path: "/project/package.json",
		type: "file",
		status: "modified",
	},
];

describe("FileTree", () => {
	it("should render tree structure", () => {
		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set()}
				onSelect={vi.fn()}
				onToggleExpand={vi.fn()}
			/>,
		);

		expect(screen.getByText("src")).toBeInTheDocument();
		expect(screen.getByText("package.json")).toBeInTheDocument();
	});

	it("should show children when folder is expanded", () => {
		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set(["/project/src"])}
				onSelect={vi.fn()}
				onToggleExpand={vi.fn()}
			/>,
		);

		expect(screen.getByText("index.ts")).toBeInTheDocument();
		expect(screen.getByText("utils")).toBeInTheDocument();
	});

	it("should not show children when folder is collapsed", () => {
		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set()}
				onSelect={vi.fn()}
				onToggleExpand={vi.fn()}
			/>,
		);

		expect(screen.queryByText("index.ts")).not.toBeInTheDocument();
	});

	it("should call onToggleExpand when folder is clicked", async () => {
		const user = userEvent.setup();
		const onToggleExpand = vi.fn();

		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set()}
				onSelect={vi.fn()}
				onToggleExpand={onToggleExpand}
			/>,
		);

		await user.click(screen.getByText("src"));

		expect(onToggleExpand).toHaveBeenCalledWith("/project/src");
	});

	it("should call onSelect when file is clicked", async () => {
		const user = userEvent.setup();
		const onSelect = vi.fn();

		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set(["/project/src"])}
				onSelect={onSelect}
				onToggleExpand={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("index.ts"));

		expect(onSelect).toHaveBeenCalledWith("/project/src/index.ts");
	});

	it("should highlight selected file", () => {
		render(
			<FileTree
				tree={mockTree}
				selectedPath="/project/src/index.ts"
				expandedPaths={new Set(["/project/src"])}
				onSelect={vi.fn()}
				onToggleExpand={vi.fn()}
			/>,
		);

		const selectedButton = screen.getByText("index.ts").closest("button");
		expect(selectedButton).toHaveClass("bg-sidebar-accent");
	});

	it("should show status indicator for modified files", () => {
		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set()}
				onSelect={vi.fn()}
				onToggleExpand={vi.fn()}
			/>,
		);

		expect(screen.getByText("M")).toBeInTheDocument();
	});

	it("should render nested expanded folders", () => {
		render(
			<FileTree
				tree={mockTree}
				selectedPath={null}
				expandedPaths={new Set(["/project/src", "/project/src/utils"])}
				onSelect={vi.fn()}
				onToggleExpand={vi.fn()}
			/>,
		);

		expect(screen.getByText("helper.ts")).toBeInTheDocument();
	});
});
