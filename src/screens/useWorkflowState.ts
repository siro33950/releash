import { useEffect, useMemo, useState } from "react";
import type { EditorContextValue } from "@/contexts/EditorContext";
import { useThreads } from "@/hooks/useThreads";
import type { TabInfo } from "@/types/editor";
import type { Theme } from "@/types/settings";

interface UseWorkflowStateParams {
	rootPath: string;
	getFileContent: (path: string) => TabInfo | undefined;
	updateContent: (path: string, content: string) => void;
	saveFile: (path: string) => Promise<void>;
	registerVirtualFile: (
		path: string,
		content: string,
		language: string,
	) => void;
	theme?: Theme;
	fontSize?: number;
	initialPanelRatios?: [number, number];
}

export interface UseWorkflowStateReturn {
	planThreads: ReturnType<typeof useThreads>["threads"];
	removePlanThread: (id: string) => void;
	resolvePlanThread: (id: string) => void;
	planEditorContextValue: EditorContextValue;
	workflowPanelRatios: [number, number] | undefined;
	setWorkflowPanelRatios: (ratios: [number, number]) => void;
}

const noop = () => {};

export function useWorkflowState({
	rootPath,
	getFileContent,
	updateContent,
	saveFile,
	registerVirtualFile,
	theme,
	fontSize,
	initialPanelRatios,
}: UseWorkflowStateParams): UseWorkflowStateReturn {
	// TODO: planDocument will become dynamic state when workflow execution is implemented.
	const planDocument = "";
	useEffect(() => {
		registerVirtualFile("workflow://plan", planDocument, "markdown");
	}, [registerVirtualFile]);
	const {
		threads: planThreads,
		createThread: createPlanThread,
		addEntry: addPlanEntry,
		removeThread: removePlanThread,
		updateEntry: updatePlanEntry,
		resolveThread: resolvePlanThread,
		showResolvedThreads: showResolvedPlanThreads,
		toggleShowResolvedThreads: toggleShowResolvedPlanThreads,
		recalculateAnchorsForFile: recalculatePlanAnchors,
	} = useThreads(`${rootPath}::plan`);

	const [workflowPanelRatios, setWorkflowPanelRatios] = useState<
		[number, number] | undefined
	>(initialPanelRatios);

	const planEditorContextValue = useMemo<EditorContextValue>(
		() => ({
			// File operations (needed for virtual file editing)
			getFileContent,
			updateContent,
			saveFile,
			rootPath,

			// Theme
			theme,
			fontSize,

			// Thread operations (plan-specific)
			threads: planThreads,
			createThread: async (
				filePath,
				lineNumber,
				content,
				endLine?,
				fileContent?,
			) => {
				await createPlanThread(
					filePath,
					lineNumber,
					content,
					endLine,
					undefined,
					undefined,
					undefined,
					fileContent,
				);
			},
			addEntry: addPlanEntry,
			deleteThread: removePlanThread,
			resolveThread: resolvePlanThread,
			updateEntry: updatePlanEntry,
			recalculateAnchorsForFile: recalculatePlanAnchors,
			showResolvedThreads: showResolvedPlanThreads,
			toggleShowResolvedThreads: toggleShowResolvedPlanThreads,

			// Git/diff defaults (not applicable to virtual workflow files)
			diffBase: "staged",
			diffMode: "inline",
			setDiffBase: noop,
			setDiffMode: noop,
			gitRefreshKey: 0,

			// LSP defaults (not applicable to virtual workflow files)
			lspStatus: "idle",
			lspError: null,
			lspCrashCount: 0,
			lspRetryManually: noop,
		}),
		[
			getFileContent,
			updateContent,
			saveFile,
			rootPath,
			theme,
			fontSize,
			planThreads,
			createPlanThread,
			addPlanEntry,
			removePlanThread,
			resolvePlanThread,
			updatePlanEntry,
			recalculatePlanAnchors,
			showResolvedPlanThreads,
			toggleShowResolvedPlanThreads,
		],
	);

	return {
		planThreads,
		removePlanThread,
		resolvePlanThread,
		planEditorContextValue,
		workflowPanelRatios,
		setWorkflowPanelRatios,
	};
}
