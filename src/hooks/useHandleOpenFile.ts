import { useCallback, useRef } from "react";
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
			await openFile(path);
			const file = getFileContent(path);
			const name = path.split(/[/\\]/).pop() ?? path;
			addTab(path, name, file?.isDirty ?? false);
			onSwitchToEditorRef.current?.();
		},
		[openFile, getFileContent, addTab],
	);
}
