import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { DiffTreeNode } from "@/types/review";
import type { DiffBase, DiffSection } from "@/types/settings";
import { DiffFileTree } from "./DiffFileTree";

// Radix UI Tooltip require pointer capture APIs not available in jsdom
if (typeof Element.prototype.hasPointerCapture !== "function") {
	Element.prototype.hasPointerCapture = () => false;
}
if (typeof Element.prototype.setPointerCapture !== "function") {
	Element.prototype.setPointerCapture = () => {};
}
if (typeof Element.prototype.releasePointerCapture !== "function") {
	Element.prototype.releasePointerCapture = () => {};
}

const fileNode = (
	path: string,
	name: string,
	status = "modified",
): DiffTreeNode => ({
	id: `file:${path}`,
	name,
	path,
	node_type: "file",
	status,
	additions: 5,
	deletions: 2,
	children: [],
});

const folderNode = (
	path: string,
	name: string,
	children: DiffTreeNode[],
): DiffTreeNode => ({
	id: `folder:${path}`,
	name,
	path,
	node_type: "folder",
	status: null,
	additions: null,
	deletions: null,
	children,
});

const sampleTree: DiffTreeNode[] = [
	folderNode("src", "src", [
		fileNode("src/app.tsx", "app.tsx"),
		fileNode("src/main.tsx", "main.tsx", "added"),
	]),
	fileNode("README.md", "README.md", "modified"),
];

interface TestProps {
	rootPath: string;
	stagedTree: DiffTreeNode[];
	changesTree: DiffTreeNode[];
	branchBaseTree: DiffTreeNode[];
	stagedFileCount: number;
	changesFileCount: number;
	diffBase: DiffBase;
	selectedFile: string | null;
	selectedSection: DiffSection;
	onSelectFile: (path: string, section: DiffSection) => void;
	onStageFile?: (path: string) => void;
	onUnstageFile?: (path: string) => void;
	onStageAll?: () => void;
	onUnstageAll?: () => void;
}

const defaultProps: TestProps = {
	rootPath: "/workspace/my-project",
	stagedTree: [],
	changesTree: [],
	branchBaseTree: [],
	stagedFileCount: 0,
	changesFileCount: 0,
	diffBase: "head",
	selectedFile: null,
	selectedSection: "changes",
	onSelectFile: vi.fn(),
	onStageFile: vi.fn(),
	onUnstageFile: vi.fn(),
	onStageAll: vi.fn(),
	onUnstageAll: vi.fn(),
};

function renderTree(props: Partial<TestProps> = {}) {
	return render(
		<TooltipProvider>
			<DiffFileTree {...defaultProps} {...props} />
		</TooltipProvider>,
	);
}

