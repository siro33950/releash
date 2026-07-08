import { ChevronDown, ChevronRight } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import type {
	DiagnosticReport,
	NodeDefinition,
	Workflow,
} from "@/types/workflow";
import { DiagnosticItemRow } from "./DiagnosticBadge";

export function WorkflowDetail({
	workflow,
	report,
	onEdit,
}: {
	workflow: Workflow;
	report: DiagnosticReport;
	onEdit: () => void;
}) {
	const items = report.items.filter((i) => i.workflow_name === workflow.name);

	return (
		<div className="flex flex-col gap-4">
			<div className="flex items-center justify-between">
				<div>
					<h4 className="text-sm font-medium">{workflow.name}</h4>
					<p className="text-xs text-muted-foreground">
						{workflow.description}
					</p>
				</div>
				{!workflow.builtin && (
					<Button variant="outline" size="sm" onClick={onEdit}>
						Edit
					</Button>
				)}
			</div>

			{items.length > 0 && (
				<div className="flex flex-col gap-1.5 rounded-md border border-border p-3">
					<span className="text-xs font-medium">Diagnostics</span>
					{items.map((item) => (
						<DiagnosticItemRow
							key={`${item.severity}-${item.message}-${item.field ?? ""}`}
							item={item}
						/>
					))}
				</div>
			)}

			<div className="flex flex-col gap-2">
				<span className="text-xs font-medium text-muted-foreground">
					Steps ({workflow.nodes.length})
				</span>
				{workflow.nodes.map((step, idx) => (
					<StepCard key={step.name} step={step} index={idx} />
				))}
			</div>
		</div>
	);
}

function StepCard({ step, index }: { step: NodeDefinition; index: number }) {
	const [expanded, setExpanded] = useState(false);
	const session = step.session;
	const facets = session?.facets;
	const fanout = step.fanout;
	const childCount = fanout?.parallel_children.length ?? 0;

	return (
		<div className="rounded-md border border-border">
			<button
				type="button"
				onClick={() => setExpanded(!expanded)}
				className="flex items-center justify-between w-full px-3 py-2 text-left"
			>
				<div className="flex items-center gap-2">
					<span className="text-xs text-muted-foreground">{index + 1}.</span>
					<span className="text-sm font-medium">{step.name}</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
						{step.kind}
					</span>
					{session?.gate === "approval" && (
						<span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-600">
							gate: approval
						</span>
					)}
					{fanout && (
						<span className="rounded bg-blue-500/10 px-1.5 py-0.5 text-[10px] text-blue-500">
							{childCount} children
						</span>
					)}
				</div>
				{expanded ? (
					<ChevronDown className="size-3.5 text-muted-foreground" />
				) : (
					<ChevronRight className="size-3.5 text-muted-foreground" />
				)}
			</button>

			{expanded && (
				<div className="px-3 pb-3 flex flex-col gap-2 text-xs">
					<Separator />
					{step.command && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">Command</span>
							<p className="text-muted-foreground whitespace-pre-wrap bg-muted rounded p-2 font-mono">
								{step.command}
							</p>
						</div>
					)}

					{session && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">Session</span>
							<div className="text-muted-foreground">
								Gate: {session.gate}
								{session.model ? ` | Model: ${session.model}` : ""}
								{session.permission
									? ` | Permission: ${session.permission}`
									: ""}
							</div>
						</div>
					)}

					{/* Facet refs */}
					{(facets?.policy ||
						facets?.knowledge ||
						facets?.instruction ||
						step.artifact ||
						step.input) && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">
								Workflow References
							</span>
							{facets?.policy && (
								<FacetRefRow label="Policy" value={facets.policy} />
							)}
							{facets?.knowledge && (
								<FacetRefRow label="Knowledge" value={facets.knowledge} />
							)}
							{facets?.instruction && (
								<FacetRefRow label="Instruction" value={facets.instruction} />
							)}
							{step.artifact && (
								<FacetRefRow label="Artifact" value={step.artifact} />
							)}
							{step.input && <FacetRefRow label="Input" value={step.input} />}
						</div>
					)}

					{/* Rules */}
					{step.rules && step.rules.length > 0 && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">
								Transition Rules
							</span>
							{step.rules.map((r) => (
								<div
									key={`${r.match}-${r.next}`}
									className="text-muted-foreground"
								>
									<span className="font-mono">{r.match}</span> →{" "}
									<span className="font-mono">{r.next}</span>
								</div>
							))}
						</div>
					)}

					{/* Cycle guard */}
					{step.cycle_guard && (
						<div className="text-muted-foreground">
							Cycle Guard: max {step.cycle_guard.max_iterations} iterations
						</div>
					)}

					{/* Collect */}
					{step.collect && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">Collect</span>
							<div className="text-muted-foreground">
								From: {step.collect.from.join(", ")} | Reduce:{" "}
								{step.collect.reduce}
							</div>
						</div>
					)}

					{/* Output passing */}
					{step.pass_previous_response && (
						<div className="text-muted-foreground">
							Pass previous response: yes
						</div>
					)}
					{step.pass_output_from && step.pass_output_from.length > 0 && (
						<div className="text-muted-foreground">
							Pass output from: {step.pass_output_from.join(", ")}
						</div>
					)}

					{/* Parallel children */}
					{fanout && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">
								Fanout Children
							</span>
							{fanout.parallel_children.map((ps) => (
								<div key={ps.name} className="ml-2 text-muted-foreground">
									• {ps.name}
									{ps.facets.instruction &&
										` — instruction: ${ps.facets.instruction}`}
								</div>
							))}
							{fanout.aggregate && (
								<div className="mt-1">
									<span className="font-medium text-muted-foreground">
										Aggregate:{" "}
									</span>
									<span className="text-muted-foreground">
										{fanout.aggregate.all_match
											? `all_match("${fanout.aggregate.all_match}")`
											: `any_match("${fanout.aggregate.any_match}")`}{" "}
										→ then: {fanout.aggregate.then}, else:{" "}
										{fanout.aggregate.else}
									</span>
								</div>
							)}
						</div>
					)}
				</div>
			)}
		</div>
	);
}

function FacetRefRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="text-muted-foreground">
			<span className="text-foreground">{label}:</span> {value}
		</div>
	);
}
