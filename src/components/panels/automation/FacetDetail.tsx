import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import type { DiagnosticReport, FacetKind } from "@/types/workflow";
import { DiagnosticItemRow } from "./DiagnosticBadge";
import { extractTemplateVariables, facetKindToDirName } from "./utils";

export function FacetDetail({
	content,
	facetKey,
	kind,
	builtin,
	report,
	onEdit,
}: {
	content: string;
	facetKey: string;
	kind: FacetKind;
	builtin: boolean;
	report: DiagnosticReport;
	onEdit: () => void;
}) {
	const dirName = facetKindToDirName(kind);
	const facetId = `${dirName}/${facetKey}`;
	const usage = report.facet_usage[facetId] ?? [];
	const diagnosticItems = report.items.filter(
		(i) => i.facet_key === facetKey && i.facet_kind === dirName,
	);

	const variables = useMemo(() => extractTemplateVariables(content), [content]);

	return (
		<div className="flex flex-col gap-4">
			<div className="flex items-center justify-between">
				<div>
					<h4 className="text-sm font-medium">{facetKey}</h4>
					<p className="text-xs text-muted-foreground capitalize">{kind}</p>
				</div>
				{!builtin && (
					<Button variant="outline" size="sm" onClick={onEdit}>
						Edit
					</Button>
				)}
			</div>

			{diagnosticItems.length > 0 && (
				<div className="flex flex-col gap-1.5 rounded-md border border-border p-3">
					<span className="text-xs font-medium">Diagnostics</span>
					{diagnosticItems.map((item) => (
						<DiagnosticItemRow
							key={`${item.severity}-${item.message}-${item.field ?? ""}`}
							item={item}
						/>
					))}
				</div>
			)}

			{variables.length > 0 && (
				<div className="flex flex-col gap-1">
					<span className="text-xs font-medium text-muted-foreground">
						Variables
					</span>
					<div className="flex flex-wrap gap-1">
						{variables.map((v) => (
							<span
								key={v}
								className="rounded bg-muted px-1.5 py-0.5 text-xs font-mono"
							>
								{`{{${v}}}`}
							</span>
						))}
					</div>
				</div>
			)}

			{usage.length > 0 && (
				<div className="flex flex-col gap-1">
					<span className="text-xs font-medium text-muted-foreground">
						Used by
					</span>
					{usage.map((u) => (
						<div
							key={`${u.workflow_name}-${u.step_name}-${u.slot}`}
							className="text-xs text-muted-foreground"
						>
							{u.workflow_name} → {u.step_name} ({u.slot})
						</div>
					))}
				</div>
			)}

			<div className="flex flex-col gap-1">
				<span className="text-xs font-medium text-muted-foreground">
					Content
				</span>
				<pre className="whitespace-pre-wrap text-xs bg-muted rounded-md p-3 max-h-64 overflow-auto">
					{content}
				</pre>
			</div>
		</div>
	);
}
