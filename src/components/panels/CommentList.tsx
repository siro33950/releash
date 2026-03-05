import {
	AlertTriangle,
	Check,
	ChevronDown,
	ChevronUp,
	Eye,
	EyeOff,
	Info,
	Lightbulb,
	MessageSquare,
	ScrollText,
	Trash2,
	XCircle,
} from "lucide-react";
import { useMemo } from "react";
import { EmptyState } from "@/components/ui/empty-state";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useUnresolvedNavigation } from "@/hooks/useUnresolvedNavigation";
import { cn } from "@/lib/utils";
import type { Thread, ThreadSeverity } from "@/types/thread";

export interface CommentListProps {
	threads: Thread[];
	onThreadClick?: (filePath: string, lineNumber: number) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	showResolvedThreads?: boolean;
	onToggleShowResolved?: () => void;
	aiTaskThreadIds?: Set<string>;
	onOpenThreadAILog?: (threadId: string) => void;
}

function SeverityIcon({ severity }: { severity?: ThreadSeverity }) {
	switch (severity) {
		case "error":
			return <XCircle className="h-3 w-3 shrink-0 mt-0.5 text-destructive" />;
		case "warning":
			return (
				<AlertTriangle className="h-3 w-3 shrink-0 mt-0.5 text-yellow-500" />
			);
		case "info":
			return <Info className="h-3 w-3 shrink-0 mt-0.5 text-blue-400" />;
		case "suggestion":
			return <Lightbulb className="h-3 w-3 shrink-0 mt-0.5 text-green-400" />;
		default:
			return (
				<MessageSquare className="h-3 w-3 shrink-0 mt-0.5 text-muted-foreground" />
			);
	}
}

