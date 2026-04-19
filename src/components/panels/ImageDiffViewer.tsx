import { ScrollArea } from "@/components/ui/scroll-area";

interface ImageDiffViewerProps {
	originalUrl: string | null;
	modifiedUrl: string | null;
	loading: boolean;
}

function ImagePane({ url, label }: { url: string | null; label: string }) {
	return (
		<div className="flex-1 flex flex-col items-center gap-2 min-w-0 p-4">
			<span className="text-xs font-medium text-muted-foreground">{label}</span>
			{url ? (
				<ScrollArea
					className="relative max-h-full w-full rounded border border-border"
					style={{
						backgroundImage:
							"repeating-conic-gradient(#80808020 0% 25%, transparent 0% 50%)",
						backgroundSize: "16px 16px",
					}}
				>
					<div className="flex items-center justify-center">
						<img
							src={url}
							alt={label}
							className="max-w-full max-h-[60vh] object-contain"
						/>
					</div>
				</ScrollArea>
			) : (
				<div className="flex items-center justify-center flex-1 w-full rounded border border-dashed border-border text-muted-foreground text-sm">
					No file
				</div>
			)}
		</div>
	);
}

export function ImageDiffViewer({
	originalUrl,
	modifiedUrl,
	loading,
}: ImageDiffViewerProps) {
	if (loading) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground text-sm">
				Loading...
			</div>
		);
	}

	return (
		<div className="flex h-full bg-background" data-testid="image-diff-viewer">
			<ImagePane url={originalUrl} label="Original" />
			<div className="w-px bg-border shrink-0" />
			<ImagePane url={modifiedUrl} label="Modified" />
		</div>
	);
}
