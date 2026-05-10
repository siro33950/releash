import { Loader2 } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { extractTemplateVariables } from "./utils";

export function FacetEditor({
	initialContent,
	facetKey,
	onSave,
	onCancel,
	renderPreview,
}: {
	initialContent: string;
	facetKey: string;
	onSave: (content: string) => Promise<{ ok: boolean; error?: string }>;
	onCancel: () => void;
	renderPreview: (
		content: string,
		sampleValues: Record<string, string>,
	) => Promise<string>;
}) {
	const [content, setContent] = useState(initialContent);
	const [preview, setPreview] = useState<string | null>(null);
	const [sampleValues, setSampleValues] = useState<Record<string, string>>({});
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const variables = useMemo(() => extractTemplateVariables(content), [content]);

	const handlePreview = useCallback(async () => {
		try {
			const rendered = await renderPreview(content, sampleValues);
			setPreview(rendered);
		} catch (e) {
			setError(e instanceof Error ? e.message : String(e));
		}
	}, [content, sampleValues, renderPreview]);

	const handleSave = async () => {
		setSaving(true);
		setError(null);
		try {
			const result = await onSave(content);
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
		<div className="flex flex-col gap-4">
			<div className="flex items-center justify-between">
				<h4 className="text-sm font-medium">Edit: {facetKey}</h4>
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

			<Textarea
				value={content}
				onChange={(e) => {
					setContent(e.target.value);
					setPreview(null);
				}}
				className="font-mono text-xs min-h-[200px]"
				rows={12}
			/>

			{variables.length > 0 && (
				<div className="flex flex-col gap-2">
					<div className="flex items-center justify-between">
						<span className="text-xs font-medium text-muted-foreground">
							Template Variables
						</span>
						<Button variant="outline" size="sm" onClick={handlePreview}>
							Preview
						</Button>
					</div>
					<div className="flex flex-col gap-1.5">
						{variables.map((v) => (
							<div key={v} className="flex items-center gap-2">
								<span className="text-xs font-mono w-32 shrink-0">
									{`{{${v}}}`}
								</span>
								<Input
									value={sampleValues[v] ?? ""}
									onChange={(e) =>
										setSampleValues((prev) => ({
											...prev,
											[v]: e.target.value,
										}))
									}
									placeholder="Sample value"
									className="h-7 text-xs"
								/>
							</div>
						))}
					</div>
				</div>
			)}

			{preview !== null && (
				<div className="flex flex-col gap-1">
					<span className="text-xs font-medium text-muted-foreground">
						Preview
					</span>
					<pre className="whitespace-pre-wrap text-xs bg-muted rounded-md p-3 max-h-48 overflow-auto">
						{preview}
					</pre>
				</div>
			)}
		</div>
	);
}
