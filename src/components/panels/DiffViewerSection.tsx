import type { ChangeGroup } from "@/lib/computeHunks";
import type { DiffMode } from "@/types/settings";
import { CodeDiffViewer } from "./CodeDiffViewer";
import { ImageDiffViewer } from "./ImageDiffViewer";
import { MarkdownDiffViewer } from "./MarkdownDiffViewer";

export interface DiffViewerSectionProps {
	isImage: boolean;
	isMarkdown: boolean;
	showPreview: boolean;
	imageDiff: {
		originalUrl: string | null;
		modifiedUrl: string | null;
		loading: boolean;
	};
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	filePath?: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
}

export function DiffViewerSection(props: DiffViewerSectionProps) {
	if (props.isImage) {
		return (
			<ImageDiffViewer
				originalUrl={props.imageDiff.originalUrl}
				modifiedUrl={props.imageDiff.modifiedUrl}
				loading={props.imageDiff.loading}
			/>
		);
	}

	if (props.isMarkdown && props.showPreview) {
		return (
			<MarkdownDiffViewer
				originalContent={props.originalContent}
				modifiedContent={props.modifiedContent}
				diffMode={props.diffMode}
			/>
		);
	}

	return (
		<CodeDiffViewer
			originalContent={props.originalContent}
			modifiedContent={props.modifiedContent}
			diffMode={props.diffMode}
			filePath={props.filePath}
			changeGroups={props.changeGroups}
			onStageGroup={props.onStageGroup}
			groupActionLabel={props.groupActionLabel}
		/>
	);
}
