import { AlertTriangle, X } from "lucide-react";
import type { SessionFeedbackEntry } from "@/hooks/useSessionStore";

interface SessionFeedbackBannersProps {
	entries: SessionFeedbackEntry[];
	onDismiss?: (entry: SessionFeedbackEntry) => void | Promise<void>;
	onRetry?: (entry: SessionFeedbackEntry) => void | Promise<void>;
	hasMore?: boolean;
	onLoadMore?: () => void | Promise<void>;
}

/** Exact backend-owned feedback controls shared by hydrated and failed-load views. */
export function SessionFeedbackBanners({
	entries,
	onDismiss,
	onRetry,
	hasMore = false,
	onLoadMore,
}: SessionFeedbackBannersProps) {
	if (entries.length === 0 && !hasMore) return null;

	return (
		<div data-testid="session-feedback-list">
			{entries.map((entry) => (
				<div className="px-2 pb-2" key={entry.feedback_id}>
					<div
						className="flex items-start gap-2 rounded border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
						role="alert"
						data-testid="session-feedback-banner"
					>
						<AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
						<div className="min-w-0 flex-1">
							<div>{entry.failure.label}</div>
							{entry.failure.detail && (
								<div className="mt-0.5 opacity-80">{entry.failure.detail}</div>
							)}
							<div className="mt-0.5 font-mono opacity-70">
								{entry.failure.correlation_id}
							</div>
						</div>
						{entry.actions.includes("retry_resolution") && onRetry && (
							<button
								type="button"
								className="shrink-0 rounded px-1.5 py-0.5 hover:bg-destructive/10"
								onClick={() => void onRetry(entry)}
							>
								Retry
							</button>
						)}
						{entry.actions.includes("dismiss") && onDismiss && (
							<button
								type="button"
								className="shrink-0 rounded p-0.5 hover:bg-destructive/10"
								aria-label="Dismiss feedback"
								onClick={() => void onDismiss(entry)}
							>
								<X className="size-3.5" />
							</button>
						)}
					</div>
				</div>
			))}
			{hasMore && onLoadMore && (
				<div className="px-2 pb-2">
					<button
						type="button"
						className="w-full rounded border px-3 py-1.5 text-xs text-muted-foreground hover:bg-muted"
						onClick={() => void onLoadMore()}
					>
						Load more feedback
					</button>
				</div>
			)}
		</div>
	);
}
