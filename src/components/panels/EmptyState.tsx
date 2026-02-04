import { FileText } from "lucide-react";

export function EmptyState() {
	return (
		<div className="flex flex-col items-center justify-center h-full text-muted-foreground">
			<FileText className="h-16 w-16 mb-4 opacity-50" />
			<h3 className="text-lg font-medium mb-2">No file selected</h3>
			<p className="text-sm">
				Select a file from the explorer to view its contents
			</p>
		</div>
	);
}
