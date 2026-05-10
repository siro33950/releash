import { Loader2, Plus } from "lucide-react";
import { useCallback, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import type { Step, StepMode, Workflow } from "@/types/workflow";
import { type FacetSlot, StepEditor } from "./StepEditor";

export function WorkflowEditor({
	workflow,
	allFacetKeys,
	onSave,
	onCancel,
}: {
	workflow: Workflow;
	allFacetKeys: Record<FacetSlot, string[]>;
	onSave: (
		wf: Workflow,
		originalName?: string,
	) => Promise<{ ok: boolean; error?: string }>;
	onCancel: () => void;
}) {
	const [draft, setDraft] = useState<Workflow>({
		...workflow,
		steps: workflow.steps.map((s) => ({ ...s })),
	});
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const originalName = workflow.name;

	const updateStep = useCallback(
		(index: number, updater: (s: Step) => Step) => {
			setDraft((prev) => ({
				...prev,
				steps: prev.steps.map((s, i) => (i === index ? updater({ ...s }) : s)),
			}));
		},
		[],
	);

	const addStep = useCallback(() => {
		const existingNames = new Set(draft.steps.map((s) => s.name));
		let suffix = draft.steps.length + 1;
		let name = `step-${suffix}`;
		while (existingNames.has(name)) {
			suffix++;
			name = `step-${suffix}`;
		}
		setDraft((prev) => ({
			...prev,
			steps: [
				...prev.steps,
				{
					name,
					mode: "auto" as StepMode,
					rules: [],
				},
			],
		}));
	}, [draft.steps]);

	const removeStep = useCallback((index: number) => {
		setDraft((prev) => ({
			...prev,
			steps: prev.steps.filter((_, i) => i !== index),
		}));
	}, []);

	const moveStep = useCallback((index: number, direction: "up" | "down") => {
		setDraft((prev) => {
			const steps = [...prev.steps];
			const target = direction === "up" ? index - 1 : index + 1;
			if (target < 0 || target >= steps.length) return prev;
			[steps[index], steps[target]] = [steps[target], steps[index]];
			return { ...prev, steps };
		});
	}, []);

	const handleSave = async () => {
		setSaving(true);
		setError(null);
		const result = await onSave(draft, originalName);
		setSaving(false);
		if (!result.ok) {
			setError(result.error ?? "Save failed");
		}
	};

	return (
		<div className="flex flex-col gap-4">
			<div className="flex items-center justify-between">
				<h4 className="text-sm font-medium">Edit Workflow</h4>
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

			<div className="flex flex-col gap-3">
				<div className="flex flex-col gap-1">
					<label
						htmlFor="wf-editor-name"
						className="text-xs font-medium text-muted-foreground"
					>
						Name
					</label>
					<Input
						id="wf-editor-name"
						value={draft.name}
						onChange={(e) =>
							setDraft((prev) => ({ ...prev, name: e.target.value }))
						}
						className="h-8 text-sm"
					/>
				</div>

				<div className="flex flex-col gap-1">
					<label
						htmlFor="wf-editor-desc"
						className="text-xs font-medium text-muted-foreground"
					>
						Description
					</label>
					<Input
						id="wf-editor-desc"
						value={draft.description}
						onChange={(e) =>
							setDraft((prev) => ({ ...prev, description: e.target.value }))
						}
						className="h-8 text-sm"
					/>
				</div>
			</div>

			<Separator />

			<div className="flex items-center justify-between">
				<span className="text-xs font-medium text-muted-foreground">
					Steps ({draft.steps.length})
				</span>
				<Button
					variant="ghost"
					size="icon"
					className="size-6"
					onClick={addStep}
					aria-label="Add step"
				>
					<Plus className="size-3.5" />
				</Button>
			</div>

			<div className="flex flex-col gap-2">
				{draft.steps.map((step, idx) => (
					<StepEditor
						// biome-ignore lint/suspicious/noArrayIndexKey: steps have no stable unique id; name can be duplicated by user edit
						key={idx}
						step={step}
						index={idx}
						totalSteps={draft.steps.length}
						allFacetKeys={allFacetKeys}
						allStepNames={draft.steps.map((s) => s.name)}
						onUpdate={(updater) => updateStep(idx, updater)}
						onRemove={() => removeStep(idx)}
						onMove={(dir) => moveStep(idx, dir)}
					/>
				))}
			</div>
		</div>
	);
}
