import { Copy, ExternalLink, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { DiagnosticReport, WorkflowSummary } from "@/types/workflow";
import { DiagnosticBadge } from "./DiagnosticBadge";

export function WorkflowList({
	workflows,
	report,
	selectedName,
	onSelect,
	onDelete,
	onDuplicate,
	onOpenInEditor,
	onCreate,
}: {
	workflows: WorkflowSummary[];
	report: DiagnosticReport;
	selectedName: string | null;
	onSelect: (name: string) => void;
	onDelete: (name: string) => void;
	onDuplicate: (name: string) => void;
	onOpenInEditor: (name: string) => void;
	onCreate: () => void;
}) {
	return (
		<div className="flex flex-col gap-2">
			<div className="flex items-center justify-between">
				<span className="text-xs font-medium text-muted-foreground">
					Workflows
				</span>
				<Button
					variant="ghost"
					size="icon"
					className="size-6"
					onClick={onCreate}
				>
					<Plus className="size-3.5" />
				</Button>
			</div>
			<div className="flex flex-col gap-1">
				{workflows.map((wf) => (
					// biome-ignore lint/a11y/useSemanticElements: <button> would nest with action <Button> children
					<div
						key={wf.name}
						role="button"
						tabIndex={0}
						onClick={() => onSelect(wf.name)}
						onKeyDown={(e) => {
							if (e.key === "Enter" || e.key === " ") {
								e.preventDefault();
								onSelect(wf.name);
							}
						}}
						className={cn(
							"flex items-center justify-between rounded-md border px-3 py-2 text-left text-sm transition-colors cursor-pointer",
							selectedName === wf.name
								? "border-primary bg-muted"
								: "border-border hover:bg-secondary",
						)}
					>
						<div className="flex flex-col gap-0.5 min-w-0 flex-1">
							<div className="flex items-center gap-2">
								<span className="font-medium truncate">{wf.name}</span>
								{wf.builtin && (
									<span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground shrink-0">
										builtin
									</span>
								)}
								<DiagnosticBadge summary={report.workflow_summaries[wf.name]} />
							</div>
							<span className="text-xs text-muted-foreground truncate">
								{wf.description}
							</span>
						</div>
						<div
							role="toolbar"
							className="flex items-center gap-0.5 shrink-0 ml-2"
							onClick={(e) => e.stopPropagation()}
							onKeyDown={(e) => {
								if (e.key === "Enter" || e.key === " ") e.stopPropagation();
							}}
						>
							{wf.builtin ? (
								<Button
									variant="ghost"
									size="icon"
									className="size-6"
									onClick={() => onDuplicate(wf.name)}
									title="Duplicate as custom"
								>
									<Copy className="size-3" />
								</Button>
							) : (
								<>
									<Button
										variant="ghost"
										size="icon"
										className="size-6"
										onClick={() => onOpenInEditor(wf.name)}
										title="Open in editor"
									>
										<ExternalLink className="size-3" />
									</Button>
									<Button
										variant="ghost"
										size="icon"
										className="size-6 text-destructive hover:text-destructive"
										onClick={() => onDelete(wf.name)}
										title="Delete"
									>
										<Trash2 className="size-3" />
									</Button>
								</>
							)}
						</div>
					</div>
				))}
			</div>
		</div>
	);
}