export function CommentList({
	threads,
	onThreadClick,
	onDeleteThread,
	onResolveThread,
	showResolvedThreads = false,
	onToggleShowResolved,
	aiTaskThreadIds,
	onOpenThreadAILog,
}: CommentListProps) {
	const resolvedCount = threads.filter((t) => t.resolved).length;

	const { currentIndex, total, goNext, goPrev } = useUnresolvedNavigation(
		threads,
		onThreadClick,
	);

	const visibleThreads = useMemo(
		() => (showResolvedThreads ? threads : threads.filter((t) => !t.resolved)),
		[threads, showResolvedThreads],
	);

	if (visibleThreads.length === 0 && resolvedCount === 0) {
		return (
			<EmptyState
				icon={MessageSquare}
				title="No comments"
				description={
					<>
						<p>
							Click the left margin of a line number, or drag to select a range
						</p>
						<p className="mt-0.5">
							<kbd className="px-1 py-0.5 bg-muted rounded text-[10px] font-mono">
								⌘K
							</kbd>{" "}
							to add a comment on the current cursor line
						</p>
					</>
				}
			/>
		);
	}

	if (visibleThreads.length === 0 && resolvedCount > 0) {
		return (
			<div className="h-full flex flex-col">
				<div className="p-2">
					{onToggleShowResolved && (
						<button
							type="button"
							onClick={onToggleShowResolved}
							className="flex items-center gap-1 w-full px-1.5 py-1 mb-1.5 text-[11px] text-muted-foreground rounded hover:bg-muted transition-colors"
							data-testid="toggle-resolved-comments"
						>
							{showResolvedThreads ? (
								<EyeOff className="h-3 w-3" />
							) : (
								<Eye className="h-3 w-3" />
							)}
							Resolved ({resolvedCount})
						</button>
					)}
				</div>
				<EmptyState
					icon={Check}
					title="All threads resolved"
					description={`${resolvedCount} thread${resolvedCount === 1 ? "" : "s"} resolved`}
				/>
			</div>
		);
	}

	const grouped = new Map<string, Thread[]>();
	for (const thread of visibleThreads) {
		const existing = grouped.get(thread.filePath);
		if (existing) {
			existing.push(thread);
		} else {
			grouped.set(thread.filePath, [thread]);
		}
	}

	return (
		<ScrollArea className="h-full [&_[data-slot=scroll-area-viewport]>div]:!block">
			<div className="p-2">
				{total > 0 && (
					<div className="flex items-center gap-1 mb-1.5 px-1">
						<span className="text-[11px] text-muted-foreground flex-1">
							{currentIndex >= 0
								? `${currentIndex + 1} / ${total}`
								: `${total} unresolved`}
						</span>
						<button
							type="button"
							onClick={goPrev}
							className="p-0.5 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
							aria-label="Previous unresolved thread"
						>
							<ChevronUp className="h-3.5 w-3.5" />
						</button>
						<button
							type="button"
							onClick={goNext}
							className="p-0.5 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
							aria-label="Next unresolved thread"
						>
							<ChevronDown className="h-3.5 w-3.5" />
						</button>
					</div>
				)}
				{resolvedCount > 0 && onToggleShowResolved && (
					<button
						type="button"
						onClick={onToggleShowResolved}
						className="flex items-center gap-1 w-full px-1.5 py-1 mb-1.5 text-[11px] text-muted-foreground rounded hover:bg-muted transition-colors"
						data-testid="toggle-resolved-comments"
					>
						{showResolvedThreads ? (
							<EyeOff className="h-3 w-3" />
						) : (
							<Eye className="h-3 w-3" />
						)}
						Resolved ({resolvedCount})
					</button>
				)}
				{[...grouped.entries()].map(([filePath, fileThreads]) => {
					const fileName = filePath.split("/").pop() ?? filePath;
					return (
						<div key={filePath} className="mb-2">
							<div className="text-xs font-medium px-1 py-0.5 truncate">
								{fileName}
							</div>
							{fileThreads
								.sort((a, b) => a.lineNumber - b.lineNumber)
								.map((thread) => {
									const firstEntry = thread.entries[0];
									const entryCount = thread.entries.length;
									return (
										// biome-ignore lint/a11y/useSemanticElements: custom styled clickable row
										<div
											role="button"
											tabIndex={0}
											key={thread.id}
											onClick={() =>
												onThreadClick?.(thread.filePath, thread.lineNumber)
											}
											onKeyDown={(e) => {
												if (
													(e.key === "Enter" || e.key === " ") &&
													e.target === e.currentTarget
												) {
													e.preventDefault();
													onThreadClick?.(thread.filePath, thread.lineNumber);
												}
											}}
											className={cn(
												"group flex items-start gap-1.5 w-full px-1 py-1 text-[11px] rounded transition-colors",
												"hover:bg-muted text-left",
												thread.resolved && "opacity-50",
											)}
										>
											<SeverityIcon severity={thread.severity} />
											<div className="min-w-0 flex-1">
												<div className="flex items-center gap-1">
													<span className="text-muted-foreground font-mono">
														L{thread.lineNumber}
														{thread.endLine != null ? `-${thread.endLine}` : ""}
													</span>
													{entryCount > 1 && (
														<span className="text-[10px] px-1 rounded bg-muted text-muted-foreground">
															{entryCount} replies
														</span>
													)}
													{(onResolveThread ||
														onDeleteThread ||
														(onOpenThreadAILog &&
															aiTaskThreadIds?.has(thread.id))) && (
														<div className="hidden group-hover:flex items-center gap-0.5 ml-auto">
															{onOpenThreadAILog &&
																aiTaskThreadIds?.has(thread.id) && (
																	// biome-ignore lint/a11y/useSemanticElements: nested inside role="button", cannot use <button>
																	<span
																		role="button"
																		tabIndex={0}
																		className="p-0.5 rounded hover:bg-blue-500/20 text-muted-foreground hover:text-blue-400 transition-colors"
																		aria-label="View AI log"
																		onClick={(e) => {
																			e.stopPropagation();
																			onOpenThreadAILog(thread.id);
																		}}
																		onKeyDown={(e) => {
																			if (e.key === "Enter" || e.key === " ") {
																				e.preventDefault();
																				e.stopPropagation();
																				onOpenThreadAILog(thread.id);
																			}
																		}}
																	>
																		<ScrollText className="h-3 w-3" />
																	</span>
																)}
															{onResolveThread && !thread.resolved && (
																// biome-ignore lint/a11y/useSemanticElements: nested inside role="button", cannot use <button>
																<span
																	role="button"
																	tabIndex={0}
																	className="p-0.5 rounded hover:bg-green-500/20 text-muted-foreground hover:text-green-500 transition-colors"
																	aria-label="Resolve thread"
																	onClick={(e) => {
																		e.stopPropagation();
																		onResolveThread(thread.id);
																	}}
																	onKeyDown={(e) => {
																		if (e.key === "Enter" || e.key === " ") {
																			e.preventDefault();
																			e.stopPropagation();
																			onResolveThread(thread.id);
																		}
																	}}
																>
																	<Check className="h-3 w-3" />
																</span>
															)}
															{onDeleteThread && (
																// biome-ignore lint/a11y/useSemanticElements: nested inside role="button", cannot use <button>
																<span
																	role="button"
																	tabIndex={0}
																	className="p-0.5 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-colors"
																	aria-label="Delete thread"
																	onClick={(e) => {
																		e.stopPropagation();
																		onDeleteThread(thread.id);
																	}}
																	onKeyDown={(e) => {
																		if (e.key === "Enter" || e.key === " ") {
																			e.preventDefault();
																			e.stopPropagation();
																			onDeleteThread(thread.id);
																		}
																	}}
																>
																	<Trash2 className="h-3 w-3" />
																</span>
															)}
														</div>
													)}
												</div>
												<span className="line-clamp-2 text-foreground">
													{firstEntry?.content ?? ""}
												</span>
											</div>
										</div>
									);
								})}
						</div>
					);
				})}
			</div>
		</ScrollArea>
	);
}
