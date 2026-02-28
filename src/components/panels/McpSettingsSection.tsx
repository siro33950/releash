import { Check, Copy, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	type GenerateResult,
	MCP_AGENT_OPTIONS,
	type McpAgentType,
	type useMcpConfig,
} from "@/hooks/useMcpConfig";

const labelClass = "text-xs font-medium text-muted-foreground";

interface McpSettingsSectionProps {
	mcp: ReturnType<typeof useMcpConfig>;
}

export function McpSettingsSection({ mcp }: McpSettingsSectionProps) {
	const {
		draft,
		setDraft,
		loading,
		error,
		regenerateToken,
		generateConfig,
		previewConfig,
	} = mcp;

	const [selectedAgent, setSelectedAgent] = useState<McpAgentType>("claude");
	const [preview, setPreview] = useState("");
	const [generateResult, setGenerateResult] = useState<GenerateResult | null>(
		null,
	);
	const [generating, setGenerating] = useState(false);
	const [copied, setCopied] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: draft triggers preview refresh on config change
	useEffect(() => {
		if (loading) return;
		let cancelled = false;
		previewConfig(selectedAgent)
			.then((content) => {
				if (!cancelled) setPreview(content);
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	}, [selectedAgent, draft.port, draft.token, loading, previewConfig]);

	const handleGenerate = useCallback(async () => {
		setGenerating(true);
		setGenerateResult(null);
		try {
			const result = await generateConfig(selectedAgent);
			setGenerateResult(result);
		} catch {
			// error is handled in hook
		} finally {
			setGenerating(false);
		}
	}, [generateConfig, selectedAgent]);

	const handleCopy = useCallback(async () => {
		if (!preview) return;
		try {
			await navigator.clipboard.writeText(preview);
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		} catch {}
	}, [preview]);

	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col gap-3">
				<h3 className="text-sm font-medium">MCP Server</h3>

				<div className="flex flex-col gap-1.5">
					<label htmlFor="mcp-port" className={labelClass}>
						Port
					</label>
					<Input
						id="mcp-port"
						type="number"
						variant="panel"
						size="sm"
						className="w-32"
						value={draft.port}
						onChange={(e) =>
							setDraft((d) => ({
								...d,
								port: Number(e.target.value) || 19801,
							}))
						}
					/>
				</div>

				<div className="flex flex-col gap-1.5">
					<label htmlFor="mcp-token" className={labelClass}>
						Token
					</label>
					<div className="flex items-center gap-2">
						<Input
							id="mcp-token"
							type="text"
							variant="panel"
							size="sm"
							className="flex-1 font-mono text-xs"
							value={draft.token}
							readOnly
						/>
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={regenerateToken}
							title="Regenerate token"
						>
							<RefreshCw className="size-3.5" />
						</Button>
					</div>
					<p className="text-[10px] text-muted-foreground">
						PTY sessions receive this token as $RELEASH_MCP_TOKEN
					</p>
				</div>
			</div>

			<div className="border-t border-border pt-4 flex flex-col gap-3">
				<h3 className="text-sm font-medium">Generate Agent Config</h3>

				<div className="flex flex-col gap-1.5">
					<label htmlFor="mcp-agent" className={labelClass}>
						Agent
					</label>
					<Select
						value={selectedAgent}
						onValueChange={(v) => setSelectedAgent(v as McpAgentType)}
					>
						<SelectTrigger id="mcp-agent" className="w-48">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{MCP_AGENT_OPTIONS.map((opt) => (
								<SelectItem key={opt.value} value={opt.value}>
									{opt.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>

				<div className="flex items-center gap-2">
					<Button
						type="button"
						size="sm"
						onClick={handleGenerate}
						disabled={generating}
					>
						{generating ? (
							<Loader2 className="size-3.5 animate-spin" />
						) : (
							"Generate"
						)}
					</Button>
					{generateResult && (
						<span className="text-xs text-green-500 truncate">
							{generateResult.file_path}
						</span>
					)}
				</div>

				{error && <p className="text-xs text-destructive">{error}</p>}
			</div>

			{preview && (
				<div className="flex flex-col gap-1.5">
					<div className="flex items-center justify-between">
						<span className={labelClass}>Preview</span>
						<Button
							type="button"
							variant="ghost"
							size="sm"
							className="h-6 px-2"
							onClick={handleCopy}
						>
							{copied ? (
								<Check className="size-3" />
							) : (
								<Copy className="size-3" />
							)}
						</Button>
					</div>
					<pre className="rounded-md bg-muted p-3 text-xs font-mono overflow-auto max-h-40 whitespace-pre-wrap">
						{generateResult?.content ?? preview}
					</pre>
				</div>
			)}
		</div>
	);
}
