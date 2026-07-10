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

type WorkflowRule = NonNullable<NodeDefinition["rules"]>[number];

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
						(step.inputs && step.inputs.length > 0) ||
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
							{step.inputs && step.inputs.length > 0 && (
								<FacetRefRow label="Inputs" value={step.inputs.join(", ")} />
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
									key={`${step.name}-rule-${ruleKey(r)}`}
									className="text-muted-foreground"
								>
									<span className="font-mono">{formatRule(r)}</span>
								</div>
							))}
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

function formatRule(rule: WorkflowRule): string {
	switch (rule.type) {
		case "when":
			return `when ${rule.on} then ${rule.then} else ${rule.next}`;
		case "switch": {
			const cases = Object.entries(rule.cases)
				.map(([value, target]) => `${value} -> ${target}`)
				.join(", ");
			return rule.next
				? `switch ${rule.on}: ${cases}, next -> ${rule.next}`
				: `switch ${rule.on}: ${cases}`;
		}
		case "loop_guard":
			return `loop_guard max ${rule.max_iterations} -> ${rule.on_exhausted}`;
		case "next":
			return `next -> ${rule.next}`;
	}
}

function ruleKey(rule: WorkflowRule): string {
	switch (rule.type) {
		case "when":
			return `when:${rule.on}:${rule.then}:${rule.next}`;
		case "switch":
			return `switch:${rule.on}:${sortedCases(rule.cases)}:${rule.next ?? ""}`;
		case "loop_guard":
			return `loop_guard:${rule.max_iterations}:${rule.on_exhausted}`;
		case "next":
			return `next:${rule.next}`;
	}
}

function sortedCases(cases: Record<string, string>): string {
	return Object.entries(cases)
		.sort(([left], [right]) => left.localeCompare(right))
		.map(([value, target]) => `${value}:${target}`)
		.join(",");
}

function FacetRefRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="text-muted-foreground">
			<span className="text-foreground">{label}:</span> {value}
		</div>
	);
}
