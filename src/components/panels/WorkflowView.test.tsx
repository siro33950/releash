import { render, screen } from "@testing-library/react";
import { useContext } from "react";
import { describe, expect, it, vi } from "vitest";
import {
	EditorContext,
	type EditorContextValue,
} from "@/contexts/EditorContext";
import { WorkflowView } from "./WorkflowView";

const captured: { context: EditorContextValue | null } = { context: null };
function getCapturedContext(): EditorContextValue | null {
	return captured.context;
}

vi.mock("@/components/panels/EditorTabContent", () => ({
	EditorTabContent: ({ filePath }: { filePath: string }) => {
		// eslint-disable-next-line react-hooks/rules-of-hooks
		captured.context = useContext(EditorContext);
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
		captured.context = null;
		render(<WorkflowView {...defaultProps} />);

		const ctx = getCapturedContext();
		expect(ctx).not.toBeNull();
		expect(ctx?.rootPath).toBe(mockPlanEditorContextValue.rootPath);
		expect(ctx?.diffBase).toBe(mockPlanEditorContextValue.diffBase);
		expect(ctx?.diffMode).toBe(mockPlanEditorContextValue.diffMode);
		expect(ctx?.threads).toBe(mockPlanEditorContextValue.threads);
		expect(ctx?.createThread).toBe(mockPlanEditorContextValue.createThread);
		expect(ctx?.getFileContent).toBe(mockPlanEditorContextValue.getFileContent);
	});
});
