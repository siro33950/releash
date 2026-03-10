import { EditorTabContent } from "@/components/panels/EditorTabContent";
import {
	EditorContext,
	type EditorContextValue,
} from "@/contexts/EditorContext";

interface WorkflowViewProps {
	planEditorContextValue: EditorContextValue;
}

export function WorkflowView({ planEditorContextValue }: WorkflowViewProps) {
	return (
		<div className="h-full relative">
			<EditorContext.Provider value={planEditorContextValue}>
				<EditorTabContent filePath="workflow://plan" />
			</EditorContext.Provider>
		</div>
	);
}
