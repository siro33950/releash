import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";

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
	useEffect(() => {
		setEdited(summary);
	}, [summary]);

	return (
		<Dialog open={open} onOpenChange={(o) => !o && !loading && onCancel()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>Post to Pull Request</DialogTitle>
				</DialogHeader>
				<Textarea
					className="min-h-[160px] font-mono resize-y"
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
