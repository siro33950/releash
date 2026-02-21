import { Eye, FileCode } from "lucide-react";
import { cn } from "@/lib/utils";

export interface PreviewToggleProps {
	showPreview: boolean;
	onShowPreviewChange: (show: boolean) => void;
}

export function PreviewToggle({
	showPreview,
	onShowPreviewChange,
}: PreviewToggleProps) {
	return (
		<div className="flex items-center gap-0.5 bg-muted rounded p-0.5">
			<button
				type="button"
				onClick={() => onShowPreviewChange(false)}
				className={cn(
					"flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] transition-colors",
					!showPreview
						? "bg-background shadow-sm text-foreground"
						: "text-muted-foreground hover:text-foreground",
				)}
				title="Editor"
			>
				<FileCode className="h-3 w-3" />
				Editor
			</button>
			<button
				type="button"
				onClick={() => onShowPreviewChange(true)}
				className={cn(
					"flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] transition-colors",
					showPreview
						? "bg-background shadow-sm text-foreground"
						: "text-muted-foreground hover:text-foreground",
				)}
				title="Preview"
			>
				<Eye className="h-3 w-3" />
				Preview
			</button>
		</div>
	);
}
