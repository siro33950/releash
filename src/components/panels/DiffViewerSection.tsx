import type { ChangeGroup } from "@/lib/computeHunks";
import type { DiffComment } from "@/types/diffComment";
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
	diffOnlyMode?: boolean;
	filePath?: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
	comments?: DiffComment[];
	onAddComment?: (lineNumber: number, content: string) => Promise<void>;
	onAddRangeComment?: (
		startLine: number,
		endLine: number,
		content: string,
	) => Promise<void>;
	onUpdateComment?: (commentId: string, content: string) => Promise<void>;
	onDeleteComment?: (commentId: string) => Promise<void>;
	onSendComment?: (commentIds: string[]) => Promise<void>;
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
				diffOnlyMode={props.diffOnlyMode}
			/>
		);
	}

	return (
		<CodeDiffViewer
			originalContent={props.originalContent}
			modifiedContent={props.modifiedContent}
			diffMode={props.diffMode}
			diffOnlyMode={props.diffOnlyMode}
			filePath={props.filePath}
			changeGroups={props.changeGroups}
			onStageGroup={props.onStageGroup}
			groupActionLabel={props.groupActionLabel}
			comments={props.comments}
			onAddComment={props.onAddComment}
			onAddRangeComment={props.onAddRangeComment}
			onUpdateComment={props.onUpdateComment}
			onDeleteComment={props.onDeleteComment}
			onSendComment={props.onSendComment}
		/>
	);
}
