import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useEditorContext } from "@/contexts/EditorContext";
import { WorkflowView } from "./WorkflowView";

let capturedContext: ReturnType<typeof useEditorContext> | null = null;

vi.mock("@/components/panels/EditorTabContent", () => ({
	EditorTabContent: ({ filePath }: { filePath: string }) => {
		// eslint-disable-next-line react-hooks/rules-of-hooks
		capturedContext = useEditorContext();
		return <div data-testid="editor-tab-content" data-file-path={filePath} />;
	},
}));

const mockPlanEditorContextValue = {
	getFileContent: vi.fn(),
	updateContent: vi.fn(),
	saveFile: vi.fn(),
	diffBase: "staged" as const,
	diffMode: "inline" as const,
	setDiffBase: vi.fn(),
	setDiffMode: vi.fn(),
	threads: [],
	createThread: vi.fn(),
	addEntry: vi.fn(),
	deleteThread: vi.fn(),
	updateEntry: vi.fn(),
	showResolvedThreads: false,
	toggleShowResolvedThreads: vi.fn(),
	rootPath: "/repo",
	gitRefreshKey: 0,
	lspStatus: "idle" as const,
	lspError: null,
	lspCrashCount: 0,
	lspRetryManually: vi.fn(),
};

describe("WorkflowView", () => {
	const defaultProps = {
		planEditorContextValue: mockPlanEditorContextValue,
	};

	it("renders EditorTabContent with workflow://plan path", () => {
		render(<WorkflowView {...defaultProps} />);

		const editor = screen.getByTestId("editor-tab-content");
		expect(editor).toBeInTheDocument();
		expect(editor).toHaveAttribute("data-file-path", "workflow://plan");
	});

	it("EditorContext.Providerが正しいplanEditorContextValueでラップされている", () => {
		capturedContext = null;
		render(<WorkflowView {...defaultProps} />);

		expect(capturedContext).not.toBeNull();
		expect(capturedContext?.rootPath).toBe(mockPlanEditorContextValue.rootPath);
		expect(capturedContext?.diffBase).toBe(mockPlanEditorContextValue.diffBase);
		expect(capturedContext?.diffMode).toBe(mockPlanEditorContextValue.diffMode);
		expect(capturedContext?.threads).toBe(mockPlanEditorContextValue.threads);
		expect(capturedContext?.createThread).toBe(
			mockPlanEditorContextValue.createThread,
		);
		expect(capturedContext?.getFileContent).toBe(
			mockPlanEditorContextValue.getFileContent,
		);
	});
});
