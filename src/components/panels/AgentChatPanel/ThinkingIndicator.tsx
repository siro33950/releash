import { ChevronRight, Loader2 } from "lucide-react";
import { useState } from "react";

interface ThinkingIndicatorProps {
	content?: string;
	isStreaming?: boolean;
}

export function ThinkingIndicator({
	content,
	isStreaming,
}: ThinkingIndicatorProps) {
	const [isOpen, setIsOpen] = useState(false);

	if (!content) {
		return (
			<div data-testid="thinking-indicator" className="px-4 py-3">
				<div className="flex items-center gap-2 text-sm text-muted-foreground">
					<Loader2 className="size-4 animate-spin" />
					<span>Thinking...</span>
				</div>
			</div>
		);
	}

	return (
		<div data-testid="thinking-indicator" className="px-4 py-2">
			<button
				type="button"
				data-testid="thinking-toggle"
				className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
				onClick={() => setIsOpen(!isOpen)}
			>
				<ChevronRight
					className={`size-3 transition-transform ${isOpen ? "rotate-90" : ""}`}
				/>
				<span>Thinking</span>
				{isStreaming && <Loader2 className="size-3 animate-spin ml-1" />}
			</button>
			{isOpen && (
				<div
					data-testid="thinking-content"
					className="mt-1 pl-4 text-xs text-muted-foreground whitespace-pre-wrap border-l border-border/50"
				>
					{content}
				</div>
			)}
		</div>
	);
}
