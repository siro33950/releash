import { AlertCircle, AlertTriangle, Info } from "lucide-react";
import type { DiagnosticItem, DiagnosticSummary } from "@/types/workflow";

export function DiagnosticBadge({ summary }: { summary?: DiagnosticSummary }) {
	if (!summary) return null;
	const { error_count, warning_count, info_count } = summary;
	if (error_count === 0 && warning_count === 0 && info_count === 0) return null;
	return (
		<div className="flex items-center gap-1">
			{error_count > 0 && (
				<span className="flex items-center gap-0.5 text-[10px] text-destructive">
					<AlertCircle className="size-3" />
					{error_count}
				</span>
			)}
			{warning_count > 0 && (
				<span className="flex items-center gap-0.5 text-[10px] text-yellow-500">
					<AlertTriangle className="size-3" />
					{warning_count}
				</span>
			)}
			{info_count > 0 && (
				<span className="flex items-center gap-0.5 text-[10px] text-blue-500">
					<Info className="size-3" />
					{info_count}
				</span>
			)}
		</div>
	);
}

export function DiagnosticItemRow({ item }: { item: DiagnosticItem }) {
	const icon =
		item.severity === "error" ? (
			<AlertCircle className="size-3 text-destructive shrink-0" />
		) : item.severity === "warning" ? (
			<AlertTriangle className="size-3 text-yellow-500 shrink-0" />
		) : (
			<Info className="size-3 text-blue-500 shrink-0" />
		);
	const spanLabel = item.span
		? `${item.span.start_line}:${item.span.start_col}`
		: null;

	return (
		<div className="flex items-start gap-2 text-xs">
			{icon}
			<div className="min-w-0 flex-1">
				<div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
					<span className="font-mono text-[10px] font-medium text-foreground">
						{item.code}
					</span>
					{spanLabel && (
						<span className="font-mono text-[10px] text-muted-foreground">
							{spanLabel}
						</span>
					)}
					<span className="text-[10px] text-muted-foreground">
						{item.stage.replace("_", " ")}
					</span>
				</div>
				<div className="break-words text-muted-foreground">{item.message}</div>
			</div>
		</div>
	);
}
