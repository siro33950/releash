import { useCallback, useRef } from "react";
import { normalizePath } from "@/lib/normalizePath";
import type { TabInfo } from "@/types/editor";

interface UseHandleOpenFileParams {
	openFile: (path: string) => Promise<void>;
	getFileContent: (path: string) => TabInfo | undefined;
	addTab: (path: string, name: string, isDirty: boolean) => void;
	onSwitchToEditor?: () => void;
}

export function useHandleOpenFile({
	openFile,
	getFileContent,
	addTab,
	onSwitchToEditor,
}: UseHandleOpenFileParams): (path: string) => Promise<void> {
	const onSwitchToEditorRef = useRef(onSwitchToEditor);
	onSwitchToEditorRef.current = onSwitchToEditor;

	return useCallback(
		async (path: string) => {
			const normalizedPath = normalizePath(path);
			await openFile(normalizedPath);
			const file = getFileContent(normalizedPath);
			const name = normalizedPath.split("/").pop() ?? normalizedPath;
			addTab(normalizedPath, name, file?.isDirty ?? false);
			onSwitchToEditorRef.current?.();
		},
		[openFile, getFileContent, addTab],
	);
}
