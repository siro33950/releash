import { invoke } from "@tauri-apps/api/core";
import { Check, Copy, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
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
		selectedAgents,
		setSelectedAgents,
		loading,
		saving,
		error,
		saveResults,
		regenerateToken,
	} = mcp;

	const [preview, setPreview] = useState("");
	const [copied, setCopied] = useState(false);

	const toggleAgent = useCallback(
		(agent: McpAgentType) => {
			setSelectedAgents((prev) =>
				prev.includes(agent)
					? prev.filter((a) => a !== agent)
					: [...prev, agent],
			);
		},
		[setSelectedAgents],
	);

	useEffect(() => {
		if (loading || selectedAgents.length === 0) {
			setPreview("");
			return;
		}
		let cancelled = false;

		Promise.allSettled(
			selectedAgents.map((agent) =>
				invoke<string>("preview_agent_mcp_config", {
					agentType: agent,
					port: draft.port,
					token: draft.token,
				}).then((content) => ({ agent, content })),
			),
		).then((results) => {
			if (cancelled) return;
			const parts: string[] = [];
			for (const r of results) {
				if (r.status === "fulfilled") {
					const { agent, content } = r.value;
					const label = MCP_AGENT_OPTIONS.find((o) => o.value === agent)?.label;
					parts.push(`// ${label}\n${content}`);
				} else {
					parts.push(`// Error: ${String(r.reason)}`);
				}
			}
			setPreview(parts.join("\n\n"));
		});

		return () => {
			cancelled = true;
		};
	}, [selectedAgents, draft.port, draft.token, loading]);

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
						min={1}
						max={65535}
						step={1}
						variant="panel"
						size="sm"
						className="w-32"
						value={draft.port}
						onChange={(e) => {
							const next = Number.parseInt(e.target.value, 10);
							setDraft((d) => ({
								...d,
								port:
									Number.isFinite(next) && next >= 1 && next <= 65535
										? next
										: 19801,
							}));
						}}
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
				<h3 className="text-sm font-medium">Agent Config</h3>
				<p className="text-[10px] text-muted-foreground">
					Select agents to generate config files on save
				</p>

				<div className="flex flex-col gap-2">
					{MCP_AGENT_OPTIONS.map((opt) => (
						<div key={opt.value} className="flex items-center gap-2">
							<Checkbox
								id={`mcp-agent-${opt.value}`}
								checked={selectedAgents.includes(opt.value)}
								onCheckedChange={() => toggleAgent(opt.value)}
								disabled={saving}
							/>
							<label
								htmlFor={`mcp-agent-${opt.value}`}
								className="text-xs cursor-pointer"
							>
								{opt.label}
							</label>
						</div>
					))}
				</div>

				{saving && (
					<div className="flex items-center gap-2 text-xs text-muted-foreground">
						<Loader2 className="size-3.5 animate-spin" />
						Saving & restarting...
					</div>
				)}

				{saveResults.length > 0 && (
					<div className="flex flex-col gap-1">
						{saveResults.map((r) => (
							<span
								key={r.file_path}
								className="text-xs text-green-500 truncate"
							>
								{r.file_path}
							</span>
						))}
					</div>
				)}

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
						{preview}
					</pre>
				</div>
			)}
		</div>
	);
}
