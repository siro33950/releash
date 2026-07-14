import { ChevronDown, ChevronRight, Save, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import type {
	DiagnosticItem,
	DiagnosticReport,
	FanoutItemsSource,
	NodeDefinition,
	Workflow,
} from "@/types/workflow";
import { DiagnosticItemRow } from "./DiagnosticBadge";

type WorkflowRule = NonNullable<NodeDefinition["rules"]>[number];

type WorkflowSaveResult =
	| { ok: true; workflow?: Workflow }
	| { ok: false; error: string; diagnostics?: DiagnosticItem[] };

export function WorkflowDetail({
	workflow,
	report,
	source,
	onEdit,
}: {
	workflow: Workflow;
	report: DiagnosticReport;
	source?: string | null;
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
							key={`${item.code}-${item.span?.start_line ?? "na"}-${item.span?.start_col ?? "na"}-${item.message}-${item.field ?? ""}`}
							item={item}
						/>
					))}
				</div>
			)}

			{source && <WorkflowSourcePane source={source} diagnostics={items} />}

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

export function WorkflowSourceDiagnosticDetail({
	name,
	report,
	source,
	onEdit,
}: {
	name: string;
	report: DiagnosticReport;
	source: string;
	onEdit: () => void;
}) {
	const items = report.items.filter((i) => i.workflow_name === name);

	return (
		<div className="flex flex-col gap-4">
			<div className="flex items-center justify-between">
				<div>
					<h4 className="text-sm font-medium">{name}</h4>
					<p className="text-xs text-muted-foreground">
						Invalid workflow definition
					</p>
				</div>
				<Button variant="outline" size="sm" onClick={onEdit}>
					Edit
				</Button>
			</div>

			{items.length > 0 && (
				<div className="flex flex-col gap-1.5 rounded-md border border-border p-3">
					<span className="text-xs font-medium">Diagnostics</span>
					{items.map((item) => (
						<DiagnosticItemRow
							key={`${item.code}-${item.span?.start_line ?? "na"}-${item.span?.start_col ?? "na"}-${item.message}-${item.field ?? ""}`}
							item={item}
						/>
					))}
				</div>
			)}

			<WorkflowSourcePane source={source} diagnostics={items} />
		</div>
	);
}

