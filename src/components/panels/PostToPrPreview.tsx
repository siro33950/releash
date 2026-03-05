import { Loader2 } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";

export interface PostToPrPreviewProps {
	open: boolean;
	summary: string;
	loading?: boolean;
	onPost: (editedSummary: string) => void;
	onCancel: () => void;
}

export function PostToPrPreview({
	open,
	summary,
	loading,
	onPost,
	onCancel,
}: PostToPrPreviewProps) {
	const [edited, setEdited] = useState(summary);

	// Sync when summary changes (new preview opened)
	const [prevSummary, setPrevSummary] = useState(summary);
	if (summary !== prevSummary) {
		setPrevSummary(summary);
		setEdited(summary);
	}

	return (
		<Dialog open={open} onOpenChange={(o) => !o && onCancel()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>Post to Pull Request</DialogTitle>
				</DialogHeader>
				<textarea
					className="w-full min-h-[160px] p-3 text-sm font-mono bg-muted border border-border rounded-md resize-y focus:outline-none focus:ring-1 focus:ring-ring"
					value={edited}
					onChange={(e) => setEdited(e.target.value)}
					disabled={loading}
				/>
				<DialogFooter>
					<Button variant="ghost" onClick={onCancel} disabled={loading}>
						Cancel
					</Button>
					<Button
						onClick={() => onPost(edited)}
						disabled={loading || !edited.trim()}
					>
						{loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
						Post
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
