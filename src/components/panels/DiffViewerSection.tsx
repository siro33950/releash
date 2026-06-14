import type { ChangeGroup } from "@/lib/computeHunks";
import type { ReviewDiscussionThread } from "@/types/diffComment";
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
	comments?: ReviewDiscussionThread[];
	onAddComment?: (lineNumber: number, content: string) => Promise<void>;
	onAddRangeComment?: (
		startLine: number,
		endLine: number,
		content: string,
	) => Promise<void>;
	onAppendComment?: (threadId: string, content: string) => Promise<void>;
	onResolveThread?: (
		threadId: string,
		outcome: string,
		summary: string,
	) => Promise<void>;
	onDeleteThread?: (threadId: string) => Promise<void>;
	scrollToLine?: number | null;
	scrollToThread?: string | null;
	onLineRangeSelected?: (startLine: number, endLine: number) => void;
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
			onAppendComment={props.onAppendComment}
			onResolveThread={props.onResolveThread}
			onDeleteThread={props.onDeleteThread}
			scrollToLine={props.scrollToLine}
			scrollToThread={props.scrollToThread}
			onLineRangeSelected={props.onLineRangeSelected}
		/>
	);
}
