import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { EditorTabContent } from "./EditorTabContent";

const mockUseGitOriginalContent = vi.fn().mockReturnValue("");

vi.mock("@/hooks/useGitOriginalContent", () => ({
	useGitOriginalContent: (...args: unknown[]) =>
		mockUseGitOriginalContent(...args),
}));

vi.mock("@/hooks/useHunks", () => ({
	useHunks: () => ({
		changeGroups: [],
		currentIndex: 0,
		total: 0,
		goTo: vi.fn(),
	}),
}));

vi.mock("@/hooks/useImageDiff", () => ({
	useImageDiff: () => null,
}));

vi.mock("./useDiffOperations", () => ({
	useDiffOperations: () => ({
		handleStageGroup: vi.fn(),
		handleUnstageGroup: vi.fn(),
		handleStageAll: vi.fn(),
		handleUnstageAll: vi.fn(),
	}),
}));

vi.mock("./DiffViewerSection", () => ({
	DiffViewerSection: () => <div data-testid="diff-viewer-section" />,
}));

vi.mock("./Breadcrumb", () => ({
	Breadcrumb: ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="breadcrumb">{children}</div>
	),
}));

const mockDiffToolbar = vi.fn((_props: Record<string, unknown>) => (
	<div data-testid="diff-toolbar" />
));

vi.mock("./DiffToolbar", () => ({
	DiffToolbar: (props: Record<string, unknown>) => mockDiffToolbar(props),
}));

vi.mock("./EmptyState", () => ({
	EmptyState: () => <div data-testid="empty-state" />,
}));

vi.mock("./PreviewToggle", () => ({
	PreviewToggle: () => null,
}));

// Radix UI pointer capture polyfill
if (typeof Element.prototype.hasPointerCapture !== "function") {
	Element.prototype.hasPointerCapture = () => false;
}
if (typeof Element.prototype.setPointerCapture !== "function") {
	Element.prototype.setPointerCapture = () => {};
}
if (typeof Element.prototype.releasePointerCapture !== "function") {
	Element.prototype.releasePointerCapture = () => {};
}

const mockEditorContextBase = {
	getFileContent: vi.fn().mockReturnValue({
		content: "modified content",
		originalContent: "original content",
		language: "typescript",
	}),
	updateContent: vi.fn(),
	diffBase: "staged" as string,
	diffMode: "inline" as const,
	setDiffMode: vi.fn(),
	threads: [],
	createThread: vi.fn(),
	addEntry: vi.fn(),
	deleteThread: vi.fn(),
	resolveThread: vi.fn(),
	implementThread: vi.fn(),
	onPostToPr: vi.fn(),
	aiRunningThreadIds: new Set<string>(),
	aiTaskThreadIds: new Set<string>(),
	onOpenThreadAIModal: vi.fn(),
	onAskAI: vi.fn(),
	updateEntry: vi.fn(),
	copyThread: vi.fn(),
	recalculateAnchorsForFile: vi.fn(),
	showResolvedThreads: false,
	rootPath: "/test/repo",
	onStageHunk: vi.fn(),
	onGitChanged: vi.fn(),
	gitRefreshKey: 0,
	theme: "dark" as const,
	fontSize: 14,
	onSearchOccurrences: vi.fn(),
	lspStatus: "idle" as const,
	lspError: null,
	lspCrashCount: 0,
	lspRetryManually: vi.fn(),
};

let currentEditorContext = { ...mockEditorContextBase };

vi.mock("@/contexts/EditorContext", () => ({
	useEditorContext: () => currentEditorContext,
}));

describe("EditorTabContent - diffBase propagation to useGitOriginalContent", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		currentEditorContext = { ...mockEditorContextBase };
	});

	it("should call useGitOriginalContent with diffBase='staged' when diffBase is staged", () => {
		currentEditorContext.diffBase = "staged";
		render(<EditorTabContent filePath="/test/repo/src/file.ts" />);

		expect(mockUseGitOriginalContent).toHaveBeenCalledTimes(2);
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			1,
			"/test/repo/src/file.ts",
			"staged",
			"original content",
			0,
		);
		// Second call: stagedContent with null filePath (not branch-base mode)
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			2,
			null,
			"staged",
			"",
			0,
		);
	});

	it("should call useGitOriginalContent with diffBase='branch-base' when diffBase is branch-base", () => {
		currentEditorContext.diffBase = "branch-base";
		render(<EditorTabContent filePath="/test/repo/src/file.ts" />);

		expect(mockUseGitOriginalContent).toHaveBeenCalledTimes(2);
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			1,
			"/test/repo/src/file.ts",
			"branch-base",
			"original content",
			0,
		);
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			2,
			"/test/repo/src/file.ts",
			"staged",
			"",
			0,
		);
	});

	it("should make a second useGitOriginalContent call for staged content when diffBase is branch-base", () => {
		currentEditorContext.diffBase = "branch-base";
		render(<EditorTabContent filePath="/test/repo/src/file.ts" />);

		expect(mockUseGitOriginalContent).toHaveBeenCalledTimes(2);
		// Second call: stagedContent with filePath (branch-base mode)
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			2,
			"/test/repo/src/file.ts",
			"staged",
			"",
			0,
		);
	});

	it("should pass null filePath for stagedContent call when diffBase is staged", () => {
		currentEditorContext.diffBase = "staged";
		render(<EditorTabContent filePath="/test/repo/src/file.ts" />);

		expect(mockUseGitOriginalContent).toHaveBeenCalledTimes(2);
		// Second call: stagedContent should receive null filePath when diffBase != "branch-base"
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			2,
			null,
			"staged",
			"",
			0,
		);
	});

	it("should not pass onDiffBaseChange to DiffToolbar", () => {
		render(<EditorTabContent filePath="/test/repo/src/file.ts" />);

		const toolbarProps = mockDiffToolbar.mock.lastCall?.[0];
		expect(toolbarProps).not.toHaveProperty("onDiffBaseChange");
	});

	it("should update useGitOriginalContent calls when diffBase changes after mount", () => {
		currentEditorContext.diffBase = "staged";
		const { rerender } = render(
			<EditorTabContent filePath="/test/repo/src/file.ts" />,
		);

		mockUseGitOriginalContent.mockClear();
		currentEditorContext = {
			...mockEditorContextBase,
			diffBase: "branch-base",
		};

		rerender(<EditorTabContent filePath="/test/repo/src/file.ts" />);

		expect(mockUseGitOriginalContent).toHaveBeenCalledTimes(2);
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			1,
			"/test/repo/src/file.ts",
			"branch-base",
			"original content",
			0,
		);
		expect(mockUseGitOriginalContent).toHaveBeenNthCalledWith(
			2,
			"/test/repo/src/file.ts",
			"staged",
			"",
			0,
		);
	});
});
