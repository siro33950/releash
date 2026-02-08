import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { FileNode } from "@/types/file-tree";
import type { FileTreeProps } from "./FileTree";
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

function defaultProps(overrides: Partial<FileTreeProps> = {}): FileTreeProps {
	return {
		rootPath: "/project",
		tree: mockTree,
		selectedPath: null,
		expandedPaths: new Set(),
		onSelect: vi.fn(),
		onToggleExpand: vi.fn(),
		clipboard: null,
		creatingNode: null,
		renamingPath: null,
		onContextNewFile: vi.fn(),
		onContextNewFolder: vi.fn(),
		onContextCut: vi.fn(),
		onContextCopy: vi.fn(),
		onContextPaste: vi.fn(),
		onContextCopyPath: vi.fn(),
		onContextCopyRelativePath: vi.fn(),
		onContextRename: vi.fn(),
		onContextDelete: vi.fn(),
		onContextRevealInFinder: vi.fn(),
		onCreateCommit: vi.fn(),
		onCreateCancel: vi.fn(),
		onRenameCommit: vi.fn(),
		onRenameCancel: vi.fn(),
		...overrides,
	};
}

describe("FileTree", () => {
	it("should render tree structure", () => {
		render(<FileTree {...defaultProps()} />);

		expect(screen.getByText("src")).toBeInTheDocument();
		expect(screen.getByText("package.json")).toBeInTheDocument();
	});

	it("should show children when folder is expanded", () => {
		render(
			<FileTree
				{...defaultProps({ expandedPaths: new Set(["/project/src"]) })}
			/>,
		);

		expect(screen.getByText("index.ts")).toBeInTheDocument();
		expect(screen.getByText("utils")).toBeInTheDocument();
	});

	it("should not show children when folder is collapsed", () => {
		render(<FileTree {...defaultProps()} />);

		expect(screen.queryByText("index.ts")).not.toBeInTheDocument();
	});

	it("should call onToggleExpand when folder is clicked", async () => {
		const user = userEvent.setup();
		const onToggleExpand = vi.fn();

		render(<FileTree {...defaultProps({ onToggleExpand })} />);

		await user.click(screen.getByText("src"));

		expect(onToggleExpand).toHaveBeenCalledWith("/project/src");
	});

	it("should call onSelect when file is clicked", async () => {
		const user = userEvent.setup();
		const onSelect = vi.fn();

		render(
			<FileTree
				{...defaultProps({
					expandedPaths: new Set(["/project/src"]),
					onSelect,
				})}
			/>,
		);

		await user.click(screen.getByText("index.ts"));

		expect(onSelect).toHaveBeenCalledWith("/project/src/index.ts");
	});

	it("should highlight selected file", () => {
		render(
			<FileTree
				{...defaultProps({
					selectedPath: "/project/src/index.ts",
					expandedPaths: new Set(["/project/src"]),
				})}
			/>,
		);

		const selectedButton = screen.getByText("index.ts").closest("button");
		expect(selectedButton).toHaveClass("bg-sidebar-accent");
	});

	it("should show status indicator for modified files", () => {
		render(<FileTree {...defaultProps()} />);

		expect(screen.getByText("M")).toBeInTheDocument();
	});

	it("should apply opacity-50 to ignored files", () => {
		const ignoredTree: FileNode[] = [
			{
				name: "node_modules",
				path: "/project/node_modules",
				type: "folder",
				status: "ignored",
				children: [
					{
						name: "pkg",
						path: "/project/node_modules/pkg",
						type: "file",
						status: "ignored",
					},
				],
			},
		];
		render(
			<FileTree
				{...defaultProps({
					tree: ignoredTree,
					expandedPaths: new Set(["/project/node_modules"]),
				})}
			/>,
		);

		const folderButton = screen.getByText("node_modules").closest("button");
		expect(folderButton).toHaveClass("opacity-50");

		const fileButton = screen.getByText("pkg").closest("button");
		expect(fileButton).toHaveClass("opacity-50");
	});

	it("should not show status indicator for ignored files", () => {
		const ignoredTree: FileNode[] = [
			{
				name: "ignored.log",
				path: "/project/ignored.log",
				type: "file",
				status: "ignored",
			},
			{
				name: "modified.ts",
				path: "/project/modified.ts",
				type: "file",
				status: "modified",
			},
		];
		render(<FileTree {...defaultProps({ tree: ignoredTree })} />);

		expect(screen.getByText("M")).toBeInTheDocument();
		const ignoredButton = screen.getByText("ignored.log").closest("button");
		expect(ignoredButton?.querySelector(".text-xs.font-mono")).toBeNull();
	});

	it("should show InlineInput at root level when creatingNode.parentPath matches rootPath", () => {
		const onCreateCommit = vi.fn();
		render(
			<FileTree
				{...defaultProps({
					creatingNode: { parentPath: "/project", type: "file" },
					onCreateCommit,
				})}
			/>,
		);

		expect(screen.getByRole("textbox")).toBeInTheDocument();
	});

	it("should show folder icon for root level folder creation", () => {
		render(
			<FileTree
				{...defaultProps({
					creatingNode: { parentPath: "/project", type: "folder" },
				})}
			/>,
		);

		expect(screen.getByRole("textbox")).toBeInTheDocument();
		const icons = screen.getAllByTestId("folder-icon");
		const emptyFolderIcon = icons.find(
			(el) => el.getAttribute("data-foldername") === "",
		);
		expect(emptyFolderIcon).toBeInTheDocument();
	});

	it("should render nested expanded folders", () => {
		render(
			<FileTree
				{...defaultProps({
					expandedPaths: new Set(["/project/src", "/project/src/utils"]),
				})}
			/>,
		);

		expect(screen.getByText("helper.ts")).toBeInTheDocument();
	});
});