export function WorkflowSourceEditor({
	name,
	initialSource,
	diagnostics,
	onSave,
	onCancel,
}: {
	name: string;
	initialSource: string;
	diagnostics: DiagnosticItem[];
	onSave: (source: string) => Promise<WorkflowSaveResult>;
	onCancel: () => void;
}) {
	const hostRef = useRef<HTMLDivElement | null>(null);
	const editorRef = useRef<
		import("monaco-editor").editor.IStandaloneCodeEditor | null
	>(null);
	const modelRef = useRef<import("monaco-editor").editor.ITextModel | null>(
		null,
	);
	const monacoRef = useRef<typeof import("monaco-editor") | null>(null);
	const [content, setContent] = useState(initialSource);
	const [saving, setSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const markerDiagnostics = useMemo(
		() => diagnostics.filter((item) => item.span),
		[diagnostics],
	);
	const markerDiagnosticsRef = useRef(markerDiagnostics);

	useEffect(() => {
		setContent(initialSource);
	}, [initialSource]);

	useEffect(() => {
		let disposed = false;
		let cleanup: (() => void) | undefined;

		void import("monaco-editor").then((monaco) => {
			if (disposed || !hostRef.current) return;

			monacoRef.current = monaco;
			const model = monaco.editor.createModel(initialSource, "yaml");
			modelRef.current = model;
			const editor = monaco.editor.create(hostRef.current, {
				model,
				readOnly: false,
				minimap: { enabled: false },
				scrollBeyondLastLine: false,
				automaticLayout: true,
				lineNumbers: "on",
				overviewRulerLanes: 2,
				tabSize: 2,
			});
			const subscription = editor.onDidChangeModelContent(() => {
				setContent(model.getValue());
			});
			editorRef.current = editor;
			applyMonacoMarkers(monaco, model, markerDiagnosticsRef.current);
			cleanup = () => {
				monaco.editor.setModelMarkers(model, "workflow-diagnostics", []);
				subscription.dispose();
				editor.dispose();
				model.dispose();
				editorRef.current = null;
				modelRef.current = null;
				monacoRef.current = null;
			};
		});

		return () => {
			disposed = true;
			cleanup?.();
		};
	}, [initialSource]);

	useEffect(() => {
		markerDiagnosticsRef.current = markerDiagnostics;
		if (!monacoRef.current || !modelRef.current) return;
		applyMonacoMarkers(monacoRef.current, modelRef.current, markerDiagnostics);
	}, [markerDiagnostics]);

	const handleSave = async () => {
		setSaving(true);
		setSaveError(null);
		const result = await onSave(content);
		setSaving(false);
		if (!result.ok) {
			setSaveError(result.error);
		}
	};

	return (
		<div className="flex flex-col gap-3">
			<div className="flex items-center justify-between">
				<div>
					<h4 className="text-sm font-medium">{name}</h4>
					<p className="text-xs text-muted-foreground">Workflow YAML</p>
				</div>
				<div className="flex items-center gap-2">
					<Button variant="outline" size="sm" onClick={onCancel}>
						<X className="size-3.5" />
						Cancel
					</Button>
					<Button size="sm" onClick={handleSave} disabled={saving}>
						<Save className="size-3.5" />
						{saving ? "Saving..." : "Save"}
					</Button>
				</div>
			</div>

			{saveError && <p className="text-xs text-destructive">{saveError}</p>}

			{diagnostics.length > 0 && (
				<div className="flex flex-col gap-1.5 rounded-md border border-border p-3">
					<span className="text-xs font-medium">Diagnostics</span>
					{diagnostics.map((item) => (
						<DiagnosticItemRow
							key={`${item.code}-${item.span?.start_line ?? "na"}-${item.span?.start_col ?? "na"}-${item.message}-${item.field ?? ""}`}
							item={item}
						/>
					))}
				</div>
			)}

			<div
				ref={hostRef}
				className="h-96 overflow-hidden rounded-md border border-border"
			/>
		</div>
	);
}

function WorkflowSourcePane({
	source,
	diagnostics,
}: {
	source: string;
	diagnostics: DiagnosticItem[];
}) {
	const hostRef = useRef<HTMLDivElement | null>(null);
	const editorRef = useRef<
		import("monaco-editor").editor.IStandaloneCodeEditor | null
	>(null);
	const modelRef = useRef<import("monaco-editor").editor.ITextModel | null>(
		null,
	);

	const markerDiagnostics = useMemo(
		() => diagnostics.filter((item) => item.span),
		[diagnostics],
	);

	useEffect(() => {
		let disposed = false;
		let cleanup: (() => void) | undefined;

		void import("monaco-editor").then((monaco) => {
			if (disposed || !hostRef.current) return;

			const model = monaco.editor.createModel(source, "yaml");
			modelRef.current = model;
			const editor = monaco.editor.create(hostRef.current, {
				model,
				readOnly: true,
				minimap: { enabled: false },
				scrollBeyondLastLine: false,
				automaticLayout: true,
				lineNumbers: "on",
				renderLineHighlight: "none",
				overviewRulerLanes: 2,
			});
			editorRef.current = editor;
			applyMonacoMarkers(monaco, model, markerDiagnostics);
			cleanup = () => {
				monaco.editor.setModelMarkers(model, "workflow-diagnostics", []);
				editor.dispose();
				model.dispose();
				editorRef.current = null;
				modelRef.current = null;
			};
		});

		return () => {
			disposed = true;
			cleanup?.();
		};
	}, [source, markerDiagnostics]);

	return (
		<div className="flex flex-col gap-1.5">
			<span className="text-xs font-medium text-muted-foreground">YAML</span>
			<div
				ref={hostRef}
				className="h-64 overflow-hidden rounded-md border border-border"
			/>
		</div>
	);
}

function applyMonacoMarkers(
	monaco: typeof import("monaco-editor"),
	model: import("monaco-editor").editor.ITextModel,
	diagnostics: DiagnosticItem[],
) {
	const markers = diagnostics.flatMap((item) => {
		if (!item.span) return [];
		const severity =
			item.severity === "error"
				? monaco.MarkerSeverity.Error
				: item.severity === "warning"
					? monaco.MarkerSeverity.Warning
					: monaco.MarkerSeverity.Info;
		return [
			{
				severity,
				message: `${item.code}: ${item.message}`,
				startLineNumber: item.span.start_line,
				startColumn: item.span.start_col,
				endLineNumber: item.span.end_line,
				endColumn: Math.max(item.span.end_col, item.span.start_col + 1),
				code: item.code,
			},
		];
	});
	monaco.editor.setModelMarkers(model, "workflow-diagnostics", markers);
}

function StepCard({ step, index }: { step: NodeDefinition; index: number }) {
	const [expanded, setExpanded] = useState(false);
	const session = step.session;
	const facets = session?.facets;
	const fanout = step.fanout;
	const childCount = fanout?.child.length ?? 0;

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

					{/* Fanout children */}
					{fanout && (
						<div className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground">
								Fanout Children
							</span>
							{fanout.child.map((childName) => (
								<div key={childName} className="ml-2 text-muted-foreground">
									• {childName}
								</div>
							))}
							{fanout.items !== undefined && (
								<FacetRefRow
									label="Items"
									value={formatFanoutItems(fanout.items)}
								/>
							)}
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

function formatFanoutItems(items: FanoutItemsSource): string {
	return typeof items === "string" ? items : JSON.stringify(items);
}

function FacetRefRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="text-muted-foreground">
			<span className="text-foreground">{label}:</span> {value}
		</div>
	);
}
