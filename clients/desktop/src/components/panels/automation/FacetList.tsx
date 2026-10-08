import { Copy, ExternalLink, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { DiagnosticReport, FacetSummary } from "@/types/workflow";
import { DiagnosticBadge } from "./DiagnosticBadge";

export function FacetList({
	facets,
	report,
	selectedKey,
	onSelect,
	onDelete,
	onDuplicate,
	onOpenInEditor,
	onCreate,
}: {
	facets: FacetSummary[];
	report: DiagnosticReport;
	selectedKey: string | null;
	onSelect: (key: string) => void;
	onDelete: (key: string) => void;
	onDuplicate: (key: string) => void;
	onOpenInEditor: (key: string) => void;
	onCreate: () => void;
}) {
	return (
		<div className="flex flex-col gap-2">
			<div className="flex items-center justify-between">
				<span className="text-xs font-medium text-muted-foreground">
					{facets.length} facets
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
				{facets.map((f) => {
					const facetId = `${f.kind}/${f.key}`;
					const usageCount = report.facet_usage[facetId]?.length ?? 0;
					return (
						// biome-ignore lint/a11y/useSemanticElements: <button> would nest with action <Button> children
						<div
							key={f.key}
							role="button"
							tabIndex={0}
							onClick={() => onSelect(f.key)}
							onKeyDown={(e) => {
								if (e.key === "Enter" || e.key === " ") {
									e.preventDefault();
									onSelect(f.key);
								}
							}}
							className={cn(
								"flex items-center justify-between rounded-md border px-3 py-2 text-left text-sm transition-colors cursor-pointer",
								selectedKey === f.key
									? "border-primary bg-muted"
									: "border-border hover:bg-secondary",
							)}
						>
							<div className="flex flex-col gap-0.5 min-w-0 flex-1">
								<div className="flex items-center gap-2">
									<span className="font-medium truncate">{f.key}</span>
									{f.builtin && (
										<span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground shrink-0">
											builtin
										</span>
									)}
									<DiagnosticBadge summary={report.facet_summaries[facetId]} />
								</div>
								<div className="flex items-center gap-2">
									<span className="text-xs text-muted-foreground truncate">
										{f.description || "(no description)"}
									</span>
									{usageCount > 0 && (
										<span className="text-[10px] text-muted-foreground shrink-0">
											Used by {usageCount}
										</span>
									)}
								</div>
							</div>
							<div
								role="toolbar"
								className="flex items-center gap-0.5 shrink-0 ml-2"
								onClick={(e) => e.stopPropagation()}
								onKeyDown={(e) => {
									if (e.key === "Enter" || e.key === " ") e.stopPropagation();
								}}
							>
								{f.builtin ? (
									<Button
										variant="ghost"
										size="icon"
										className="size-6"
										onClick={() => onDuplicate(f.key)}
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
											onClick={() => onOpenInEditor(f.key)}
											title="Open in editor"
										>
											<ExternalLink className="size-3" />
										</Button>
										<Button
											variant="ghost"
											size="icon"
											className="size-6 text-destructive hover:text-destructive"
											onClick={() => onDelete(f.key)}
											title="Delete"
										>
											<Trash2 className="size-3" />
										</Button>
									</>
								)}
							</div>
						</div>
					);
				})}
			</div>
		</div>
	);
}
