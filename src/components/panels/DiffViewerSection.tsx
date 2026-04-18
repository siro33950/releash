import type { DiffMode } from "@/types/settings";
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
		<div className="flex items-center justify-center h-full text-muted-foreground text-xs">
			Diff viewer not available
		</div>
	);
}
