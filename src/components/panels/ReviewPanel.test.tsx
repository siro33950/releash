import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ReviewPanel } from "./ReviewPanel";

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

vi.mock("@react-symbols/icons/utils", () => ({
	FileIcon: ({ fileName }: { fileName: string }) => (
		<span data-testid="file-icon" data-filename={fileName} />
	),
	FolderIcon: ({ folderName }: { folderName: string }) => (
		<span data-testid="folder-icon" data-foldername={folderName} />
	),
}));

// react-resizable-panels does not work in jsdom
vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => (
		<div data-testid="resizable-group">{children}</div>
	),
	Panel: ({
		children,
		id,
	}: {
		children: React.ReactNode;
		id?: string;
		[key: string]: unknown;
	}) => <div data-testid={`panel-${id ?? "unknown"}`}>{children}</div>,
	Separator: () => <div data-testid="separator" />,
}));

vi.mock("@/hooks/useReviewPanel", () => ({
	useReviewPanel: vi.fn().mockReturnValue({
		diffBase: "head",
		diffMode: "gutter",
		selectedFile: null,
		selectedSection: "changes",
		setDiffBase: vi.fn(),
		setDiffMode: vi.fn(),
		selectFile: vi.fn(),
	}),
}));

vi.mock("@/hooks/useBranchDiffFiles", () => ({
	useBranchDiffFiles: vi.fn().mockReturnValue({ files: [] }),
}));

vi.mock("@/hooks/useGitStatus", () => ({
	useGitStatus: vi.fn().mockReturnValue({
		stagedFiles: [],
		changedFiles: [],
		refresh: vi.fn(),
	}),
}));

vi.mock("@/hooks/useDiffFileTree", () => ({
	useDiffFileTree: vi.fn().mockReturnValue({
		stagedTree: [],
		changesTree: [],
		stagedFileCount: 0,
		changesFileCount: 0,
		branchBaseTree: [],
		branchBaseFileCount: 0,
		loading: false,
	}),
}));

vi.mock("@/hooks/useFileDiffContent", () => ({
	useFileDiffContent: vi.fn().mockReturnValue({
		originalContent: "",
		modifiedContent: "",
	}),
}));

vi.mock("@/hooks/useHunks", () => ({
	useHunks: vi.fn().mockReturnValue({
		changeGroups: [],
		currentIndex: 0,
		total: 0,
		goToNext: vi.fn(),
		goToPrev: vi.fn(),
	}),
}));

vi.mock("@/hooks/useImageDiff", () => ({
	useImageDiff: vi.fn().mockReturnValue(null),
}));

vi.mock("@/hooks/useGitActions", () => ({
	useGitActions: vi.fn().mockReturnValue({
		stage: vi.fn(),
		unstage: vi.fn(),
		stageHunk: vi.fn(),
		unstageHunk: vi.fn(),
	}),
}));

const { useDiffFileTree } = await import("@/hooks/useDiffFileTree");
const { useReviewPanel } = await import("@/hooks/useReviewPanel");

describe("ReviewPanel", () => {
	it("should show 'No changes' when totalFileCount is 0", () => {
		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					baseBranch="main"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByText("No changes")).toBeInTheDocument();
	});

	it("should show 'Select a file to view diff' when no file is selected and files exist", () => {
		vi.mocked(useDiffFileTree).mockReturnValue({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					baseBranch="main"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		expect(screen.getByText("Select a file to view diff")).toBeInTheDocument();
	});

	it("should render DiffFileTree when files exist", () => {
		vi.mocked(useDiffFileTree).mockReturnValue({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel
					rootPath="/repo"
					baseBranch="main"
					diffOnlyMode={false}
					onDiffOnlyModeChange={vi.fn()}
				/>
			</TooltipProvider>,
		);

		// DiffFileTree is rendered inside the diff-files panel
		expect(screen.getByTestId("panel-diff-files")).toBeInTheDocument();
		expect(screen.getByTestId("panel-diff-view")).toBeInTheDocument();
	});

	it("should show breadcrumb when a file is selected", () => {
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: "src/components/App.tsx",
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});
		vi.mocked(useDiffFileTree).mockReturnValue({
			stagedTree: [],
			changesTree: [
				{
					id: "file:src/components/App.tsx",
					name: "App.tsx",
					path: "src/components/App.tsx",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel rootPath="/repo" baseBranch="main" />
			</TooltipProvider>,
		);

		expect(screen.getByTestId("breadcrumb")).toBeInTheDocument();
		const breadcrumb = within(screen.getByTestId("breadcrumb"));
		expect(breadcrumb.getByText("src")).toBeInTheDocument();
		expect(breadcrumb.getByText("components")).toBeInTheDocument();
		expect(breadcrumb.getByText("App.tsx")).toBeInTheDocument();
	});

	it("should not show breadcrumb when no file is selected", () => {
		vi.mocked(useReviewPanel).mockReturnValue({
			diffBase: "head",
			diffMode: "gutter",
			selectedFile: null,
			selectedSection: "changes",
			setDiffBase: vi.fn(),
			setDiffMode: vi.fn(),
			selectFile: vi.fn(),
		});
		vi.mocked(useDiffFileTree).mockReturnValue({
			stagedTree: [],
			changesTree: [
				{
					id: "file:file.ts",
					name: "file.ts",
					path: "file.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
			branchBaseTree: [],
			branchBaseFileCount: 0,
			loading: false,
		});

		render(
			<TooltipProvider>
				<ReviewPanel rootPath="/repo" baseBranch="main" />
			</TooltipProvider>,
		);

		expect(screen.queryByTestId("breadcrumb")).not.toBeInTheDocument();
	});
});
