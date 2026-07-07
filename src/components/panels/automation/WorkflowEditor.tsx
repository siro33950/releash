import MonacoEditor from "@monaco-editor/react";
import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import type { DiagnosticReport, Workflow } from "@/types/workflow";
import { DiagnosticItemRow } from "./DiagnosticBadge";

type SaveResult =
	| { ok: true; workflow: Workflow }
	| { ok: false; error?: string };

const MONACO_OPTIONS = {
	ariaLabel: "Workflow YAML",
	automaticLayout: true,
	fontSize: 12,
	lineNumbers: "on",
	minimap: { enabled: false },
	renderWhitespace: "selection",
	scrollBeyondLastLine: false,
	tabSize: 2,
	wordWrap: "on",
} as const;

export function WorkflowEditor({
	workflow,
	source,
	report,
	onSave,
	onCancel,
}: {
	workflow: Workflow;
	source: string;
	report: DiagnosticReport;
	onSave: (source: string, originalName?: string) => Promise<SaveResult>;
	onCancel: () => void;
}) {
	const [draft, setDraft] = useState(source);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const originalName = workflow.name;
	const items = report.items.filter((i) => i.workflow_name === workflow.name);

	useEffect(() => {
		setDraft(source);
		setError(null);
	}, [source]);

	const handleSave = async () => {
		setSaving(true);
		setError(null);
		try {
			const result = await onSave(draft, originalName);
			if (!result.ok) {
				setError(result.error ?? "Save failed");
			}
		} catch (e) {
			setError(e instanceof Error ? e.message : String(e));
		} finally {
			setSaving(false);
		}
	};

	return (
		<div className="flex flex-col gap-3">
			<div className="flex items-center justify-between">
				<h4 className="text-sm font-medium">Edit Workflow YAML</h4>
				<div className="flex items-center gap-2">
					<Button variant="ghost" size="sm" onClick={onCancel}>
						Cancel
					</Button>
					<Button size="sm" onClick={handleSave} disabled={saving}>
						{saving ? <Loader2 className="size-3.5 animate-spin" /> : "Save"}
					</Button>
				</div>
			</div>

			{error && <p className="text-xs text-destructive">{error}</p>}

			<div
				data-testid="workflow-yaml-monaco"
				className="min-h-[520px] overflow-hidden rounded-md border border-border bg-background"
			>
				<MonacoEditor
					height="520px"
					language="yaml"
					path={`${workflow.name}.workflow.yml`}
					theme="vs-dark"
					value={draft}
					onChange={(value) => setDraft(value ?? "")}
					options={MONACO_OPTIONS}
					loading={
						<div className="flex h-[520px] items-center justify-center text-xs text-muted-foreground">
							Loading YAML editor...
						</div>
					}
				/>
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
		</div>
	);
}