describe("DiffFileTree", () => {
	describe("HEAD mode - section display", () => {
		it("should always render both section headers even when counts are 0", () => {
			renderTree();

			expect(screen.getByText("Unstaged")).toBeInTheDocument();
			expect(screen.getByText("Staged")).toBeInTheDocument();
		});

		it("should render Unstaged section header with file count", () => {
			renderTree({
				changesTree: sampleTree,
				changesFileCount: 3,
			});

			expect(screen.getByText("Unstaged")).toBeInTheDocument();
			expect(screen.getByText("(3)")).toBeInTheDocument();
		});

		it("should render Staged section header with file count", () => {
			renderTree({
				stagedTree: [fileNode("file.txt", "file.txt")],
				stagedFileCount: 1,
			});

			expect(screen.getByText("Staged")).toBeInTheDocument();
			expect(screen.getByText("(1)")).toBeInTheDocument();
		});

		it("should render both Unstaged and Staged sections when both have files", () => {
			renderTree({
				changesTree: [fileNode("a.txt", "a.txt")],
				changesFileCount: 1,
				stagedTree: [fileNode("b.txt", "b.txt")],
				stagedFileCount: 1,
			});

			expect(screen.getByText("Unstaged")).toBeInTheDocument();
			expect(screen.getByText("Staged")).toBeInTheDocument();
		});
	});

	describe("branch-base mode - flat tree", () => {
		it("should render files without section headers", () => {
			renderTree({
				diffBase: "branch-base",
				branchBaseTree: [fileNode("file.txt", "file.txt")],
			});

			expect(screen.queryByText("Unstaged")).not.toBeInTheDocument();
			expect(screen.queryByText("Staged")).not.toBeInTheDocument();
			expect(screen.getByText("file.txt")).toBeInTheDocument();
		});

		it("should not show Stage/Unstage action buttons on file nodes", () => {
			renderTree({
				diffBase: "branch-base",
				branchBaseTree: [fileNode("file.txt", "file.txt")],
				onStageFile: undefined,
				onUnstageFile: undefined,
			});

			expect(
				screen.queryByRole("button", { name: "Stage file" }),
			).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "Unstage file" }),
			).not.toBeInTheDocument();
		});
	});

	describe("tree rendering", () => {
		it("should render folder and file nodes", () => {
			renderTree({
				changesTree: sampleTree,
				changesFileCount: 3,
			});

			expect(screen.getByText("src")).toBeInTheDocument();
			expect(screen.getByText("app.tsx")).toBeInTheDocument();
			expect(screen.getByText("main.tsx")).toBeInTheDocument();
			expect(screen.getByText("README.md")).toBeInTheDocument();
		});

		it("should display additions and deletions for file nodes", () => {
			renderTree({
				changesTree: [fileNode("file.txt", "file.txt")],
				changesFileCount: 1,
			});

			expect(screen.getByText("+5")).toBeInTheDocument();
			expect(screen.getByText("-2")).toBeInTheDocument();
		});
	});

	describe("folder toggle", () => {
		it("should collapse folder children when folder is clicked", () => {
			renderTree({
				changesTree: sampleTree,
				changesFileCount: 3,
			});

			// Children should be visible initially (folders expanded by default)
			expect(screen.getByText("app.tsx")).toBeInTheDocument();

			// Click folder to collapse
			fireEvent.click(screen.getByText("src"));

			// Children should be hidden
			expect(screen.queryByText("app.tsx")).not.toBeInTheDocument();
		});

		it("should re-expand folder when clicked again", () => {
			renderTree({
				changesTree: sampleTree,
				changesFileCount: 3,
			});

			// Collapse
			fireEvent.click(screen.getByText("src"));
			expect(screen.queryByText("app.tsx")).not.toBeInTheDocument();

			// Re-expand
			fireEvent.click(screen.getByText("src"));
			expect(screen.getByText("app.tsx")).toBeInTheDocument();
		});
	});

	describe("file selection", () => {
		it("should call onSelectFile with path and section when file is clicked", () => {
			const onSelectFile = vi.fn();
			renderTree({
				changesTree: [fileNode("file.txt", "file.txt")],
				changesFileCount: 1,
				onSelectFile,
			});

			fireEvent.click(screen.getByText("file.txt"));

			expect(onSelectFile).toHaveBeenCalledWith("file.txt", "changes");
		});

		it("should call onSelectFile with staged section for staged files", () => {
			const onSelectFile = vi.fn();
			renderTree({
				stagedTree: [fileNode("file.txt", "file.txt")],
				stagedFileCount: 1,
				onSelectFile,
			});

			fireEvent.click(screen.getByText("file.txt"));

			expect(onSelectFile).toHaveBeenCalledWith("file.txt", "staged");
		});
	});

	describe("Stage All / Unstage All buttons", () => {
		it("should render Stage All button in Unstaged section header", () => {
			renderTree({
				changesTree: [fileNode("file.txt", "file.txt")],
				changesFileCount: 1,
			});

			expect(
				screen.getByRole("button", { name: "Stage All" }),
			).toBeInTheDocument();
		});

		it("should call onStageAll when Stage All is clicked", () => {
			const onStageAll = vi.fn();
			renderTree({
				changesTree: [fileNode("file.txt", "file.txt")],
				changesFileCount: 1,
				onStageAll,
			});

			fireEvent.click(screen.getByRole("button", { name: "Stage All" }));

			expect(onStageAll).toHaveBeenCalledOnce();
		});

		it("should render Unstage All button in Staged section header", () => {
			renderTree({
				stagedTree: [fileNode("file.txt", "file.txt")],
				stagedFileCount: 1,
			});

			expect(
				screen.getByRole("button", { name: "Unstage All" }),
			).toBeInTheDocument();
		});

		it("should call onUnstageAll when Unstage All is clicked", () => {
			const onUnstageAll = vi.fn();
			renderTree({
				stagedTree: [fileNode("file.txt", "file.txt")],
				stagedFileCount: 1,
				onUnstageAll,
			});

			fireEvent.click(screen.getByRole("button", { name: "Unstage All" }));

			expect(onUnstageAll).toHaveBeenCalledOnce();
		});

		it("should not render Stage All / Unstage All in branch-base mode", () => {
			renderTree({
				diffBase: "branch-base",
				branchBaseTree: [fileNode("file.txt", "file.txt")],
			});

			expect(
				screen.queryByRole("button", { name: "Stage All" }),
			).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "Unstage All" }),
			).not.toBeInTheDocument();
		});
	});

	describe("file action buttons", () => {
		it("should call onStageFile when stage icon is clicked on unstaged file", () => {
			const onStageFile = vi.fn();
			renderTree({
				changesTree: [fileNode("file.txt", "file.txt")],
				changesFileCount: 1,
				onStageFile,
			});

			const stageButton = screen.getByRole("button", {
				name: "Stage file",
			});
			fireEvent.click(stageButton);

			expect(onStageFile).toHaveBeenCalledWith("file.txt");
		});

		it("should call onUnstageFile when unstage icon is clicked on staged file", () => {
			const onUnstageFile = vi.fn();
			renderTree({
				stagedTree: [fileNode("file.txt", "file.txt")],
				stagedFileCount: 1,
				onUnstageFile,
			});

			const unstageButton = screen.getByRole("button", {
				name: "Unstage file",
			});
			fireEvent.click(unstageButton);

			expect(onUnstageFile).toHaveBeenCalledWith("file.txt");
		});
	});

	describe("section toggle", () => {
		it("should collapse Unstaged section when header is clicked", () => {
			renderTree({
				changesTree: [fileNode("file.txt", "file.txt")],
				changesFileCount: 1,
			});

			expect(screen.getByText("file.txt")).toBeInTheDocument();

			// Click section header to collapse
			fireEvent.click(screen.getByText("Unstaged"));

			expect(screen.queryByText("file.txt")).not.toBeInTheDocument();
		});
	});

	describe("Expand All / Collapse All in branch-base mode", () => {
		it("should show Expand All and Collapse All when tree has folders", () => {
			renderTree({
				diffBase: "branch-base",
				branchBaseTree: sampleTree,
			});

			expect(
				screen.getByRole("button", { name: "Expand All" }),
			).toBeInTheDocument();
			expect(
				screen.getByRole("button", { name: "Collapse All" }),
			).toBeInTheDocument();
		});

		it("should collapse all folders when Collapse All is clicked", () => {
			renderTree({
				diffBase: "branch-base",
				branchBaseTree: sampleTree,
			});

			expect(screen.getByText("app.tsx")).toBeInTheDocument();

			fireEvent.click(screen.getByRole("button", { name: "Collapse All" }));

			expect(screen.queryByText("app.tsx")).not.toBeInTheDocument();
		});

		it("should re-expand all folders when Expand All is clicked after collapse", () => {
			renderTree({
				diffBase: "branch-base",
				branchBaseTree: sampleTree,
			});

			fireEvent.click(screen.getByRole("button", { name: "Collapse All" }));
			expect(screen.queryByText("app.tsx")).not.toBeInTheDocument();

			fireEvent.click(screen.getByRole("button", { name: "Expand All" }));
			expect(screen.getByText("app.tsx")).toBeInTheDocument();
		});
	});

	describe("context menu - copy path", () => {
		it("should show context menu with copy options when file is right-clicked", async () => {
			renderTree({
				changesTree: [fileNode("src/app.tsx", "app.tsx")],
				changesFileCount: 1,
			});

			const fileButton = screen.getByText("app.tsx");
			fireEvent.contextMenu(fileButton);

			await waitFor(() => {
				expect(screen.getByText("Copy Relative Path")).toBeInTheDocument();
				expect(screen.getByText("Copy Absolute Path")).toBeInTheDocument();
			});
		});

		it("should copy relative path to clipboard when 'Copy Relative Path' is clicked", async () => {
			const writeText = vi.fn().mockResolvedValue(undefined);
			Object.assign(navigator, {
				clipboard: { writeText },
			});

			renderTree({
				changesTree: [fileNode("src/app.tsx", "app.tsx")],
				changesFileCount: 1,
			});

			fireEvent.contextMenu(screen.getByText("app.tsx"));

			await waitFor(() => {
				expect(screen.getByText("Copy Relative Path")).toBeInTheDocument();
			});

			fireEvent.click(screen.getByText("Copy Relative Path"));

			await waitFor(() => {
				expect(writeText).toHaveBeenCalledWith("src/app.tsx");
			});
		});

		it("should copy absolute path to clipboard when 'Copy Absolute Path' is clicked", async () => {
			const writeText = vi.fn().mockResolvedValue(undefined);
			Object.assign(navigator, {
				clipboard: { writeText },
			});

			renderTree({
				rootPath: "/workspace/my-project",
				changesTree: [fileNode("src/app.tsx", "app.tsx")],
				changesFileCount: 1,
			});

			fireEvent.contextMenu(screen.getByText("app.tsx"));

			await waitFor(() => {
				expect(screen.getByText("Copy Absolute Path")).toBeInTheDocument();
			});

			fireEvent.click(screen.getByText("Copy Absolute Path"));

			await waitFor(() => {
				expect(writeText).toHaveBeenCalledWith(
					"/workspace/my-project/src/app.tsx",
				);
			});
		});
	});
});
